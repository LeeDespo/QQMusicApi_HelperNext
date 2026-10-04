//! The two upstream shapes this component speaks.
//!
//! 1. **`cgi-bin/musicu.fcg`** — the modern envelope: a `comm` block plus one or
//!    more `req_<n>` blocks of `{module, method, param}`. Every catalogue read,
//!    the account's 我喜欢, the follow list and the login/QR calls go through it.
//! 2. **the legacy `c.y.qq.com` fcgi** — form-encoded GETs that still answer with
//!    the account's own playlists and favorited albums. The library's own
//!    `PlaylistBaseRead` candidates answer `40000` there, which is why the old
//!    helper went through this path too (verified 2026-09-18 in that code).
//!
//! Both are exercised with the account's cookies; `g_tk` is `hash33(qm_keyst)`.

use crate::credential::Credential;
use crate::device::DeviceStore;
use crate::guard::{CircuitBreaker, Class, RateLimit};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

const MUSICU_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
/// The signed twin of `musicu.fcg`. Same envelope, but the URL carries a `zzc`
/// signature over the exact body sent (see [`Upstream::call_signed`]).
const MUSICS_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musics.fcg";

/// The platform profile a request is sent under.
///
/// The upstream varies behaviour — and acceptance — by caller identity: the
/// **web** profile (`cv 4747474 / ct 24 / platform yqq.json`) is what the account
/// and catalogue endpoints want, while some interfaces are documented (and in
/// QQMusicApi implemented) against the **android** profile
/// (`ct 11 / cv 14090008`, which also carries device parameters). Nothing about
/// the credential changes; only this block of the envelope does.
///
/// It is a per-call choice rather than a global one because the same component
/// serves both kinds of call, and a host that needs the other profile for one
/// interface must not have to rebuild for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Platform {
    #[default]
    Web,
    Android,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Web => "web",
            Platform::Android => "android",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "web" | "yqq" | "yqq.json" => Some(Platform::Web),
            "android" => Some(Platform::Android),
            _ => None,
        }
    }

    /// The `comm` fields this profile adds on top of the shared ones.
    fn comm_overlay(self) -> Vec<(&'static str, serde_json::Value)> {
        match self {
            Platform::Web => vec![
                ("cv", json!(4747474)),
                ("ct", json!(24)),
                ("platform", json!("yqq.json")),
                ("needNewCode", json!(1)),
            ],
            Platform::Android => vec![
                ("cv", json!(14090008)),
                ("ct", json!(11)),
                ("v", json!(14090008)),
                ("platform", json!("yqq.json")),
                ("needNewCode", json!(1)),
            ],
        }
    }
}
const PROFILE_ASSETS_ENDPOINT: &str =
    "https://c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg";

/// One `req_<n>` block.
pub struct Call {
    pub module: &'static str,
    pub method: &'static str,
    pub param: Value,
}

pub struct Upstream {
    agent: ureq::Agent,
    pub limiter: RateLimit,
    pub breaker: CircuitBreaker,
    /// Generated once and kept beside the credential. Consulted only for the
    /// android profile, whose interfaces are the ones that want a device.
    device: DeviceStore,
    /// The download engine, managed here because it lives beside this component
    /// and only the component knows which downloads are in flight.
    pub aria2: crate::aria2::Aria2,
    /// See [`Upstream::encrypted_uin`].
    encrypt_uin_cache: Mutex<Option<String>>,
}

#[derive(Debug)]
pub enum UpstreamError {
    /// The breaker refused the call; the string says how long it will stay open.
    Refused(String),
    Transport(String),
    /// A shaped response whose code says no (`code != 0`), or an unparsable one.
    Upstream(String),
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpstreamError::Refused(reason) => write!(f, "{reason}"),
            UpstreamError::Transport(detail) => write!(f, "网络请求失败：{detail}"),
            UpstreamError::Upstream(detail) => write!(f, "{detail}"),
        }
    }
}

impl Upstream {
    pub fn new() -> Self {
        Self::with_device(DeviceStore::for_directory(&crate::data_directory()))
    }

    pub fn with_device(device: DeviceStore) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                // The QIMEI handshake can be slower than a catalogue read, and it
                // happens once a day; the ordinary calls keep the tighter bound
                // through the per-request timeout below.
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .into(),
            limiter: RateLimit::new(Duration::from_secs(10)),
            breaker: CircuitBreaker::default(),
            device,
            aria2: crate::aria2::Aria2::new(crate::data_directory()),
            encrypt_uin_cache: Mutex::new(None),
        }
    }

    /// The account's encrypted uin, which 关注歌手 addresses the account by.
    ///
    /// `get_login_status` reads it from the credential; a credential written before
    /// the component understood the login response's `encryptUin` spelling has
    /// none, and `GetLoginUserInfo` is where the account itself reports it. Cached
    /// for the life of the process: it cannot change while a session lasts.
    pub fn encrypted_uin(&self, credential: &Credential) -> Result<String, UpstreamError> {
        if !credential.encrypted_uin.is_empty() {
            return Ok(credential.encrypted_uin.clone());
        }
        if let Some(cached) = self.encrypt_uin_cache.lock().expect("encrypt uin").clone() {
            return Ok(cached);
        }
        let data = self.call_with(
            credential,
            Class::Account,
            Platform::Android,
            Call {
                module: "music.UserInfo.userInfoServer",
                method: "GetLoginUserInfo",
                param: json!({}),
            },
        )?;
        // Verified live that this endpoint carries no encrypted uin (2026-10-01);
        // the caller falls back to the numeric id, which the same endpoint accepts.
        let found = first_text(&data, &["encryptUin", "encrypt_uin"])
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                UpstreamError::Upstream("上游没有返回 encrypt_uin（可用数字 uin 代替）".into())
            })?;
        *self.encrypt_uin_cache.lock().expect("encrypt uin") = Some(found.clone());
        Ok(found)
    }

    /// An agent for the login handshake: no redirect following, because two of
    /// its steps answer with the redirect *as the result* (the cookie on the
    /// response, and the `code` in its `Location`).
    pub fn login_agent(&self) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .max_redirects(0)
            .build()
            .into()
    }

    /// Run one call with an explicit `tmeLoginType` in `comm`.
    ///
    /// The login endpoints select their behaviour with it (2 for the QQ Connect
    /// exchange, 6 for the phone scanner), and it is not a device property, so
    /// it is a parameter here rather than something the profile decides.
    pub fn call_with_tme_login_type(
        &self,
        credential: &Credential,
        platform: Platform,
        login_type: i64,
        call: Call,
    ) -> Result<Value, UpstreamError> {
        let mut body = self.envelope(credential, platform, vec![call])?;
        if let Some(comm) = body
            .get_mut("comm")
            .and_then(serde_json::Value::as_object_mut)
        {
            comm.insert("tmeLoginType".into(), json!(login_type));
        }
        let response = self.post_json(credential, Class::Account, MUSICU_ENDPOINT, &body, &[])?;
        let slot = response
            .get("req_0")
            .ok_or_else(|| UpstreamError::Upstream("响应里没有 req_0".into()))?;
        let code = slot
            .get("code")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        match slot.get("data") {
            Some(serde_json::Value::Object(data)) if !data.is_empty() => {
                Ok(serde_json::Value::Object(data.clone()))
            }
            _ => Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{}",
                slot.get("msg")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
            ))),
        }
    }

    /// The device the component presents, and the identity it has for it.
    pub fn device(&self) -> &DeviceStore {
        &self.device
    }

    /// Run one `req_0` call and return its `data` object, under the web profile.
    pub fn call(
        &self,
        credential: &Credential,
        class: Class,
        call: Call,
    ) -> Result<Value, UpstreamError> {
        self.call_with(credential, class, Platform::Web, call)
    }

    /// Run one `req_0` call on the **signed** `musics.fcg` route.
    ///
    /// The reference library's `sign=True` endpoints (乐谱、不喜欢列表) live on
    /// this route: the same `{comm, req_0}` envelope, but the URL carries a
    /// `zzc` signature over the exact bytes sent — the server answers `2000`
    /// (需要签名) when it does not verify, which is how a missing or wrong
    /// signature shows up. `params` are extra URL query parameters the endpoint
    /// wants alongside the signature.
    ///
    /// `comm_override` is the reference's `override_comm=True`: a few endpoints
    /// want their own `comm` block instead of the account one (the sheet-music
    /// reads ask as an anonymous `h5` caller, `uin ""` and `g_tk 5381`). Pass
    /// `None` for the ordinary account envelope.
    ///
    /// The signed endpoints put their business status inside `data`
    /// (`Retcode`); a non-zero outer code with a non-empty `data` is still the
    /// endpoint answering (an empty 乐谱 answers `10007`), so only an empty
    /// slot is treated as an error here.
    pub fn call_signed(
        &self,
        credential: &Credential,
        class: Class,
        platform: Platform,
        call: Call,
        params: &[(&str, String)],
        comm_override: Option<Value>,
    ) -> Result<Value, UpstreamError> {
        let mut envelope = match comm_override {
            Some(comm) => json!({ "comm": comm }),
            None => self.envelope(credential, platform, Vec::new())?,
        };
        envelope["req_0"] = json!({
            "module": call.module,
            "method": call.method,
            "param": call.param,
        });
        // The signature must cover the bytes actually sent, so the body is
        // serialized once here and posted verbatim.
        let body = serde_json::to_string(&envelope)
            .map_err(|error| UpstreamError::Upstream(error.to_string()))?;
        let mut query: Vec<(String, String)> = vec![
            ("_".into(), now_millis().to_string()),
            (
                "sign".into(),
                crate::port::signed::zzc_sign(body.as_bytes()),
            ),
        ];
        query.extend(
            params
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone())),
        );
        let url = format!("{MUSICS_ENDPOINT}?{}", encode_query(&query));
        let response = self.post_raw(credential, class, &url, &body)?;
        let slot = response
            .get("req_0")
            .ok_or_else(|| UpstreamError::Upstream("响应里没有 req_0".into()))?;
        let code = slot.get("code").and_then(Value::as_i64).unwrap_or(0);
        if let Some(reason) = refusal_reason(code) {
            return Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{reason}"
            )));
        }
        match slot.get("data") {
            Some(Value::Object(data)) if !data.is_empty() => Ok(Value::Object(data.clone())),
            _ => Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{}",
                slot.get("msg").and_then(Value::as_str).unwrap_or("")
            ))),
        }
    }

    /// Run one `req_0` call under an explicit platform profile.
    pub fn call_with(
        &self,
        credential: &Credential,
        class: Class,
        platform: Platform,
        call: Call,
    ) -> Result<Value, UpstreamError> {
        let envelope = self.envelope(credential, platform, vec![call])?;
        let response = self.post_json(credential, class, MUSICU_ENDPOINT, &envelope, &[])?;
        let slot = response
            .get("req_0")
            .ok_or_else(|| UpstreamError::Upstream("响应里没有 req_0".into()))?;
        let code = slot.get("code").and_then(Value::as_i64).unwrap_or(0);
        // The refusal codes are errors *even when data came along*: risk control
        // answers 2001 with an empty result set, and reading that as "no matches"
        // is how a throttled search turns into an empty page that looks like the
        // catalogue has nothing. (The reference library raises on the same four.)
        if let Some(reason) = refusal_reason(code) {
            return Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{reason}"
            )));
        }
        match slot.get("data") {
            Some(Value::Object(data)) if !data.is_empty() => Ok(Value::Object(data.clone())),
            _ => Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{}",
                slot.get("msg").and_then(Value::as_str).unwrap_or("")
            ))),
        }
    }

    /// The account's own asset list (`reqtype` 2 = albums, 3 = playlists).
    ///
    /// The legacy endpoint wants the *numeric* uin as a query parameter, and the
    /// cookies for the authorisation. `format=json` is not optional: without it
    /// the route answers something this parser cannot read, which looks exactly
    /// like an account with no collections.
    ///
    /// An empty `data` is reported as a refusal rather than as "you have none".
    /// The app caches whatever list this returns — a session that has gone stale
    /// answers with an empty envelope, and passing that on would wipe the
    /// favourites the user was looking at.
    pub fn profile_assets(
        &self,
        credential: &Credential,
        reqtype: u32,
        limit: u32,
    ) -> Result<Value, UpstreamError> {
        if !credential.is_usable() {
            return Err(UpstreamError::Upstream("需要登录后才能读取".into()));
        }
        let url = format!(
            "{PROFILE_ASSETS_ENDPOINT}?ct=20&cid=205360956&userid={}&reqtype={reqtype}\
&sin=0&ein={limit}&format=json",
            credential.music_id
        );
        let response = self.get_json(credential, Class::Account, &url)?;
        let code = first_int(&response, &["code", "retcode"]).unwrap_or(0);
        if code != 0 {
            let message = first_text(&response, &["msg", "message", "subcode"]).unwrap_or_default();
            return Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{message}"
            )));
        }
        match response.get("data") {
            Some(Value::Object(data)) if !data.is_empty() => Ok(Value::Object(data.clone())),
            _ => Err(UpstreamError::Upstream(format!(
                "上游没有返回数据（code={code}）：登录可能已过期"
            ))),
        }
    }

    /// GET a legacy `c.y.qq.com` fcgi route and return the parsed object.
    ///
    /// Those routes answer with a JSON body that some of them wrap in a JSONP
    /// callback, which is why the caller gets the whole object and not a slot.
    pub fn get_fcgi(
        &self,
        credential: &Credential,
        class: Class,
        url: &str,
    ) -> Result<Value, UpstreamError> {
        self.get_json(credential, class, url)
    }

    fn envelope(
        &self,
        credential: &Credential,
        platform: Platform,
        calls: Vec<Call>,
    ) -> Result<Value, UpstreamError> {
        let mut comm = serde_json::Map::new();
        comm.insert("format".into(), json!("json"));
        comm.insert("inCharset".into(), json!("utf-8"));
        comm.insert("outCharset".into(), json!("utf-8"));
        comm.insert("notice".into(), json!(0));
        comm.insert(
            "uin".into(),
            json!(if credential.music_id.is_empty() {
                "0".to_string()
            } else {
                credential.music_id.clone()
            }),
        );
        comm.insert("g_tk".into(), json!(credential.g_tk()));
        for (key, value) in platform.comm_overlay() {
            comm.insert(key.into(), value);
        }
        if platform == Platform::Android {
            for (key, value) in self.android_comm(credential) {
                comm.insert(key, value);
            }
            // The session is what the search, feed and write endpoints want on
            // top of the device identity; an empty one is sent when the login
            // step has not happened yet, and those endpoints say so themselves.
            if let Some((uid, sid)) = self.ensure_session(credential) {
                comm.insert("uid".into(), json!(uid));
                comm.insert("sid".into(), json!(sid));
            } else {
                comm.insert("uid".into(), json!(""));
                comm.insert("sid".into(), json!(""));
            }
        }
        let mut body = json!({ "comm": comm });
        for (index, call) in calls.into_iter().enumerate() {
            body[format!("req_{index}")] = json!({
                "module": call.module,
                "method": call.method,
                "param": call.param,
            });
        }
        Ok(body)
    }

    /// The android profile's own `comm` fields: the credential in the body, the
    /// device, and the QIMEI identity. No `uid`/`sid` — those are the session,
    /// and asking for the session needs this block already assembled.
    fn android_comm(&self, credential: &Credential) -> Vec<(String, serde_json::Value)> {
        let mut fields: Vec<(String, serde_json::Value)> = Vec::new();
        if !credential.music_id.is_empty() {
            fields.push(("qq".into(), json!(credential.music_id)));
        }
        if !credential.music_key.is_empty() {
            fields.push(("authst".into(), json!(credential.music_key)));
        }
        fields.push(("tmeAppID".into(), json!("qqmusic")));
        fields.push(("tmeLoginType".into(), json!(1)));
        fields.push(("chid".into(), json!("10003505")));
        let device = self.device.load_or_create();
        fields.push(("OpenUDID".into(), json!(device.open_udid)));
        fields.push(("OpenUDID2".into(), json!(device.open_udid)));
        fields.push(("udid".into(), json!(device.open_udid)));
        fields.push(("aid".into(), json!(device.android_id)));
        fields.push(("os_ver".into(), json!(device.os_version_text())));
        fields.push(("phonetype".into(), json!(device.model)));
        fields.push(("devicelevel".into(), json!(device.os_sdk)));
        fields.push(("newdevicelevel".into(), json!(device.os_sdk)));
        fields.push(("rom".into(), json!(device.proc_version)));
        let (q16, q36) = self
            .device
            .qimei(&self.agent)
            .unwrap_or_else(|| (String::new(), String::new()));
        fields.push(("QIMEI".into(), json!(q16)));
        fields.push(("QIMEI36".into(), json!(q36)));
        fields
    }

    /// The device session, obtained once a day.
    ///
    /// `music.getSession.session / GetSession` answers `{uid, sid, vkey}` for a
    /// request that carries the device identity. This is the second half of what
    /// the android interfaces want: a device that has logged in, not merely one
    /// that is identifiable.
    fn ensure_session(&self, credential: &Credential) -> Option<(String, String)> {
        if let Some(session) = self.device.load_or_create().fresh_session() {
            return Some(session);
        }
        let mut comm = serde_json::Map::new();
        comm.insert("format".into(), json!("json"));
        comm.insert("inCharset".into(), json!("utf-8"));
        comm.insert("outCharset".into(), json!("utf-8"));
        comm.insert("notice".into(), json!(0));
        comm.insert(
            "uin".into(),
            json!(if credential.music_id.is_empty() {
                "0".to_string()
            } else {
                credential.music_id.clone()
            }),
        );
        comm.insert("g_tk".into(), json!(credential.g_tk()));
        for (key, value) in Platform::Android.comm_overlay() {
            comm.insert(key.into(), value);
        }
        for (key, value) in self.android_comm(credential) {
            comm.insert(key, value);
        }
        let body = json!({
            "comm": comm,
            "req_0": {
                "module": "music.getSession.session",
                "method": "GetSession",
                "param": { "uid": "", "vkey": 0, "caller": 0 },
            },
        });
        let response = self
            .post_json(credential, Class::Account, MUSICU_ENDPOINT, &body, &[])
            .ok()?;
        let session = response
            .get("req_0")
            .and_then(|slot| slot.get("data"))
            .and_then(|data| data.get("session"))?;
        let uid = session.get("uid").map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })?;
        let sid = session
            .get("sid")
            .and_then(serde_json::Value::as_str)?
            .to_string();
        let vkey = session
            .get("vkey")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        self.device.apply_session(&uid, &sid, vkey);
        Some((uid, sid))
    }

    fn post_json(
        &self,
        credential: &Credential,
        class: Class,
        url: &str,
        body: &Value,
        extra_headers: &[(&str, &str)],
    ) -> Result<Value, UpstreamError> {
        if let Some(reason) = self.breaker.check() {
            return Err(UpstreamError::Refused(reason));
        }
        self.limiter.acquire(class);

        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Referer", "https://y.qq.com/");
        let cookies = credential.cookie_header();
        if !cookies.is_empty() {
            request = request.header("Cookie", &cookies);
        }
        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }

        match request.send_json(body) {
            Ok(mut response) => {
                let bytes = response
                    .body_mut()
                    .read_to_vec()
                    .map_err(|error| UpstreamError::Transport(error.to_string()))?;
                let value = decode_post_json(&bytes, body)
                    .map_err(|error| UpstreamError::Transport(error.to_string()))?;
                self.breaker.record_success();
                Ok(value)
            }
            Err(error) => {
                self.breaker.record_failure();
                Err(UpstreamError::Transport(error.to_string()))
            }
        }
    }

    /// POST a body the caller serialized, verbatim.
    ///
    /// The signed route must sign the exact bytes it sends, so it cannot go
    /// through `post_json`'s re-serialization.
    fn post_raw(
        &self,
        credential: &Credential,
        class: Class,
        url: &str,
        body: &str,
    ) -> Result<Value, UpstreamError> {
        if let Some(reason) = self.breaker.check() {
            return Err(UpstreamError::Refused(reason));
        }
        self.limiter.acquire(class);

        let mut request = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Referer", "https://y.qq.com/");
        let cookies = credential.cookie_header();
        if !cookies.is_empty() {
            request = request.header("Cookie", &cookies);
        }

        match request.send(body) {
            Ok(mut response) => {
                let bytes = response
                    .body_mut()
                    .read_to_vec()
                    .map_err(|error| UpstreamError::Transport(error.to_string()))?;
                let value = decode_upstream_json(&bytes)
                    .map_err(|error| UpstreamError::Transport(error.to_string()))?;
                self.breaker.record_success();
                Ok(value)
            }
            Err(error) => {
                self.breaker.record_failure();
                Err(UpstreamError::Transport(error.to_string()))
            }
        }
    }

    fn get_json(
        &self,
        credential: &Credential,
        class: Class,
        url: &str,
    ) -> Result<Value, UpstreamError> {
        if let Some(reason) = self.breaker.check() {
            return Err(UpstreamError::Refused(reason));
        }
        self.limiter.acquire(class);

        let mut request = self.agent.get(url).header("Referer", "https://y.qq.com/");
        let cookies = credential.cookie_header();
        if !cookies.is_empty() {
            request = request.header("Cookie", &cookies);
        }
        match request.call() {
            Ok(mut response) => {
                let bytes = response
                    .body_mut()
                    .read_to_vec()
                    .map_err(|error| UpstreamError::Transport(error.to_string()))?;
                let value = decode_upstream_json(&bytes)
                    .map_err(|error| UpstreamError::Transport(error.to_string()))?;
                self.breaker.record_success();
                Ok(value)
            }
            Err(error) => {
                self.breaker.record_failure();
                Err(UpstreamError::Transport(error.to_string()))
            }
        }
    }
}

/// PlaylistBaseWrite and PlaylistDetailWrite answer with GBK bytes even when their declared charset is
/// UTF-8 (verified for a Chinese playlist name). Allow that legacy encoding
/// only for this request module and only when the response is not valid UTF-8.
fn decode_post_json(bytes: &[u8], request: &Value) -> Result<Value, serde_json::Error> {
    let is_playlist_write = request.as_object().is_some_and(|envelope| {
        envelope.iter().any(|(name, slot)| {
            name.starts_with("req_")
                && matches!(
                    slot.get("module").and_then(Value::as_str),
                    Some(
                        "music.musicasset.PlaylistBaseWrite"
                            | "music.musicasset.PlaylistDetailWrite"
                    )
                )
        })
    });
    if is_playlist_write && std::str::from_utf8(bytes).is_err() {
        let (decoded, had_errors) = encoding_rs::GBK.decode_without_bom_handling(bytes);
        if !had_errors {
            return decode_upstream_json(decoded.as_bytes());
        }
    }
    decode_upstream_json(bytes)
}

/// Decode upstream JSON, tolerating only unpaired UTF-16 surrogate escapes.
///
/// Some upstream strings can contain a lone `\uD800`-style code unit, which
/// Python's JSON decoder accepts but Rust strings cannot represent. Replace
/// those units with U+FFFD; serde still validates every other part of the JSON.
fn decode_upstream_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Ok(value),
        Err(error) => match replace_unpaired_surrogates(bytes) {
            Some(normalized) => serde_json::from_slice(&normalized),
            None => Err(error),
        },
    }
}

fn replace_unpaired_surrogates(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut normalized: Option<Vec<u8>> = None;
    let mut copied_until = 0;
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                if let Some(unit) = unicode_escape_at(bytes, index) {
                    if (0xD800..=0xDBFF).contains(&unit) {
                        if unicode_escape_at(bytes, index + 6)
                            .is_some_and(|next| (0xDC00..=0xDFFF).contains(&next))
                        {
                            index += 12;
                            continue;
                        }
                    } else if !(0xDC00..=0xDFFF).contains(&unit) {
                        index += 6;
                        continue;
                    }
                    let output = normalized.get_or_insert_with(|| Vec::with_capacity(bytes.len()));
                    output.extend_from_slice(&bytes[copied_until..index]);
                    output.extend_from_slice(br"\uFFFD");
                    index += 6;
                    copied_until = index;
                } else {
                    // Consume the escaped character with its backslash. This
                    // keeps literal `\\uD800` and escaped quotes untouched.
                    index += 2;
                }
            }
            _ => index += 1,
        }
    }
    if let Some(output) = normalized.as_mut() {
        output.extend_from_slice(&bytes[copied_until..]);
    }
    normalized
}

fn unicode_escape_at(bytes: &[u8], index: usize) -> Option<u16> {
    let escape = bytes.get(index..index.checked_add(6)?)?;
    if escape[0] != b'\\' || escape[1] != b'u' {
        return None;
    }
    escape[2..].iter().try_fold(0u16, |value, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        Some(value * 16 + u16::from(digit))
    })
}

/// The refusal codes the reference library treats as errors, with the same
/// meanings — 2001 is the one that matters in practice, because the upstream
/// answers it with a *successful-looking* empty result set.
fn refusal_reason(code: i64) -> Option<&'static str> {
    match code {
        2000 => Some("需要签名"),
        2001 => Some("触发风控：请稍后再试（应用会退避）"),
        1000 | 104401 | 104400 => Some("登录已过期，请重新登录"),
        _ => None,
    }
}

/// Milliseconds since the Unix epoch, for the `_` anti-cache query parameter.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// Percent-encode a query the way a form body would be written.
///
/// The signed route's `_`/`sign` pair is the one place this component builds a
/// query by hand, so this lives next to its caller.
fn encode_query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", encode_component(key), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Pick the first present key out of `keys`, as a string.
pub fn first_text(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value.get(*key) {
            match found {
                Value::String(text) if !text.is_empty() => return Some(text.clone()),
                Value::Number(number) => return Some(number.to_string()),
                _ => {}
            }
        }
    }
    None
}

/// Pick the first present key out of `keys`, as an integer.
pub fn first_int(value: &Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(found) = value.get(*key) {
            match found {
                Value::Number(number) => return number.as_i64(),
                Value::String(text) => {
                    if let Ok(parsed) = text.parse::<i64>() {
                        return Some(parsed);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// The first object inside `keys` (the upstream nests the same entity under
/// different names depending on the endpoint).
pub fn first_object<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .filter(|found| found.is_object())
}

/// A list nested under any of `keys`.
pub fn first_array<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(Value::as_array)
}

#[cfg(test)]
mod tests {
    use super::{decode_post_json, decode_upstream_json, replace_unpaired_surrogates};
    use serde_json::json;

    #[test]
    fn playlist_write_decodes_legacy_gbk_chinese_result() {
        let request = json!({"req_0": {"module": "music.musicasset.PlaylistBaseWrite"}});
        let bytes =
            b"{\"req_0\":{\"code\":0,\"data\":{\"result\":{\"dirName\":\"\xb2\xe2\xca\xd4\"}}}}";
        assert!(std::str::from_utf8(bytes).is_err());
        let expected = json!({"req_0": {"code": 0, "data": {"result": {"dirName": "测试"}}}});
        assert_eq!(decode_post_json(bytes, &request).unwrap(), expected);
        let detail_request = json!({"req_0": {"module": "music.musicasset.PlaylistDetailWrite"}});
        assert_eq!(decode_post_json(bytes, &detail_request).unwrap(), expected);
        assert_eq!(
            decode_post_json(expected.to_string().as_bytes(), &request).unwrap(),
            expected
        );
        let other = json!({"req_0": {"module": "music.musicasset.PlaylistBaseRead"}});
        assert!(decode_post_json(bytes, &other).is_err());
        assert!(decode_post_json(bytes, &json!({})).is_err());
    }

    #[test]
    fn playlist_write_keeps_json_and_legacy_encoding_validation() {
        let request = json!({"req_0": {"module": "music.musicasset.PlaylistBaseWrite"}});
        // A truncated GBK lead byte must never be decoded with replacement.
        assert!(decode_post_json(b"\"\x81\"", &request).is_err());
        assert!(decode_post_json(b"\xb2\xe2\xca\xd4", &request).is_err());
        assert!(decode_post_json(b"{\"name\":\"\xb2\xe2\xca\xd4\",}", &request).is_err());
        // Valid UTF-8 stays strict, even for this legacy module.
        assert!(decode_post_json(br#"{"name":"test",}"#, &request).is_err());
    }

    #[test]
    fn valid_unicode_pairs_and_literals_are_unchanged() {
        let bytes = br#"{"pair":"\uD83D\uDE00","literal":"\\uD800","quote":"\"","text":"\u4e2d"}"#;
        assert!(replace_unpaired_surrogates(bytes).is_none());
        assert_eq!(
            decode_upstream_json(bytes).unwrap(),
            json!({"pair": "😀", "literal": r"\uD800", "quote": "\"", "text": "中"})
        );
    }

    #[test]
    fn lone_surrogates_are_replaced_inside_strings_and_keys() {
        assert_eq!(
            decode_upstream_json(
                br#"{"\uD800":"a\ud800b\uDC00c","pair":"\uD83D\uDE00","literal":"\\uDC00"}"#
            )
            .unwrap(),
            json!({"�": "a�b�c", "pair": "😀", "literal": r"\uDC00"})
        );
        assert_eq!(
            decode_upstream_json(br#""\uDBFF\uD800\uDC00\uDFFF""#).unwrap(),
            json!("�𐀀�")
        );
    }

    #[test]
    fn ordinary_non_string_json_is_unchanged() {
        for bytes in [b"null".as_slice(), b"123", b"true", b"[1,false,null]"] {
            assert!(replace_unpaired_surrogates(bytes).is_none());
            assert_eq!(
                decode_upstream_json(bytes).unwrap(),
                serde_json::from_slice::<serde_json::Value>(bytes).unwrap()
            );
        }
        assert!(replace_unpaired_surrogates(br"\uD800").is_none());
        assert!(decode_upstream_json(br"\uD800").is_err());
    }

    #[test]
    fn all_other_malformed_json_remains_rejected() {
        for bytes in [
            br#"{"value":"\uD800",}"#.as_slice(),
            br#""\uD800\uZZZZ""#,
            br#""\uD800\x""#,
            br#""\uD800" trailing"#,
            br#""\uD800"#,
            b"\"\\uD800\n\"",
            b"\"\\uD800\xff\"",
            br#"["\uD800" true]"#,
        ] {
            assert!(
                decode_upstream_json(bytes).is_err(),
                "accepted malformed JSON: {bytes:?}"
            );
        }
    }
}

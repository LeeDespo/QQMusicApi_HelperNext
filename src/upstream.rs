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
use crate::guard::{Class, CircuitBreaker, RateLimit};
use serde_json::{json, Value};
use std::time::Duration;

const MUSICU_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";

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
const PROFILE_ASSETS_ENDPOINT: &str = "https://c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg";

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
        }
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
        if let Some(comm) = body.get_mut("comm").and_then(serde_json::Value::as_object_mut) {
            comm.insert("tmeLoginType".into(), json!(login_type));
        }
        let response = self.post_json(credential, Class::Account, MUSICU_ENDPOINT, &body, &[])?;
        let slot = response
            .get("req_0")
            .ok_or_else(|| UpstreamError::Upstream("响应里没有 req_0".into()))?;
        let code = slot.get("code").and_then(serde_json::Value::as_i64).unwrap_or(0);
        match slot.get("data") {
            Some(serde_json::Value::Object(data)) if !data.is_empty() => {
                Ok(serde_json::Value::Object(data.clone()))
            }
            _ => Err(UpstreamError::Upstream(format!(
                "上游返回错误（{code}）：{}",
                slot.get("msg").and_then(serde_json::Value::as_str).unwrap_or("")
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
    /// cookies for the authorisation.
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
            "{PROFILE_ASSETS_ENDPOINT}?ct=20&cid=205360956&userid={}&reqtype={reqtype}&sin=0&ein={limit}",
            credential.music_id
        );
        let response = self.get_json(credential, Class::Account, &url)?;
        response
            .get("data")
            .cloned()
            .ok_or_else(|| UpstreamError::Upstream("响应里没有 data".into()))
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
            json!(if credential.music_id.is_empty() { "0".to_string() } else { credential.music_id.clone() }),
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
            json!(if credential.music_id.is_empty() { "0".to_string() } else { credential.music_id.clone() }),
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
        let sid = session.get("sid").and_then(serde_json::Value::as_str)?.to_string();
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
                let value: Value = response
                    .body_mut()
                    .read_json()
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
                let value: Value = response
                    .body_mut()
                    .read_json()
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

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
use crate::guard::{Class, CircuitBreaker, RateLimit};
use serde_json::{json, Value};
use std::time::Duration;

const MUSICU_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
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
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(12)))
                .build()
                .into(),
            limiter: RateLimit::new(Duration::from_secs(10)),
            breaker: CircuitBreaker::default(),
        }
    }

    /// Run one `req_0` call and return its `data` object.
    pub fn call(
        &self,
        credential: &Credential,
        class: Class,
        call: Call,
    ) -> Result<Value, UpstreamError> {
        let envelope = self.envelope(credential, vec![call])?;
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

    fn envelope(&self, credential: &Credential, calls: Vec<Call>) -> Result<Value, UpstreamError> {
        let mut body = json!({
            "comm": {
                "cv": 4747474,
                "ct": 24,
                "format": "json",
                "inCharset": "utf-8",
                "outCharset": "utf-8",
                "notice": 0,
                "platform": "yqq.json",
                "needNewCode": 1,
                "uin": if credential.music_id.is_empty() { "0".to_string() } else { credential.music_id.clone() },
                "g_tk": credential.g_tk(),
            }
        });
        for (index, call) in calls.into_iter().enumerate() {
            body[format!("req_{index}")] = json!({
                "module": call.module,
                "method": call.method,
                "param": call.param,
            });
        }
        Ok(body)
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

//! Logging in.
//!
//! Two ways in, both of them entirely inside this component — which matters
//! because the component owns its own credential store, so a host that cannot
//! produce cookies has no other way to establish a session:
//!
//! * [`start_login`] / [`poll_login`] — scan a QR with the QQ app. Five HTTP
//!   steps, reproduced from QQMusicApi's own `modules/login.py`:
//!   `ptqrshow` (the image and a `qrsig` cookie), `ptqrlogin` (the scan state),
//!   `check_sig` (a `p_skey`), `graph.qq.com/oauth2.0/authorize` (a `code`), and
//!   finally `QQConnectLogin.LoginServer/QQLogin`, which answers the credential.
//! * [`crate::api::import_credential`] — hand over `uin` + `qm_keyst` from a
//!   login a host performed itself (a web view, for instance). That pair *is* a
//!   complete login: `qm_keyst` is also the CDN playback ticket.
//!
//! The QR flow is stateless between calls on purpose: the `qrsig` the first step
//! returns is the identifier the caller hands back when polling, so no login
//! state has to survive in the component (the Python helper did the same).

use crate::credential::{hash33, hash33_seeded, Credential, CredentialStore};
use crate::upstream::{first_text, Platform, Upstream, UpstreamError};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

const QR_SHOW: &str = "https://ssl.ptlogin2.qq.com/ptqrshow";
const QR_POLL: &str = "https://ssl.ptlogin2.qq.com/ptqrlogin";
const CHECK_SIG: &str = "https://ssl.ptlogin2.graph.qq.com/check_sig";
const AUTHORIZE: &str = "https://graph.qq.com/oauth2.0/authorize";
/// ptlogin2 answers `403` to anything that does not look like a browser, so the
/// handshake carries a browser's own user agent. (The library's HTTP session
/// sends one for the same reason.)
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";

/// QQ Connect's application id, as the library uses it.
const CONNECT_APP_ID: &str = "100497308";
const CONNECT_AID: &str = "716027609";
const CONNECT_DAID: &str = "383";

/// What a scan has done so far. The names are the component's own vocabulary;
/// the wire codes they come from are `ptuiCB`'s first argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginEvent {
    /// Nobody has scanned it yet (66, 408).
    Scan,
    /// Scanned, waiting for the user to confirm on their phone (67, 404).
    Conf,
    /// The code expired (65, 402).
    Timeout,
    /// The user refused (68, 403).
    Refuse,
    /// Login finished (0, 405) — the credential is stored.
    Done,
}

impl LoginEvent {
    fn from_code(code: i64) -> Option<Self> {
        match code {
            0 | 405 => Some(LoginEvent::Done),
            66 | 408 => Some(LoginEvent::Scan),
            67 | 404 => Some(LoginEvent::Conf),
            65 | 402 => Some(LoginEvent::Timeout),
            68 | 403 => Some(LoginEvent::Refuse),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LoginEvent::Scan => "SCAN",
            LoginEvent::Conf => "CONF",
            LoginEvent::Timeout => "TIMEOUT",
            LoginEvent::Refuse => "REFUSE",
            LoginEvent::Done => "DONE",
        }
    }
}

/// A QR code to draw, plus what to poll it with.
pub struct LoginQrCode {
    /// The `qrsig` cookie: the caller echoes it back to `poll_login`.
    pub identifier: String,
    pub mimetype: String,
    /// The PNG, base64-encoded — the shape a host can put straight into an
    /// `<img>` or an `NSImage`.
    pub image_base64: String,
}

/// Start a QQ scan-to-login.
pub fn start_login(upstream: &Upstream) -> Result<LoginQrCode, UpstreamError> {
    let agent = upstream.login_agent();
    let url = format!(
        "{QR_SHOW}?appid={CONNECT_AID}&e=2&l=M&s=3&d=72&v=4&t={nonce}&daid={CONNECT_DAID}&pt_3rd_aid={CONNECT_APP_ID}",
        nonce = rand_nonce()
    );
    let response = agent
        .get(&url)
        .header("Referer", "https://xui.ptlogin2.qq.com/")
        .header("User-Agent", BROWSER_UA)
        .call()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    let cookies = collect_cookies(&response);
    let identifier = cookies.get("qrsig").cloned().ok_or_else(|| {
        UpstreamError::Upstream("二维码响应里没有 qrsig".into())
    })?;
    let mut response = response;
    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    if bytes.is_empty() {
        return Err(UpstreamError::Upstream("二维码图片为空".into()));
    }
    Ok(LoginQrCode {
        identifier,
        mimetype: "image/png".into(),
        image_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

/// Ask what the scan has done so far, and finish the login when it is done.
pub fn poll_login(
    upstream: &Upstream,
    store: &CredentialStore,
    identifier: &str,
) -> Result<Value, UpstreamError> {
    let agent = upstream.login_agent();
    // ptlogin's token starts from seed 0 — *not* the 5381 `g_tk` uses. The
    // library's `hash33` defaults to 0 for precisely this call.
    let token = hash33_seeded(identifier, 0);
    let url = format!(
        "{QR_POLL}?u1=https%3A%2F%2Fgraph.qq.com%2Foauth2.0%2Flogin_jump&ptqrtoken={token}\
&ptredirect=0&h=1&t=1&g=1&from_ui=1&ptlang=2052&action=0-0-{millis}&js_ver=20102616&js_type=1\
&pt_uistyle=40&aid={CONNECT_AID}&daid={CONNECT_DAID}&pt_3rd_aid={CONNECT_APP_ID}&has_onekey=1",
        millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0)
    );
    let mut response = agent
        .get(&url)
        .header("Referer", "https://xui.ptlogin2.qq.com/")
        .header("Origin", "https://xui.ptlogin2.qq.com")
        .header("User-Agent", BROWSER_UA)
        .header("Cookie", format!("qrsig={identifier}"))
        .call()
        .map_err(poll_refused)?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;

    let (code, payload) = parse_ptui(&text)?;
    let event = LoginEvent::from_code(code)
        .ok_or_else(|| UpstreamError::Upstream(format!("无法识别的扫码状态码：{code}")))?;
    if event != LoginEvent::Done {
        return Ok(json!({ "event": event.as_str(), "loggedIn": false }));
    }

    // Logged in: the redirect url carries the account and the ticket to trade.
    let uin = extract(&payload, "uin=").ok_or_else(|| UpstreamError::Upstream("缺少 uin".into()))?;
    let sigx = extract(&payload, "ptsigx=").ok_or_else(|| UpstreamError::Upstream("缺少 ptsigx".into()))?;
    exchange_for_credential(upstream, store, &uin, &sigx)?;
    let status = crate::methods::dispatch(upstream, store.load().as_ref(), "get_login_status", &json!({}))
        .unwrap_or_else(|_| json!({ "login": { "loggedIn": true } }));
    Ok(json!({ "event": "DONE", "loggedIn": true, "login": status.get("login") }))
}

/// Trade the scanned ticket for a credential: `check_sig` → `authorize` →
/// `QQLogin`.
fn exchange_for_credential(
    upstream: &Upstream,
    store: &CredentialStore,
    uin: &str,
    sigx: &str,
) -> Result<(), UpstreamError> {
    let agent = upstream.login_agent();
    // `check_sig` answers with a redirect we must not follow: the cookie on the
    // response *is* the result.
    let url = format!(
        "{CHECK_SIG}?uin={uin}&pttype=1&service=ptqrlogin&nodirect=0&ptsigx={sigx}\
&s_url=https%3A%2F%2Fgraph.qq.com%2Foauth2.0%2Flogin_jump&ptlang=2052&ptredirect=100\
&aid={CONNECT_AID}&daid={CONNECT_DAID}&j_later=0&low_login_hour=0&regmaster=0\
&pt_login_type=3&pt_aid=0&pt_aaid=16&pt_light=0&pt_3rd_aid={CONNECT_APP_ID}"
    );
    let response = agent
        .get(&url)
        .header("Referer", "https://xui.ptlogin2.qq.com/")
        .header("User-Agent", BROWSER_UA)
        .call()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    let cookies = collect_cookies(&response);
    let p_skey = cookies
        .get("p_skey")
        .cloned()
        .ok_or_else(|| UpstreamError::Upstream("换取 p_skey 失败".into()))?;

    // Then `authorize`, whose redirect carries the `code` the music API takes.
    let cookie_header = cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    let form = [
        ("response_type", "code"),
        ("client_id", CONNECT_APP_ID),
        ("redirect_uri", "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/"),
        ("scope", "get_user_info,get_app_friends"),
        ("state", "state"),
        ("switch", ""),
        ("from_ptlogin", "1"),
        ("src", "1"),
        ("update_auth", "1"),
        ("openapi", "1010_1030"),
        ("auth_time", &auth_time_string()),
        ("ui", &rand_uuid()),
    ];
    let g_tk = hash33(&p_skey).to_string();
    let mut body_parts: Vec<String> = form
        .iter()
        .map(|(name, value)| format!("{name}={}", encode(value)))
        .collect();
    body_parts.push(format!("g_tk={g_tk}"));
    let response = agent
        .post(AUTHORIZE)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", BROWSER_UA)
        .header("Cookie", cookie_header)
        .send(body_parts.join("&"))
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let code = extract(&location, "code=").ok_or_else(|| UpstreamError::Upstream("获取 code 失败".into()))?;

    // Finally the music API turns the connect code into this account's credential.
    let data = upstream.call_with_tme_login_type(
        &Credential::default(),
        Platform::Android,
        2,
        crate::upstream::Call {
            module: "QQConnectLogin.LoginServer",
            method: "QQLogin",
            param: json!({ "code": code }),
        },
    )?;
    let credential = Credential::from_json(&data)
        .ok_or_else(|| UpstreamError::Upstream("登录响应里没有凭据".into()))?;
    store
        .store(&credential)
        .map_err(|error| UpstreamError::Transport(error.to_string()))
}

/// Say what a refused status query means.
///
/// 403 here has exactly one cause in practice — the token or the cookie was not
/// accepted — and it is indistinguishable from a permission problem unless the
/// message says so. (Getting this wrong once cost a round of hunting through
/// referers, user agents and cookie jars; the fault was the `hash33` seed.)
fn poll_refused(error: ureq::Error) -> UpstreamError {
    match error {
        ureq::Error::StatusCode(code) => UpstreamError::Upstream(format!(
            "扫码状态查询被拒绝（HTTP {code}）：qrsig/ptqrtoken 未被接受 —— \
二维码可能已过期，重新生成即可；若每次都这样，是 token 算法问题"
        )),
        other => UpstreamError::Transport(other.to_string()),
    }
}

/// Read a `ptuiCB('0','0','url','0','msg','nick')` reply.
fn parse_ptui(text: &str) -> Result<(i64, String), UpstreamError> {
    let start = text.find("ptuiCB(").ok_or_else(|| {
        UpstreamError::Upstream(format!("扫码响应无法解析：{}", &text[..text.len().min(80)]))
    })?;
    let body = &text[start + "ptuiCB(".len()..];
    let end = body.find(')').unwrap_or(body.len());
    let args: Vec<String> = body[..end]
        .split(',')
        .map(|arg| arg.trim().trim_matches('\'').to_string())
        .collect();
    let code: i64 = args
        .first()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| UpstreamError::Upstream("扫码响应没有状态码".into()))?;
    let payload = args.get(2).cloned().unwrap_or_default();
    Ok((code, payload))
}

/// The `name=value` of a query or form fragment.
fn extract(text: &str, key: &str) -> Option<String> {
    let start = text.find(key)? + key.len();
    let rest = &text[start..];
    let end = rest.find('&').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn collect_cookies(response: &ureq::http::Response<ureq::Body>) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    for value in response.headers().get_all("set-cookie") {
        let Ok(text) = value.to_str() else { continue };
        for part in text.split(';') {
            let part = part.trim();
            if let Some((name, value)) = part.split_once('=') {
                if !name.is_empty() && !cookies.contains_key(name) {
                    cookies.insert(name.to_string(), value.to_string());
                }
            }
        }
    }
    cookies
}

fn rand_nonce() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!("0.{:016}", rng.gen::<u64>())
}

/// A `uuid4`-shaped `ui`, which is what the library sends to `authorize`.
fn rand_uuid() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `str(int(time()) * 1000)` — milliseconds, which is what the reference sends.
fn auth_time_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".into())
}

/// Percent-encode the characters a form value cannot carry raw.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// Kept for the settings page: how long a QR stays valid before the upstream
/// stops accepting it. Not enforced here — the upstream answers `TIMEOUT`.
pub const QR_LIFETIME: Duration = Duration::from_secs(180);

/// A credential's own fields, for the tests to read.
pub fn credential_uin(credential: &Credential) -> &str {
    &credential.music_id
}

/// The account's display name, when a response carries one.
pub fn nickname_from(value: &Value) -> Option<String> {
    first_text(value, &["nick", "name", "nickname"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptui_replies_are_parsed_into_code_and_payload() {
        let (code, payload) = parse_ptui(
            "ptuiCB('67','0','','0','二维码已扫描，请确认。','')",
        )
        .expect("parses");
        assert_eq!(code, 67);
        assert!(payload.is_empty());

        let (code, payload) = parse_ptui(
            "ptuiCB('0','0','https://ptlogin2.qq.com/login?uin=123&ptsigx=abc&k=1','0','登录成功','昵称')",
        )
        .expect("parses");
        assert_eq!(code, 0);
        assert_eq!(extract(&payload, "uin=").as_deref(), Some("123"));
        assert_eq!(extract(&payload, "ptsigx=").as_deref(), Some("abc"));
    }

    #[test]
    fn events_map_to_the_codes_the_service_uses() {
        assert_eq!(LoginEvent::from_code(0), Some(LoginEvent::Done));
        assert_eq!(LoginEvent::from_code(405), Some(LoginEvent::Done));
        assert_eq!(LoginEvent::from_code(66), Some(LoginEvent::Scan));
        assert_eq!(LoginEvent::from_code(67), Some(LoginEvent::Conf));
        assert_eq!(LoginEvent::from_code(65), Some(LoginEvent::Timeout));
        assert_eq!(LoginEvent::from_code(68), Some(LoginEvent::Refuse));
        assert_eq!(LoginEvent::from_code(999), None);
        assert_eq!(LoginEvent::Scan.as_str(), "SCAN");
    }

    #[test]
    fn form_values_are_percent_encoded() {
        assert_eq!(encode("https://y.qq.com/a?b=1&c=2"), "https%3A%2F%2Fy.qq.com%2Fa%3Fb%3D1%26c%3D2");
        assert_eq!(encode("abc-_.~"), "abc-_.~");
    }
}

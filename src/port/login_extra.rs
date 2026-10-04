//! 登录扩展：微信扫码、手机验证码、凭据刷新。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/login.py`
//! （`_get_wx_qr` / `_check_wx_qr` / `_authorize_wx_qr`、`send_authcode` /
//! `phone_authorize`、`refresh_credential`）。QQ 扫码与 cookie 导入已由既有
//! `src/login.rs` 覆盖，不在这里重复；手机客户端二维码（MQTT）不移植，
//! 见文件尾的说明。
//!
//! # 平台档案
//!
//! `send_authcode` / `phone_authorize` 参考里标了 `platform=ANDROID`（文档也说
//! 这两个接口固定 Android），所以本文件对它们**固定用 Android 档案**，调用方给
//! 什么都一样；`_authorize_wx_qr` / `refresh_credential` 没标档案，按本层约定
//! 默认 Web，调用方显式传 `platform` 时尊重调用方（与既有 `fetch_artist_detail`
//! 同款）。微信取码的三步是普通 HTTP，不涉及平台档案。
//!
//! # 为什么手拼 comm、走 `get_fcgi` 而不是 `call_with`
//!
//! 参考的这几个调用都带**每次请求的 comm 覆盖**（`tmeLoginType` / `tmeLoginMethod`），
//! 而且按 `req_0.code` 分派业务结果：`SendPhoneAuthCode` 的 `20276`（滑块验证）与
//! `100001`（发送频繁）是答案而不是错误，`Login` 的 `20271`（验证码错误）之类要报成
//! 精确的登录错误。组件的 `Upstream::call_with` 只回 `data`、并按风控码表把
//! `1000/104401/104400` 一律收成错误——这两个能力它都没有；`call_signed` 同样只回
//! `data`。因此这里照参考的语义自己拼 comm 与 `{comm, req_0}` 信封，经
//! `Upstream::get_fcgi`（`GET musicu.fcg?format=json&data=<整体编码的信封>`）发出，
//! 拿回**整个外层信封**再按参考的 `_validate_result` 分派。这条 GET 路与 POST
//! 返回同一个 `req_0.code`/`data`（2026-10-03 用真实号码与假号各验过一次），
//! 且仍然只走组件自己的 Upstream（限流、熔断、cookie 都在里面）。
//!
//! 微信取码的三步（页面 HTML、二维码 JPEG、长轮询文本）不是 JSON，只能走
//! `Upstream::login_agent()`——既有 QQ 扫码的五步就是这么做的（`src/login.rs`），
//! 本文件照同款先例。
//!
//! # 回值形状
//!
//! 包装层复用组件既有的登录模型：`fetch_wx_qrcode` → `LoginQrCode`（与
//! `start_login` 同一个形状，`loginType` 为 `"wx"`）、`check_wx_qrcode` →
//! `LoginPoll`（与 `poll_login` 同一个形状）、`phone_login` / `refresh_credential`
//! → `LoginStatus`。参考的 `QRLoginResult.credential` **不随 JSON 回值给出**：
//! 组件自己持有凭据文件（这是它的既有契约，凭据不进回值），拿到新凭据后直接写入
//! 存储——与 `poll_login` 的行为一致，否则宿主没有第二条路把微信/手机登录落盘。
//!
//! 空值判据：这些接口的"空"都是故障而不是答案（登录失败、凭据没换来、
//! 二维码没拿到），一律报错；它们也不做整表读取，不涉及"空列表"那条判据。
//!
//! # 不移植的部分：手机客户端二维码（MQTT）
//!
//! 参考的 `_get_mobile_qr` / `checking_mobile_qrcode` 要一条 WebSocket MQTT
//! 长连接（`mu.y.qq.com/ws/handshake`，自定义属性 `authorization`/`pubsub`）
//! 与服务端事件流，参考里也是异步生成器；它不是一次请求-回值，移植进来要么给
//! 组件加一个 MQTT 客户端与事件循环，要么伪装成一个假的"轮询"。两条都不做：
//! 微信扫码与手机验证码已经覆盖了这两条登录路径，MQTT 那条（手机 App 扫码）
//! 留待需要时再单独设计，见 openQuestions。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::models::{LoginPoll, LoginQrCode, LoginStatus};
use crate::upstream::{first_int, first_text, Platform, Upstream, UpstreamError};
use crate::Class;
use base64::Engine;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "fetch_wx_qrcode",
    "check_wx_qrcode",
    "send_phone_authcode",
    "phone_login",
    "refresh_credential",
];

/// 参考的登录端点模块名（`send_authcode` / `phone_authorize` / `_authorize_wx_qr` /
/// `refresh_credential` 全在它上面）。
const LOGIN_MODULE: &str = "music.login.LoginServer";
const MUSICU_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";

/// 微信开放平台的音乐应用 id（参考 `_get_wx_qr` 逐字给出）。
const WX_APP_ID: &str = "wx48db31d50e334801";
const WX_QRCONNECT: &str = "https://open.weixin.qq.com/connect/qrconnect";
const WX_QRCODE_IMAGE: &str = "https://open.weixin.qq.com/connect/qrcode";
const WX_QR_STATUS: &str = "https://lp.open.weixin.qq.com/connect/l/qrconnect";
/// `login_type=2` 就是微信（参考 `redirect_uri` 里的参数，逐字照抄）。
const WX_REDIRECT_URI: &str =
    "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/";
const WX_HREF: &str =
    "https://y.qq.com/mediastyle/music_v17/src/css/popup_wechat.css#wechat_redirect";
/// 微信长轮询一次最长压 35 秒（参考 `timeout=35.0`），超时按「扫码中」解释。
const WX_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);

/// ptlogin / 微信开放平台对非浏览器 UA 会回 403，与 `src/login.rs` 用同一个。
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";

/// 参考 `PhoneLoginEvents`：`SEND=0`、`CAPTCHA=20276`、`FREQUENCY=100001`。
const PHONE_EVENT_SEND: &str = "SEND";
const PHONE_EVENT_CAPTCHA: &str = "CAPTCHA";
const PHONE_EVENT_FREQUENCY: &str = "FREQUENCY";
const PHONE_CODE_CAPTCHA: i64 = 20276;
const PHONE_CODE_FREQUENCY: i64 = 100001;
/// 参考 `send_authcode(country_code: int = 86)`。
const DEFAULT_COUNTRY_CODE: i64 = 86;

// MARK: - 模型

/// 参考 `PhoneAuthCodeResult`：发验证码的结果。
///
/// `event` 取参考枚举名（`SEND` / `CAPTCHA` / `FREQUENCY`）；`CAPTCHA` 时
/// `info` 是滑块验证地址（参考读 `data.securityURL`）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PhoneAuthCodeResult {
    pub event: String,
    pub info: Option<String>,
}

// MARK: - 微信扫码（照参考的三步）

/// 取码第一步的地址（参考 `_get_wx_qr` 的 params，逐字）。
fn wx_qrcode_page_url() -> String {
    format!(
        "{WX_QRCONNECT}?appid={WX_APP_ID}&redirect_uri={}&response_type=code&scope=snsapi_login\
&state=STATE&href={}",
        urlencode(WX_REDIRECT_URI),
        urlencode(WX_HREF)
    )
}

/// 取码第二步的地址（参考 `f"https://open.weixin.qq.com/connect/qrcode/{uuid}"`）。
fn wx_qrcode_image_url(uuid: &str) -> String {
    format!("{WX_QRCODE_IMAGE}/{uuid}")
}

/// 长轮询地址；`_` 是参考的 `str(int(time()) * 1000)`（秒乘 1000，照抄）。
fn wx_status_url(uuid: &str, seconds: u64) -> String {
    format!("{WX_QR_STATUS}?uuid={uuid}&_={}", seconds * 1000)
}

/// 从 qrconnect 页面里取 uuid。
///
/// 参考的正则是 `uuid=(.+?)"`：取**第一个非空**的 `uuid="` 段（页面里还有一处
/// `uuid=""` 的模板串，正则不命中它，这里也一样跳过）。Python 的 `.` 不跨行，
/// 所以引号必须与 `uuid=` 同一行；否则跳到下一个 `uuid=` 再试。
fn parse_wx_uuid(html: &str) -> Result<String, UpstreamError> {
    let mut rest = html;
    while let Some((_, after)) = rest.split_once("uuid=") {
        let line = after.split('\n').next().unwrap_or(after);
        if let Some((value, _)) = line.split_once('"') {
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
        rest = after;
    }
    Err(UpstreamError::Upstream(
        "获取 uuid 失败：页面里没有 uuid".into(),
    ))
}

/// 解析长轮询的 `window.wx_errcode=<n>;window.wx_code='<code>'`。
///
/// 两段都必须出现（参考是**一条**正则，缺一段就是不匹配）。
fn parse_wx_status(text: &str) -> Result<(i64, String), UpstreamError> {
    let (_, rest) = text
        .split_once("window.wx_errcode=")
        .ok_or_else(|| UpstreamError::Upstream("获取二维码状态失败：无法解析响应".into()))?;
    let (digits, rest) = rest
        .split_once(';')
        .ok_or_else(|| UpstreamError::Upstream("获取二维码状态失败：无法解析响应".into()))?;
    let code: i64 = digits
        .trim()
        .parse()
        .map_err(|_| UpstreamError::Upstream("获取二维码状态失败：无效的错误码".into()))?;
    let (_, rest) = rest
        .split_once("window.wx_code='")
        .ok_or_else(|| UpstreamError::Upstream("获取二维码状态失败：无法解析响应".into()))?;
    let (wx_code, _) = rest
        .split_once('\'')
        .ok_or_else(|| UpstreamError::Upstream("获取二维码状态失败：无法解析响应".into()))?;
    Ok((code, wx_code.to_string()))
}

/// 参考 `QRCodeLoginEvents.get_by_value` 的状态码表。
///
/// 返回 None 表示"无法识别的二维码登录状态码"（参考抛 `ValueError`）。
fn wx_event(code: i64) -> Option<&'static str> {
    match code {
        0 | 405 => Some("DONE"),
        66 | 408 => Some("SCAN"),
        67 | 404 => Some("CONF"),
        65 | 402 => Some("TIMEOUT"),
        68 | 403 => Some("REFUSE"),
        _ => None,
    }
}

/// `_get_wx_qr`：取二维码页面 → 解析 uuid → 取二维码图片（JPEG），不扫码。
fn wx_qrcode(upstream: &Upstream) -> Result<Value, UpstreamError> {
    let agent = upstream.login_agent();
    let mut page = agent
        .get(&wx_qrcode_page_url())
        .header("User-Agent", BROWSER_UA)
        .call()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    let html = page
        .body_mut()
        .read_to_string()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    if html.is_empty() {
        return Err(UpstreamError::Upstream(
            "获取二维码失败：微信页面是空的".into(),
        ));
    }
    let uuid = parse_wx_uuid(&html)?;

    let mut image = agent
        .get(&wx_qrcode_image_url(&uuid))
        .header("Referer", WX_QRCONNECT)
        .header("User-Agent", BROWSER_UA)
        .call()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    let bytes = image
        .body_mut()
        .read_to_vec()
        .map_err(|error| UpstreamError::Transport(error.to_string()))?;
    if bytes.is_empty() {
        return Err(UpstreamError::Upstream("获取二维码失败：图片是空的".into()));
    }
    Ok(json!({
        "identifier": uuid,
        "loginType": "wx",
        // 参考 `QR(..., "image/jpeg", uuid)`——微信这条给的就是 JPEG。
        "mimetype": "image/jpeg",
        "imageBase64": base64::engine::general_purpose::STANDARD.encode(&bytes),
    }))
}

/// `_check_wx_qr`：长轮询一次状态；`DONE` 时走 `_authorize_wx_qr` 换凭据。
///
/// `DONE` 拿到凭据后**写入组件的凭据存储**（与既有 `poll_login` 一致；参考把
/// credential 交给调用方，组件则由自己持有凭据文件）。
fn wx_qrcode_status(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let identifier = first_text(params, &["identifier", "uuid"])
        .ok_or_else(|| UpstreamError::Upstream("缺少二维码标识 identifier".into()))?;
    let agent = upstream.login_agent();
    // 长轮询通常压满 30 秒左右，`login_agent` 的 20 秒全局超时不够，单次请求放宽到
    // 参考的 35 秒；其它用法不受影响。
    let request = agent
        .get(&wx_status_url(&identifier, now_seconds()))
        .header("Referer", "https://open.weixin.qq.com/")
        .header("User-Agent", BROWSER_UA)
        .config()
        .timeout_global(Some(WX_LONG_POLL_TIMEOUT))
        .build();
    let text = match request.call() {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .map_err(|error| UpstreamError::Transport(error.to_string()))?,
        // 参考把长轮询超时解释成「扫码中」（`TimeoutNetworkError → SCAN`）。
        Err(ureq::Error::Timeout(_)) => return Ok(json!({ "event": "SCAN", "loggedIn": false })),
        Err(error) => return Err(UpstreamError::Transport(error.to_string())),
    };

    let (code, wx_code) = parse_wx_status(&text)?;
    let event = wx_event(code)
        .ok_or_else(|| UpstreamError::Upstream(format!("无法识别的二维码登录状态码：{code}")))?;
    if event != "DONE" {
        return Ok(json!({ "event": event, "loggedIn": false }));
    }
    if wx_code.is_empty() {
        return Err(UpstreamError::Upstream(
            "获取 code 失败：无效的 code".into(),
        ));
    }

    let data = authorize_wx_qr(upstream, credential, platform, &wx_code)?;
    let refreshed = Credential::from_json(&data)
        .ok_or_else(|| UpstreamError::Upstream("登录响应里没有凭据".into()))?;
    store_credential(&refreshed)?;
    Ok(json!({
        "event": "DONE",
        "loggedIn": true,
        "login": status_from_payload(&data, &refreshed),
    }))
}

/// `_authorize_wx_qr`：微信 code 换凭据（`tmeLoginType=1` 照参考）。
fn authorize_wx_qr(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    code: &str,
) -> Result<Value, UpstreamError> {
    let slot = login_raw(
        upstream,
        credential,
        platform,
        &[("tmeLoginType", 1)],
        "Login",
        json!({ "code": code, "strAppid": WX_APP_ID }),
    )?;
    validate_login_result(&slot)
}

// MARK: - 手机验证码

/// 参考 `send_authcode` 的参数：`tmeAppid`（注意小写 d）与 `areaCode` 是字符串。
fn send_authcode_param(params: &Value) -> Result<Value, UpstreamError> {
    let (key, value) = phone_target(params)?;
    let country_code =
        first_int(params, &["countryCode", "country_code"]).unwrap_or(DEFAULT_COUNTRY_CODE);
    let mut param = json!({
        "tmeAppid": "qqmusic",
        "areaCode": country_code.to_string(),
    });
    param[key] = json!(value);
    Ok(param)
}

/// 参考 `phone_authorize` 的参数：`loginMode=1`，号码同样二选一。
fn phone_login_param(params: &Value) -> Result<Value, UpstreamError> {
    let (key, value) = phone_target(params)?;
    let auth_code = first_text(params, &["authCode", "auth_code"])
        .ok_or_else(|| UpstreamError::Upstream("缺少 authCode（短信验证码）".into()))?;
    let mut param = json!({ "code": auth_code, "loginMode": 1 });
    param[key] = json!(value);
    Ok(param)
}

/// 号码二选一，照参考 Web 层 `PhoneTargetRequest` 的判据：
/// `phone` 是**数字**（明文），`encryptedPhone` 是字符串（加密号码），
/// 两者必须且只能给一个。字符串形式的 `phone` 有歧义，拒收并提示走
/// `encryptedPhone`（参考模块层的 `int | str` 重载就是靠这个区分）。
fn phone_target(params: &Value) -> Result<(&'static str, String), UpstreamError> {
    let encrypted = first_text(
        params,
        &["encryptedPhone", "encrypted_phone", "encryptedPhoneNo"],
    );
    let plain = params.get("phone").filter(|value| !value.is_null());
    match (plain, encrypted) {
        (Some(_), Some(_)) => Err(UpstreamError::Upstream(
            "phone 与 encryptedPhone 只能提供一个".into(),
        )),
        (None, None) => Err(UpstreamError::Upstream(
            "缺少 phone 或 encryptedPhone".into(),
        )),
        (Some(Value::Number(number)), None) => Ok(("phoneNo", number.to_string())),
        (Some(_), None) => Err(UpstreamError::Upstream(
            "phone 必须是数字；加密手机号请放在 encryptedPhone".into(),
        )),
        (None, Some(text)) => Ok(("encryptedPhoneNo", text)),
    }
}

/// `send_authcode` 的结果分派（参考按 `resp.code` 三选一，其余报错）。
fn authcode_result(slot: &Value) -> Result<Value, UpstreamError> {
    let code = slot.get("code").and_then(Value::as_i64).unwrap_or(0);
    let data = slot.get("data").cloned().unwrap_or_else(|| json!({}));
    match code {
        PHONE_CODE_CAPTCHA => Ok(json!({
            "event": PHONE_EVENT_CAPTCHA,
            "info": first_text(&data, &["securityURL", "securityUrl"]),
        })),
        PHONE_CODE_FREQUENCY => Ok(json!({ "event": PHONE_EVENT_FREQUENCY })),
        0 => Ok(json!({ "event": PHONE_EVENT_SEND })),
        _ => {
            let detail = first_text(&data, &["errMsg", "errTip", "errtip"]).unwrap_or_default();
            Err(login_failure("发送验证码失败", code, &detail))
        }
    }
}

/// `phone_authorize`：验证码换凭据并落盘（参考成功后写回 client 的凭据）。
fn phone_authorize(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = phone_login_param(params)?;
    let slot = login_raw(
        upstream,
        credential,
        // 参考标了 `platform=Platform.ANDROID`：固定 Android，忽略调用方档案。
        Platform::Android,
        &[("tmeLoginMethod", 3), ("tmeLoginType", 0)],
        "Login",
        param,
    )?;
    let data = validate_login_result(&slot)?;
    let refreshed = Credential::from_json(&data)
        .ok_or_else(|| UpstreamError::Upstream("登录响应里没有凭据".into()))?;
    store_credential(&refreshed)?;
    Ok(json!(status_from_payload(&data, &refreshed)))
}

// MARK: - 凭据刷新

/// 参考 `refresh_credential` 的 `match target.login_type`：1、2 与默认三套参数，
/// 字段逐字照抄（`expired_in` 收的是凭据的 `expired_at`）。
fn refresh_param(credential: &Credential) -> (i64, Value) {
    let login_type = credential_login_type(credential);
    let text = |keys: &[&str]| first_text(&credential.raw, keys).unwrap_or_default();
    let openid = text(&["openid"]);
    let refresh_token = text(&["refresh_token", "refreshToken"]);
    let access_token = text(&["access_token", "accessToken"]);
    let unionid = text(&["unionid"]);
    let refresh_key = text(&["refresh_key", "refreshKey"]);
    let expired_at = first_int(&credential.raw, &["expired_at", "expiredAt"]).unwrap_or(0);
    let musicid = credential.music_id.parse::<i64>().unwrap_or(0);
    let str_musicid = text(&["str_musicid", "strMusicId"]);
    let str_musicid = if str_musicid.is_empty() {
        credential.music_id.clone()
    } else {
        str_musicid
    };
    let param = match login_type {
        1 => json!({
            "openid": openid,
            "refresh_token": refresh_token,
            "str_musicid": str_musicid,
            "musickey": credential.music_key,
            "unionid": unionid,
            "refresh_key": refresh_key,
            "loginMode": 2,
        }),
        2 => json!({
            "openid": openid,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expired_in": expired_at,
            "musicid": musicid,
            "musickey": credential.music_key,
            "refresh_key": refresh_key,
            "loginMode": 2,
        }),
        _ => json!({
            "openid": openid,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expired_in": expired_at,
            "str_musicid": str_musicid,
            "musicid": musicid,
            "musickey": credential.music_key,
            "unionid": unionid,
            "refresh_key": refresh_key,
            "loginMode": 2,
        }),
    };
    (login_type, param)
}

/// 凭据的登录类型：`raw.loginType` 优先；没有就按参考的推断——`musickey` 以
/// `W_X` 开头是微信（1），否则 2；**没有 musickey 时不推断，落回 0**
/// （走参考 `_` 的默认参数分支）。参考的 `login_type` 缺省就是 0。
fn credential_login_type(credential: &Credential) -> i64 {
    first_int(&credential.raw, &["loginType", "login_type"]).unwrap_or_else(|| {
        if credential.music_key.is_empty() {
            0
        } else if credential.music_key.starts_with("W_X") {
            1
        } else {
            2
        }
    })
}

/// `refresh_credential`：刷新当前凭据并写回存储。
fn refresh_current(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
) -> Result<Value, UpstreamError> {
    if !credential.is_usable() {
        return Err(UpstreamError::Upstream("需要登录后才能刷新凭据".into()));
    }
    let (login_type, param) = refresh_param(credential);
    let slot = login_raw(
        upstream,
        credential,
        platform,
        &[("tmeLoginType", login_type)],
        "Login",
        param,
    )?;
    let data = validate_login_result(&slot)?;
    let refreshed = Credential::from_json(&data)
        .ok_or_else(|| UpstreamError::Upstream("刷新响应里没有新的凭据".into()))?;
    store_credential(&refreshed)?;
    Ok(json!(status_from_payload(&data, &refreshed)))
}

// MARK: - 信封与校验（照参考 `_build_cgi` + `_validate_result`）

/// 按平台拼登录调用用的 comm（参考 `build_comm` 与我们自己的 android 档案：
/// 参考的 comm 是"默认档案 + 本次覆盖"，覆盖由 [`login_request`] 最后落上去）。
///
/// Android 档案带设备身份；QIMEI 只取**缓存**，不在登录前触发一次 QIMEI 握手
/// ——登录端点不依赖它（2026-10-03 用一个最小 comm 实测过：服务端照样回真实业务码
/// `104400`/`1000`），而多一次网络就多一个失败模式。
fn login_comm(upstream: &Upstream, credential: &Credential, platform: Platform) -> Value {
    let mut comm = match platform {
        Platform::Web => json!({
            "format": "json",
            "inCharset": "utf-8",
            "outCharset": "utf-8",
            "notice": 0,
            "needNewCode": 1,
            "platform": "yqq.json",
            "ct": 24,
            "cv": 4747474,
            "chid": "0",
        }),
        Platform::Android => json!({
            "format": "json",
            "inCharset": "utf-8",
            "outCharset": "utf-8",
            "notice": 0,
            "needNewCode": 1,
            "platform": "yqq.json",
            "ct": 11,
            "cv": 14090008,
            "v": 14090008,
            "chid": "10003505",
            "tmeAppID": "qqmusic",
        }),
    };
    match platform {
        Platform::Web => {
            comm["uin"] = json!(if credential.music_id.is_empty() {
                "0".to_string()
            } else {
                credential.music_id.clone()
            });
            comm["g_tk"] = json!(credential.g_tk());
        }
        Platform::Android => {
            if credential.is_usable() {
                comm["qq"] = json!(credential.music_id);
                comm["authst"] = json!(credential.music_key);
            }
            // android 档案的默认登录类型；调用方每次都会用 `tmeLoginType` 覆盖它
            // （参考的 `build_comm` 也是从凭据取值后被本次 comm 覆盖）。
            comm["tmeLoginType"] = json!(1);
            let device = upstream.device().load_or_create();
            let open_udid = device.open_udid.clone();
            comm["OpenUDID"] = json!(open_udid.clone());
            comm["OpenUDID2"] = json!(open_udid.clone());
            comm["udid"] = json!(open_udid);
            comm["aid"] = json!(device.android_id.clone());
            comm["os_ver"] = json!(device.os_version_text());
            comm["phonetype"] = json!(device.model.clone());
            comm["devicelevel"] = json!(device.os_sdk.clone());
            comm["newdevicelevel"] = json!(device.os_sdk.clone());
            comm["rom"] = json!(device.proc_version.clone());
            if let Some((q16, q36)) = upstream.device().cached_identity() {
                comm["QIMEI"] = json!(q16);
                comm["QIMEI36"] = json!(q36);
            }
            if let Some((uid, sid)) = device.fresh_session() {
                comm["uid"] = json!(uid);
                comm["sid"] = json!(sid);
            }
        }
    }
    comm
}

/// 拼一次登录调用的请求体与 GET 地址（纯函数，便于测参数拼装）。
///
/// 外层信封与 `Upstream::call_with` 的形状一致，只是走 `get_fcgi` 的 GET 形式；
/// `comm_extra` 是参考每次调用带的覆盖（如 `tmeLoginType`）。
fn login_request(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    comm_extra: &[(&str, i64)],
    method: &str,
    param: Value,
) -> (Value, String) {
    let mut comm = login_comm(upstream, credential, platform);
    for (key, value) in comm_extra {
        comm[*key] = json!(value);
    }
    let body = json!({
        "comm": comm,
        "req_0": {
            "module": LOGIN_MODULE,
            "method": method,
            "param": param,
        },
    });
    let url = format!(
        "{MUSICU_ENDPOINT}?format=json&data={}",
        urlencode(&body.to_string())
    );
    (body, url)
}

/// 发一次登录调用，返回 `req_0` 槽（`{code, data}`）。
fn login_raw(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    comm_extra: &[(&str, i64)],
    method: &str,
    param: Value,
) -> Result<Value, UpstreamError> {
    let (_, url) = login_request(upstream, credential, platform, comm_extra, method, param);
    let envelope = upstream.get_fcgi(credential, Class::Account, &url)?;
    if let Some(code) = envelope.get("code").and_then(Value::as_i64) {
        if code != 0 {
            // 参考 `unwrap_cgi_envelope`：外层 code != 0 是网关拦截。
            return Err(UpstreamError::Upstream(format!(
                "请求被网关拒绝（code={code}）"
            )));
        }
    }
    envelope
        .get("req_0")
        .cloned()
        .ok_or_else(|| UpstreamError::Upstream("响应里没有 req_0".into()))
}

/// 参考 `_validate_result`：按 `req_0.code` 分派登录错误，`0` 时回 `data`
/// （成功时 `data` 就是凭据载荷本身）。
fn validate_login_result(slot: &Value) -> Result<Value, UpstreamError> {
    let code = slot.get("code").and_then(Value::as_i64).unwrap_or(0);
    let data = slot.get("data").cloned().unwrap_or_else(|| json!({}));
    match code {
        0 => Ok(data),
        _ => {
            let detail = first_text(&data, &["errMsg", "errTip", "errtip", "feedbackURL"])
                .unwrap_or_default();
            let message = match code {
                1000 | 104401 | 104400 => "登录鉴权参数无效或已过期",
                20261 => "登录参数错误",
                20271 => "验证码错误",
                20272 => "账号绑定异常",
                20274 => "账号绑定缺失",
                20277 | 20278 => "账号受限",
                20279 => "登录设备超限",
                20450 => "账号已被封禁",
                104604 => "操作过于频繁",
                _ => "登录失败",
            };
            Err(login_failure(message, code, &detail))
        }
    }
}

/// 登录类错误的措辞：带上参考的错误码（错误消息里 `code=` 是参考测试的断言点）。
fn login_failure(message: &str, code: i64, detail: &str) -> UpstreamError {
    if detail.is_empty() {
        UpstreamError::Upstream(format!("{message}（code={code}）"))
    } else {
        UpstreamError::Upstream(format!("{message}（code={code}）：{detail}"))
    }
}

// MARK: - 落盘与状态

/// 把新凭据写进组件自己的凭据文件（合并不是这里做的：`CredentialStore::store`
/// 会保留我们没读的键）。
fn store_credential(credential: &Credential) -> Result<(), UpstreamError> {
    crate::CredentialStore::for_directory(&crate::data_directory())
        .store(credential)
        .map_err(|error| UpstreamError::Upstream(format!("无法保存凭据：{error}")))
}

/// 登录响应载荷 → 组件对外的账号状态（`LoginStatus`，与 `get_login_status` 同形）。
fn status_from_payload(data: &Value, credential: &Credential) -> LoginStatus {
    LoginStatus {
        logged_in: true,
        music_id: credential
            .music_id
            .parse::<i64>()
            .ok()
            .or_else(|| first_int(data, &["musicid", "musicId"])),
        nickname: first_text(data, &["nick", "nickname", "name"]),
        vip_type: first_int(data, &["viptype", "vipType", "vip"]),
        expired: Some(false),
        has_playback_key: Some(!credential.music_key.is_empty()),
    }
}

/// 秒级时间戳（参考的 `int(time())`）。
fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 把一次值编码进 query（与 `upstream.rs` 的私有实现同款，签名路之外这里也用到）。
fn urlencode(value: &str) -> String {
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

// MARK: - 分发

/// 协议分发：只认领 `METHODS` 里的名字，其余返回 `None`。
pub fn dispatch(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    method: &str,
    params: &Value,
) -> Option<Result<Value, UpstreamError>> {
    let result = match method {
        "fetch_wx_qrcode" => wx_qrcode(upstream),
        "check_wx_qrcode" => wx_qrcode_status(upstream, credential, platform, params),
        "send_phone_authcode" => phone_authcode(upstream, credential, params),
        "phone_login" => phone_authorize(upstream, credential, params),
        "refresh_credential" => refresh_current(upstream, credential, platform),
        _ => return None,
    };
    Some(result)
}

/// `send_authcode`：固定 Android 档案、`tmeLoginMethod=3`，按回值 `code` 三选一。
fn phone_authcode(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = send_authcode_param(params)?;
    let slot = login_raw(
        upstream,
        credential,
        Platform::Android,
        &[("tmeLoginMethod", 3)],
        "SendPhoneAuthCode",
        param,
    )?;
    authcode_result(&slot)
}

// MARK: - 宿主包装

/// 号码参数的两种形状，照参考 Web 层的 `phone` / `encrypted_phone`。
fn phone_params(phone: Option<i64>, encrypted_phone: Option<String>) -> Value {
    let mut params = json!({});
    if let Some(phone) = phone {
        params["phone"] = json!(phone);
    }
    if let Some(encrypted_phone) = encrypted_phone {
        params["encryptedPhone"] = json!(encrypted_phone);
    }
    params
}

/// 取微信登录二维码（只取码，不扫码）。`loginType` 是 `"wx"`。
#[export]
pub fn fetch_wx_qrcode() -> Result<LoginQrCode, crate::HelperError> {
    crate::port::call("fetch_wx_qrcode", json!({}))
}

/// 查一次微信扫码状态；返回 `SCAN`/`CONF`/`REFUSE`/`TIMEOUT`，
/// `DONE` 时凭据已写入组件存储，`login` 里的是刚登录的账号。
#[export]
pub fn check_wx_qrcode(identifier: String) -> Result<LoginPoll, crate::HelperError> {
    crate::port::call("check_wx_qrcode", json!({ "identifier": identifier }))
}

/// 发手机验证码（固定 Android）。`phone` 是明文号码，`encrypted_phone` 是加密号码，
/// 两者只能给一个；`country_code` 缺省 86。
#[export]
pub fn send_phone_authcode(
    phone: Option<i64>,
    encrypted_phone: Option<String>,
    country_code: Option<i64>,
) -> Result<PhoneAuthCodeResult, crate::HelperError> {
    let mut params = phone_params(phone, encrypted_phone);
    if let Some(country_code) = country_code {
        params["countryCode"] = json!(country_code);
    }
    crate::port::call("send_phone_authcode", params)
}

/// 验证码登录（固定 Android）；成功后凭据已写入组件存储。
#[export]
pub fn phone_login(
    phone: Option<i64>,
    encrypted_phone: Option<String>,
    auth_code: String,
) -> Result<LoginStatus, crate::HelperError> {
    let mut params = phone_params(phone, encrypted_phone);
    params["authCode"] = json!(auth_code);
    crate::port::call("phone_login", params)
}

/// 刷新当前凭据并写回存储，返回刷新后的账号状态。
#[export]
pub fn refresh_credential() -> Result<LoginStatus, crate::HelperError> {
    crate::port::call("refresh_credential", json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::hash33;
    use crate::device::DeviceStore;

    /// 一个不碰真实数据目录的 Upstream（android 档案会读设备身份）。
    fn test_upstream() -> Upstream {
        Upstream::with_device(DeviceStore::for_directory(
            &std::env::temp_dir().join("qqmusic-helper-next-login-extra-test"),
        ))
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 5);
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_liked_songs",
            &json!({})
        )
        .is_none());
    }

    #[test]
    fn the_wx_page_url_carries_the_reference_parameters() {
        let url = wx_qrcode_page_url();
        assert!(url.starts_with(WX_QRCONNECT));
        assert!(url.contains("appid=wx48db31d50e334801"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=snsapi_login"));
        assert!(url.contains("state=STATE"));
        assert!(
            url.contains("login_type%3D2"),
            "redirect_uri 整体编码进 query：{url}"
        );
        assert!(url.contains("popup_wechat.css%23wechat_redirect"));
        assert_eq!(
            wx_qrcode_image_url("abc123"),
            "https://open.weixin.qq.com/connect/qrcode/abc123"
        );
    }

    #[test]
    fn the_uuid_is_the_first_non_empty_match() {
        let html = "<script>var tool = \"https://long.open.weixin.qq.com/connect/l/qrconnect?\
uuid=091ujJb73BxYll2u\";</script><span>uuid=\"\"</span>";
        assert_eq!(parse_wx_uuid(html).unwrap(), "091ujJb73BxYll2u");
        assert!(parse_wx_uuid("<html>no uuid here</html>").is_err());
        assert!(parse_wx_uuid("uuid=\"\"").is_err(), "空 uuid 不算命中");
        // Python 的 `.` 不跨行：引号必须与 `uuid=` 同一行。
        assert!(parse_wx_uuid("uuid=\n\"091\"").is_err(), "跨行不算命中");
        assert_eq!(
            parse_wx_uuid("uuid=\"\"\n?uuid=ok1\"").unwrap(),
            "ok1",
            "跳过空 uuid，继续找页面里真正的那个"
        );
    }

    #[test]
    fn wx_status_text_parses_into_code_and_wx_code() {
        assert_eq!(
            parse_wx_status("window.wx_errcode=408;window.wx_code=''").unwrap(),
            (408, String::new())
        );
        assert_eq!(
            parse_wx_status("window.wx_errcode=405;window.wx_code='CODE-1'").unwrap(),
            (405, "CODE-1".to_string())
        );
        assert!(parse_wx_status("<html>").is_err());
        assert!(parse_wx_status("window.wx_errcode=abc;window.wx_code=''").is_err());
    }

    #[test]
    fn wx_events_follow_the_reference_table() {
        assert_eq!(wx_event(0), Some("DONE"));
        assert_eq!(wx_event(405), Some("DONE"));
        assert_eq!(wx_event(66), Some("SCAN"));
        assert_eq!(wx_event(408), Some("SCAN"));
        assert_eq!(wx_event(67), Some("CONF"));
        assert_eq!(wx_event(404), Some("CONF"));
        assert_eq!(wx_event(65), Some("TIMEOUT"));
        assert_eq!(wx_event(402), Some("TIMEOUT"));
        assert_eq!(wx_event(68), Some("REFUSE"));
        assert_eq!(wx_event(403), Some("REFUSE"));
        assert_eq!(wx_event(999), None);
    }

    #[test]
    fn phone_target_keeps_plain_and_encrypted_numbers_apart() {
        assert_eq!(
            phone_target(&json!({ "phone": 10_000_000_000_i64 })).unwrap(),
            ("phoneNo", "10000000000".to_string())
        );
        assert_eq!(
            phone_target(&json!({ "encryptedPhone": "AbC123" })).unwrap(),
            ("encryptedPhoneNo", "AbC123".to_string())
        );
        assert!(phone_target(&json!({ "phone": 1, "encryptedPhone": "x" })).is_err());
        assert!(phone_target(&json!({})).is_err());
        assert!(
            phone_target(&json!({ "phone": "10000000000" })).is_err(),
            "字符串手机号有歧义，照参考 Web 层拒收"
        );
    }

    #[test]
    fn the_phone_params_are_spelled_like_the_reference() {
        assert_eq!(
            send_authcode_param(&json!({ "phone": 13_800_138_000_i64 })).unwrap(),
            json!({ "tmeAppid": "qqmusic", "areaCode": "86", "phoneNo": "13800138000" })
        );
        assert_eq!(
            send_authcode_param(&json!({ "phone": 138, "countryCode": 852 })).unwrap(),
            json!({ "tmeAppid": "qqmusic", "areaCode": "852", "phoneNo": "138" })
        );
        assert_eq!(
            phone_login_param(&json!({ "encryptedPhone": "E1", "authCode": "1234" })).unwrap(),
            json!({ "code": "1234", "loginMode": 1, "encryptedPhoneNo": "E1" })
        );
        assert!(
            phone_login_param(&json!({ "phone": 138 })).is_err(),
            "缺验证码"
        );
    }

    #[test]
    fn the_phone_events_are_the_three_the_reference_names() {
        assert_eq!(
            authcode_result(&json!({ "code": 0 })).unwrap(),
            json!({ "event": "SEND" })
        );
        assert_eq!(
            authcode_result(&json!({ "code": 20276, "data": { "securityURL": "https://x" } }))
                .unwrap(),
            json!({ "event": "CAPTCHA", "info": "https://x" })
        );
        assert_eq!(
            authcode_result(&json!({ "code": 100001 })).unwrap(),
            json!({ "event": "FREQUENCY" })
        );
        let error = authcode_result(&json!({ "code": 20261, "data": { "errMsg": "bad" } }))
            .expect_err("其余码是失败");
        assert!(error.to_string().contains("发送验证码失败"));
        assert!(error.to_string().contains("code=20261"));
        assert!(error.to_string().contains("bad"));
    }

    #[test]
    fn login_validation_maps_the_reference_codes() {
        let data = json!({ "musicid": 1, "musickey": "K" });
        assert_eq!(
            validate_login_result(&json!({ "code": 0, "data": data })).unwrap(),
            data
        );
        let cases: [(i64, &str); 13] = [
            (1000, "登录鉴权参数无效或已过期"),
            (104401, "登录鉴权参数无效或已过期"),
            (104400, "登录鉴权参数无效或已过期"),
            (20261, "登录参数错误"),
            (20271, "验证码错误"),
            (20272, "账号绑定异常"),
            (20274, "账号绑定缺失"),
            (20277, "账号受限"),
            (20278, "账号受限"),
            (20279, "登录设备超限"),
            (20450, "账号已被封禁"),
            (104604, "操作过于频繁"),
            (12345, "登录失败"),
        ];
        for (code, wording) in cases {
            let error =
                validate_login_result(&json!({ "code": code, "data": { "errMsg": "detail" } }))
                    .expect_err(&format!("code {code} 要报错"));
            let text = error.to_string();
            assert!(text.contains(wording), "{code}: {text}");
            assert!(text.contains(&format!("code={code}")), "{code}: {text}");
        }
    }

    #[test]
    fn the_wx_authorize_call_is_the_reference_one() {
        let upstream = Upstream::new();
        let (body, url) = login_request(
            &upstream,
            &Credential::default(),
            Platform::Web,
            &[("tmeLoginType", 1)],
            "Login",
            json!({ "code": "C1", "strAppid": WX_APP_ID }),
        );
        assert_eq!(body["req_0"]["module"], LOGIN_MODULE);
        assert_eq!(body["req_0"]["method"], "Login");
        assert_eq!(body["req_0"]["param"]["code"], "C1");
        assert_eq!(body["req_0"]["param"]["strAppid"], "wx48db31d50e334801");
        assert_eq!(body["comm"]["tmeLoginType"], 1);
        assert_eq!(body["comm"]["platform"], "yqq.json");
        assert!(
            url.starts_with("https://u.y.qq.com/cgi-bin/musicu.fcg?format=json&data="),
            "{url}"
        );
        assert!(!url.contains('{'), "信封要整体编码进 data");
    }

    #[test]
    fn the_phone_calls_force_the_android_profile_and_the_reference_comm() {
        let upstream = test_upstream();
        let (body, _) = login_request(
            &upstream,
            &Credential::default(),
            Platform::Android,
            &[("tmeLoginMethod", 3), ("tmeLoginType", 0)],
            "Login",
            json!({ "code": "1", "loginMode": 1 }),
        );
        assert_eq!(body["comm"]["ct"], 11);
        assert_eq!(body["comm"]["cv"], 14090008);
        assert_eq!(body["comm"]["chid"], "10003505");
        assert_eq!(body["comm"]["tmeAppID"], "qqmusic");
        assert_eq!(body["comm"]["tmeLoginMethod"], 3);
        assert_eq!(
            body["comm"]["tmeLoginType"], 0,
            "调用方的 0 覆盖档案默认的 1"
        );
        assert!(body["comm"]["OpenUDID"].is_string(), "android 带设备身份");
        assert!(body["comm"].get("qq").is_none(), "没登录就不带账号身份");
    }

    #[test]
    fn the_web_comm_carries_the_account_and_its_gtk() {
        let upstream = Upstream::new();
        let credential = Credential {
            music_id: "12345".to_string(),
            music_key: "KEY".to_string(),
            ..Default::default()
        };
        let (body, _) = login_request(
            &upstream,
            &credential,
            Platform::Web,
            &[("tmeLoginType", 2)],
            "Login",
            json!({}),
        );
        assert_eq!(body["comm"]["ct"], 24);
        assert_eq!(body["comm"]["uin"], "12345");
        assert_eq!(body["comm"]["g_tk"], json!(hash33("KEY")));
        assert_eq!(body["comm"]["tmeLoginType"], 2);
    }

    #[test]
    fn refresh_uses_the_column_of_the_credentials_login_type() {
        let wechat = Credential {
            music_id: "42".to_string(),
            music_key: "W_X_key".to_string(),
            raw: json!({
                "openid": "o", "refresh_token": "rt", "unionid": "u",
                "refresh_key": "rk", "loginType": 1
            }),
            ..Default::default()
        };
        let (login_type, param) = refresh_param(&wechat);
        assert_eq!(login_type, 1);
        assert_eq!(
            param,
            json!({
                "openid": "o", "refresh_token": "rt", "str_musicid": "42",
                "musickey": "W_X_key", "unionid": "u", "refresh_key": "rk", "loginMode": 2
            })
        );
        assert!(param.get("musicid").is_none(), "type=1 不带数字 musicid");

        let qq = Credential {
            music_id: "42".to_string(),
            music_key: "Q_H".to_string(),
            raw: json!({
                "access_token": "at", "refresh_token": "rt", "expired_at": 99, "loginType": 2
            }),
            ..Default::default()
        };
        let (login_type, param) = refresh_param(&qq);
        assert_eq!(login_type, 2);
        assert_eq!(
            param,
            json!({
                "openid": "", "access_token": "at", "refresh_token": "rt", "expired_in": 99,
                "musicid": 42, "musickey": "Q_H", "refresh_key": "", "loginMode": 2
            })
        );
        assert!(param.get("str_musicid").is_none(), "type=2 不带字符串 id");

        let inferred = Credential {
            music_id: "7".to_string(),
            music_key: "W_X_abc".to_string(),
            ..Default::default()
        };
        assert_eq!(
            refresh_param(&inferred).0,
            1,
            "没有 loginType 时按 musickey 前缀推断"
        );

        let no_key = Credential {
            music_id: "7".to_string(),
            ..Default::default()
        };
        assert_eq!(
            credential_login_type(&no_key),
            0,
            "没有 musickey 时不推断，落回 0（参考的默认分支）"
        );

        let other = Credential {
            music_id: "8".to_string(),
            music_key: "K".to_string(),
            raw: json!({ "loginType": 0, "str_musicid": "008" }),
            ..Default::default()
        };
        let (login_type, param) = refresh_param(&other);
        assert_eq!(login_type, 0, "0 走参考的默认分支");
        assert_eq!(param["str_musicid"], "008");
        assert_eq!(param["musicid"], 8);
        assert_eq!(param["expired_in"], 0);
        assert_eq!(param["loginMode"], 2);
    }

    #[test]
    fn the_wx_result_payloads_fit_the_host_models() {
        let qr: LoginQrCode = serde_json::from_value(json!({
            "identifier": "u1", "loginType": "wx",
            "mimetype": "image/jpeg", "imageBase64": "aGk="
        }))
        .expect("二维码回值");
        assert_eq!(qr.identifier, "u1");
        assert_eq!(qr.login_type, "wx");
        assert_eq!(qr.mimetype, "image/jpeg");

        let poll: LoginPoll = serde_json::from_value(json!({
            "event": "DONE", "loggedIn": true,
            "login": { "loggedIn": true, "musicId": 42, "nickname": "甲" }
        }))
        .expect("状态回值");
        assert_eq!(poll.event, "DONE");
        assert!(poll.logged_in);
        let login = poll.login.expect("登录完成带账号");
        assert_eq!(login.music_id, Some(42));
        assert_eq!(login.nickname.as_deref(), Some("甲"));
    }

    #[test]
    fn the_phone_result_payload_fits_its_model() {
        let result: PhoneAuthCodeResult =
            serde_json::from_value(json!({ "event": "CAPTCHA", "info": "https://x" }))
                .expect("验证码回值");
        assert_eq!(result.event, "CAPTCHA");
        assert_eq!(result.info.as_deref(), Some("https://x"));
        let sent: PhoneAuthCodeResult =
            serde_json::from_value(json!({ "event": "SEND" })).expect("发送回值");
        assert_eq!(sent.info, None);
    }

    #[test]
    fn a_login_status_is_summarized_from_the_login_payload() {
        let credential = Credential {
            music_id: "42".to_string(),
            music_key: "K".to_string(),
            ..Default::default()
        };
        let status = status_from_payload(&json!({ "nick": "甲", "viptype": 2 }), &credential);
        assert!(status.logged_in);
        assert_eq!(status.music_id, Some(42));
        assert_eq!(status.nickname.as_deref(), Some("甲"));
        assert_eq!(status.vip_type, Some(2));
        assert_eq!(status.expired, Some(false));
        assert_eq!(status.has_playback_key, Some(true));
    }
}

#[cfg(test)]
mod live_probes {
    use super::*;

    /// 真机验证 [`wx_qrcode`]（默认忽略，需要网络；不扫码、不需要凭据）：
    /// `cargo test --lib live_wx_qrcode -- --ignored --nocapture`
    ///
    /// 2026-10-03 实测通过：uuid 非空，二维码是 470×470 的 JPEG。
    #[test]
    #[ignore]
    fn live_wx_qrcode() {
        let upstream = Upstream::new();
        let payload = wx_qrcode(&upstream).expect("微信二维码应当取得到");
        let uuid = payload["identifier"].as_str().unwrap_or_default();
        let image = payload["imageBase64"].as_str().unwrap_or_default();
        println!("uuid={uuid} base64_len={}", image.len());
        assert!(!uuid.is_empty());
        assert!(image.starts_with("/9j/"), "JPEG 的 base64 头");
    }

    /// 真机验证微信长轮询的解析（默认忽略）：未扫码时应回 `SCAN`（`408`）。
    /// `cargo test --lib live_wx_status -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_wx_status() {
        let upstream = Upstream::new();
        let qr = wx_qrcode(&upstream).expect("先取码");
        let identifier = qr["identifier"].as_str().unwrap_or_default();
        let status = wx_qrcode_status(
            &upstream,
            &Credential::default(),
            Platform::Web,
            &json!({ "identifier": identifier }),
        )
        .expect("查状态");
        println!("status={status}");
        assert_eq!(status["event"], "SCAN");
    }
}

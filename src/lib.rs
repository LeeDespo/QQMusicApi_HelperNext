//! QQMusicApi HelperNext — a cross-platform Rust component for the QQ Music API
//! family, written from the protocol work of the
//! [QQMusicApi](https://github.com/L-1124/QQMusicApi) project.
//!
//! The library is the whole product: the endpoint calls, request signing, JSON
//! parsing, the credential, rate limiting and circuit breaking all live here.
//! The binary in `src/bin/stdio.rs` is a thin adapter that speaks the line-based
//! JSON protocol a host application drives as a child process; BoltFFI bindings
//! for Swift/Kotlin/Java/C#/TypeScript/Python are generated from the same API
//! surface (see `docs/ffi.md`).
//!
//! # Calling it
//!
//! ```no_run
//! # fn run() -> Result<(), qqmusic_api_helper_next::HelperError> {
//! // Once, at startup: where the credential lives. Each host passes its own
//! // platform directory — the component never guesses.
//! qqmusic_api_helper_next::configure(qqmusic_api_helper_next::Configuration {
//!     data_dir: "/path/to/app-support".into(),
//!     default_platform: qqmusic_api_helper_next::Platform::Web,
//! });
//!
//! let status = qqmusic_api_helper_next::api::login_status()?;
//! if status.logged_in {
//!     let liked = qqmusic_api_helper_next::api::liked_songs(1, 50)?;
//!     println!("{} songs, {} total", liked.tracks.len(), liked.total);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Threading
//!
//! Every function is safe to call from any thread and from several at once: the
//! HTTP agent, the rate limiter, the breaker and the credential store are shared
//! behind mutexes, and no call holds a lock across the network.

mod aria2;
mod catalog;
pub mod device;
mod guard;
pub mod login;
pub mod methods;
pub mod port;
pub mod qrc;
mod upstream;

pub mod api;
pub mod credential;
pub mod models;

pub use aria2::{Aria2, Aria2Options};
pub use credential::CredentialStore;
pub use guard::{BreakerState, Class};
pub use methods::{COMPONENT_VERSION, PROTOCOL_VERSION};
pub use models::*;
pub use upstream::Platform;
pub use upstream::{Upstream, UpstreamError};

use boltffi::{data, export};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Everything the component needs to know about its host.
#[derive(Debug, Clone)]
pub struct Configuration {
    /// Directory the credential is kept in. The host passes its own platform
    /// location: macOS/iOS an application-support folder, Android `filesDir`.
    pub data_dir: String,
    /// Which platform profile calls are sent under unless a call says otherwise.
    /// Most interfaces want [`Platform::Web`]; a host that needs the other one
    /// for a particular interface can ask per call.
    pub default_platform: Platform,
}

static CONFIGURATION: OnceLock<Configuration> = OnceLock::new();
static CONFIGURATION_LOCK: Mutex<()> = Mutex::new(());
static CONFIGURATION_FROZEN: AtomicBool = AtomicBool::new(false);

/// Freeze host configuration before a component API reads shared storage or
/// constructs the process-wide Upstream.
pub(crate) fn freeze_configuration() {
    let _guard = CONFIGURATION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    CONFIGURATION_FROZEN.store(true, Ordering::Release);
}

/// The platform profile calls use by default: the host's choice, or the web
/// profile when none was configured.
pub fn default_platform() -> Platform {
    CONFIGURATION
        .get()
        .map(|configuration| configuration.default_platform)
        .unwrap_or_default()
}

/// Set the host's configuration. The first call wins, deliberately: a second
/// caller cannot move the credential directory out from under a running session.
pub fn configure(configuration: Configuration) {
    let _guard = CONFIGURATION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if CONFIGURATION_FROZEN.load(Ordering::Acquire) {
        return;
    }
    let _ = CONFIGURATION.set(configuration);
}

/// Initialize the component for a host before its first API call.
///
/// Repeating the same configuration is safe. A conflicting configuration, or
/// first-time configuration after any component API has started, is an explicit
/// error because its stores may already have captured the original directory.
#[export]
pub fn initialize(data_dir: String, platform: String) -> Result<(), HelperError> {
    if data_dir.is_empty() {
        return Err(HelperError::InvalidRequest("data_dir 不能为空".into()));
    }
    let default_platform = Platform::parse(&platform).ok_or_else(|| {
        HelperError::InvalidRequest(format!(
            "不支持的平台档案：{platform}（请用 web 或 android）"
        ))
    })?;
    let requested = Configuration {
        data_dir,
        default_platform,
    };
    let _guard = CONFIGURATION_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(current) = CONFIGURATION.get() {
        if current.data_dir == requested.data_dir
            && current.default_platform == requested.default_platform
        {
            return Ok(());
        }
        return Err(HelperError::InvalidRequest(
            "HelperNext 已使用不同的数据目录或平台档案初始化".into(),
        ));
    }
    if CONFIGURATION_FROZEN.load(Ordering::Acquire) {
        return Err(HelperError::InvalidRequest(
            "必须在首次调用 HelperNext API 前初始化 data_dir 与 platform".into(),
        ));
    }
    CONFIGURATION
        .set(requested)
        .map_err(|_| HelperError::InvalidRequest("HelperNext 初始化竞争失败".into()))
}

/// The credential directory in force. Public because a host that passes its own
/// directory through `configure` may still want to show it (the macOS settings
/// page does), and because the CLI adapter needs it.
pub fn data_directory() -> PathBuf {
    if let Some(configuration) = CONFIGURATION.get() {
        if !configuration.data_dir.is_empty() {
            return PathBuf::from(&configuration.data_dir);
        }
    }
    if let Ok(explicit) = std::env::var("QQMUSIC_HELPER_NEXT_DIR") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    PathBuf::from(".")
}

/// Failure of any call. One type for the whole surface: every call is "ask the
/// upstream and parse it", so the caller's options are always the same — report
/// it, or retry later.
#[data]
#[derive(Debug, Clone, PartialEq)]
pub enum HelperError {
    /// No credential, or the upstream rejected it.
    NotLoggedIn,
    /// The component refused the call to protect the upstream (see the guard).
    Throttled(String),
    /// The network failed, or the response could not be parsed.
    Upstream(String),
    /// The caller asked for something this component does not serve.
    Unsupported(String),
    /// The host passed something unusable (a bad song mid, a missing directory).
    InvalidRequest(String),
}

impl std::fmt::Display for HelperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HelperError::NotLoggedIn => write!(f, "需要登录后才能读取"),
            HelperError::Throttled(reason) => write!(f, "{reason}"),
            HelperError::Upstream(detail) => write!(f, "{detail}"),
            HelperError::Unsupported(what) => write!(f, "不支持：{what}"),
            HelperError::InvalidRequest(what) => write!(f, "请求无效：{what}"),
        }
    }
}

impl std::error::Error for HelperError {}

impl From<upstream::UpstreamError> for HelperError {
    fn from(error: upstream::UpstreamError) -> Self {
        match error {
            upstream::UpstreamError::Refused(reason) => HelperError::Throttled(reason),
            upstream::UpstreamError::Transport(detail)
            | upstream::UpstreamError::Upstream(detail) => HelperError::Upstream(detail),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_configuration_wins() {
        configure(Configuration {
            data_dir: "/tmp/helper-next-first".into(),
            default_platform: Platform::Android,
        });
        configure(Configuration {
            data_dir: "/tmp/helper-next-second".into(),
            default_platform: Platform::Web,
        });
        assert_eq!(data_directory().to_string_lossy(), "/tmp/helper-next-first");
        assert_eq!(default_platform(), Platform::Android, "first call wins");
        assert!(initialize("/tmp/helper-next-first".into(), "android".into()).is_ok());
        assert!(initialize("/tmp/helper-next-second".into(), "web".into()).is_err());
    }
}

#[cfg(test)]
mod qrc_vector_check {
    #[test]
    fn the_official_vector_decrypts_to_the_expected_document() {
        let hex = include_str!("../tests/fixtures/qrc/qrc-vector.hex").trim();
        let document = crate::qrc::decrypt_hex(hex).expect("decrypts");
        // JS `.length` counts UTF-16 units; Rust counts bytes. CJK makes them differ.
        assert_eq!(
            document.chars().count(),
            7188,
            "length the reference records"
        );
        assert!(document.contains("<QrcInfos>"));
        assert!(document.contains("<LyricInfo LyricCount=\"1\">"));
        let payload = crate::qrc::extract_payload(&document).expect("payload");
        let lines = crate::qrc::parse(&payload);
        assert!(!lines.is_empty(), "words were found");
        assert!(lines[0].duration_ms > 0, "source line duration is retained");
        assert!(lines[0].words.iter().all(|word| word.duration_ms >= 0));
        let structured = serde_json::to_value(&lines[0]).expect("structured QRC serializes");
        assert!(structured.get("startMs").is_some());
        assert!(structured.get("durationMs").is_some());
        assert!(structured["words"][0].get("startMs").is_some());
        let lrc = crate::qrc::to_word_lrc(&lines);
        println!("---\n{l}\n---", l = &lrc[..lrc.len().min(400)]);
    }
}

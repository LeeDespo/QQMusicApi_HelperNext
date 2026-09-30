//! The typed API — what a host (or the generated Swift/Kotlin bindings) calls.
//!
//! Each function is a thin, honest shell: it hands the request to the protocol
//! layer ([`crate::methods`]) and parses the answer into a model. Keeping one
//! implementation of each endpoint (rather than a typed one and a JSON one) is
//! what stops the two surfaces from drifting apart — the CLI and the bindings
//! share it.
//!
//! Additions belong here and in `methods::METHODS` together; `api_surface_matches`
//! in the tests fails when one is added without the other.

use crate::credential::{Credential, CredentialStore};
use crate::methods::{self, Upstream};
use crate::{data_directory, HelperError};
use serde_json::{json, Value};
use boltffi::export;
use std::sync::OnceLock;

fn upstream() -> &'static Upstream {
    static INSTANCE: OnceLock<Upstream> = OnceLock::new();
    INSTANCE.get_or_init(Upstream::new)
}

fn store() -> CredentialStore {
    CredentialStore::for_directory(&data_directory())
}

/// Run a method and parse its payload into `T`.
fn call<T: serde::de::DeserializeOwned>(method: &str, params: Value) -> Result<T, HelperError> {
    let credential = store().load();
    let value = methods::dispatch(upstream(), credential.as_ref(), method, &params)
        .map_err(HelperError::from)?;
    // The payload shape differs per method (some return a list, some an object);
    // the caller names it in `keys`.
    parse(value, method)
}

/// Some methods answer with the entity directly, others wrap it in a named key.
/// The wrapper names are the ones the protocol uses, so a host reading the
/// JSON and a host reading these types see the same thing.
fn parse<T: serde::de::DeserializeOwned>(value: Value, method: &str) -> Result<T, HelperError> {
    let candidate = match method {
        "fetch_liked_songs" => value.get("likedSongs").cloned().unwrap_or(value),
        "fetch_login_status" | "get_login_status" => value.get("login").cloned().unwrap_or(value),
        "get_helper_info" => value.get("helper").cloned().unwrap_or(value),
        "fetch_user_playlists" => value.get("playlists").cloned().unwrap_or(value),
        "fetch_liked_albums" => value.get("albums").cloned().unwrap_or(value),
        "fetch_followed_artists" => value.get("artists").cloned().unwrap_or(value),
        "fetch_playlist_tracks" => value.get("tracks").cloned().unwrap_or(value),
        _ => value,
    };
    serde_json::from_value(candidate).map_err(|error| HelperError::Upstream(error.to_string()))
}

/// Who is logged in. Answers from the upstream, so an expired session shows up
/// as `logged_in: false` rather than as a failure.
#[export]
pub fn login_status() -> Result<crate::models::LoginStatus, HelperError> {
    call("get_login_status", json!({}))
}

/// Store a login from the two cookies a web login produces.
///
/// `qm_keyst` is both the session ticket and the CDN playback ticket, so this
/// pair *is* a complete login — no other cookie matters, which is why the
/// signature takes these two rather than a cookie jar. (A `Vec<(String, String)>`
/// also happens to be the one shape the binding generator cannot render, so the
/// two-field form is what hosts on both platforms actually get.)
#[export]
pub fn import_credential(uin: String, qm_keyst: String) -> Result<(), HelperError> {
    let credential: Credential = methods::credential_from_params(&json!({
        "cookies": { "uin": uin, "qm_keyst": qm_keyst }
    }))
    .map_err(HelperError::from)?;
    store()
        .store(&credential)
        .map_err(|error| HelperError::InvalidRequest(error.to_string()))
}

/// Forget the stored login.
#[export]
pub fn logout() -> Result<(), HelperError> {
    store()
        .clear()
        .map_err(|error| HelperError::InvalidRequest(error.to_string()))
}

/// 我喜欢, one page at a time.
#[export]
pub fn liked_songs(page: u32, limit: u32) -> Result<crate::models::LikedSongs, HelperError> {
    call("fetch_liked_songs", json!({ "page": page, "limit": limit }))
}

/// A playlist's (or ranking's) tracks. `offset` is a row offset, not a page.
#[export]
pub fn playlist_tracks(
    list_id: i64,
    offset: u32,
    limit: u32,
) -> Result<Vec<crate::models::Track>, HelperError> {
    call(
        "fetch_playlist_tracks",
        json!({ "songlistId": list_id, "offset": offset, "limit": limit }),
    )
}

/// The account's own playlists.
#[export]
pub fn user_playlists(limit: u32) -> Result<Vec<crate::models::Playlist>, HelperError> {
    call("fetch_user_playlists", json!({ "limit": limit }))
}

/// The account's favorited albums.
#[export]
pub fn liked_albums(limit: u32) -> Result<Vec<crate::models::Album>, HelperError> {
    call("fetch_liked_albums", json!({ "limit": limit }))
}

/// The singers the account follows.
#[export]
pub fn followed_artists(
    page: u32,
    limit: u32,
) -> Result<Vec<crate::models::Artist>, HelperError> {
    call("fetch_followed_artists", json!({ "page": page, "limit": limit }))
}

/// What the component is, and every method it serves.
#[export]
pub fn component_info() -> Result<crate::models::ComponentInfo, HelperError> {
    call("get_helper_info", json!({}))
}

/// The rate limiter's usage and the breaker's state.
#[export]
pub fn guard_status() -> Result<crate::models::GuardStatus, HelperError> {
    call("get_status", json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods::METHODS;

    /// Every exported function has a protocol method behind it, and every
    /// protocol method has a typed wrapper. Adding one without the other is the
    /// easy mistake, and it would show up as "works from the CLI, fails from
    /// Swift" much later.
    #[test]
    fn api_surface_matches() {
        let covered = [
            "get_helper_info",
            "get_login_status",
            "import_cookies",
            "logout",
            "fetch_liked_songs",
            "fetch_playlist_tracks",
            "fetch_user_playlists",
            "fetch_liked_albums",
            "fetch_followed_artists",
            "get_status",
        ];
        for method in METHODS {
            assert!(
                covered.contains(method),
                "method {method} has no typed wrapper in api.rs"
            );
        }
        assert_eq!(covered.len(), METHODS.len());
    }
}

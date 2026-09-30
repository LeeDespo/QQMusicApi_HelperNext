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

    /// Every protocol method has a typed wrapper in this file.
    ///
    /// Adding an endpoint without its wrapper is the easy mistake, and it would
    /// surface as "works from the CLI, missing from the bindings" much later.
    #[test]
    fn api_surface_matches() {
        let source = include_str!("api.rs");
        // `import_cookies` is the protocol name; the typed wrapper is
        // `import_credential` (the two-field form the generator can render).
        // The protocol names and the typed wrappers differ where the wrapper
        // reads better (`get_helper_info` → `component_info`).
        let aliases = [
            ("import_cookies", "import_credential"),
            ("get_helper_info", "component_info"),
            ("get_status", "guard_status"),
            ("get_login_status", "login_status"),
            ("fetch_liked_songs", "liked_songs"),
            ("fetch_playlist_tracks", "playlist_tracks"),
            ("fetch_user_playlists", "user_playlists"),
            ("fetch_liked_albums", "liked_albums"),
            ("fetch_followed_artists", "followed_artists"),
            ("fetch_song_detail", "song_detail"),
            ("fetch_album_detail", "album_detail"),
            ("fetch_album_tracks", "album_tracks"),
            ("fetch_artist_songs", "artist_songs"),
            ("fetch_artist_albums", "artist_albums"),
            ("fetch_artist_detail", "artist_detail"),
            ("fetch_toplist_categories", "toplist_categories"),
            ("fetch_toplist_tracks", "toplist_tracks"),
            ("fetch_radio_stations", "radio_stations"),
            ("fetch_radio_tracks", "radio_tracks"),
            ("fetch_new_songs", "new_songs"),
            ("fetch_recommend_feed", "recommend_feed"),
            ("fetch_lyric", "lyric"),
            ("resolve_song_url", "resolve_song_url"),
        ];
        for method in METHODS {
            // Wrappers name the method they call, so the method string appearing
            // in a `call("…")` (or a matching arm) is the check.
            let wrapper = aliases
                .iter()
                .find(|(protocol, _)| protocol == method)
                .map(|(_, wrapper)| *wrapper)
                .unwrap_or(*method);
            assert!(
                source.contains(&format!("pub fn {wrapper}")),
                "method {method} has no typed wrapper ({wrapper}) in api.rs"
            );
        }
    }
}

// MARK: - Catalogue prose, an artist's works, an album's tracks

/// A song's catalogue entry, including its 简介 (empty when it has none).
#[export]
pub fn song_detail(song_mid: String) -> Result<crate::models::SongDetail, HelperError> {
    call("fetch_song_detail", json!({ "songMid": song_mid }))
}

/// An album's catalogue entry. Address it by mid or by its numeric id.
#[export]
pub fn album_detail(
    album_mid: Option<String>,
    album_id: Option<i64>,
) -> Result<crate::models::AlbumDetail, HelperError> {
    call("fetch_album_detail", json!({ "albumMid": album_mid, "albumId": album_id }))
}

/// An album's tracks.
#[export]
pub fn album_tracks(
    album_mid: Option<String>,
    album_id: Option<i64>,
    offset: i64,
    limit: i64,
) -> Result<Vec<crate::models::Track>, HelperError> {
    call(
        "fetch_album_tracks",
        json!({ "albumMid": album_mid, "albumId": album_id, "offset": offset, "limit": limit }),
    )
}

/// An artist's songs. `sort` is `hot` or `latest` ("最新" is computed locally —
/// the upstream ignores its ordering parameter).
#[export]
pub fn artist_songs(
    singer_mid: String,
    sort: String,
    page: i64,
    limit: i64,
) -> Result<Vec<crate::models::Track>, HelperError> {
    call("fetch_artist_songs", json!({ "singerMid": singer_mid, "sort": sort, "page": page, "limit": limit }))
}

/// An artist's albums, same two sorts.
#[export]
pub fn artist_albums(
    singer_mid: String,
    sort: String,
    page: i64,
    limit: i64,
) -> Result<Vec<crate::models::Album>, HelperError> {
    call("fetch_artist_albums", json!({ "singerMid": singer_mid, "sort": sort, "page": page, "limit": limit }))
}

/// An artist's profile and biography.
#[export]
pub fn artist_detail(singer_mid: String) -> Result<crate::models::ArtistDetail, HelperError> {
    call("fetch_artist_detail", json!({ "singerMid": singer_mid }))
}

// MARK: - Rankings, radio, new songs, recommendations

/// The ranking groups, each with the rankings it contains.
#[export]
pub fn toplist_categories() -> Result<Vec<crate::models::ToplistGroup>, HelperError> {
    call("fetch_toplist_categories", json!({}))
}

/// One ranking's tracks.
#[export]
pub fn toplist_tracks(
    top_id: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<crate::models::Track>, HelperError> {
    call("fetch_toplist_tracks", json!({ "topId": top_id, "offset": offset, "limit": limit }))
}

/// The radio groups, each with its stations.
#[export]
pub fn radio_stations() -> Result<Vec<crate::models::RadioGroup>, HelperError> {
    call("fetch_radio_stations", json!({}))
}

/// A station's next songs (a fresh rotation on every call).
#[export]
pub fn radio_tracks(
    station_id: i64,
    limit: i64,
    first_play: bool,
) -> Result<Vec<crate::models::Track>, HelperError> {
    call("fetch_radio_tracks", json!({ "stationId": station_id, "limit": limit, "firstPlay": first_play }))
}

/// New songs for one region: 0 最新, 1 内地, 2 港台, 3 欧美, 4 日本, 5 韩国.
#[export]
pub fn new_songs(region_type: i64) -> Result<Vec<crate::models::Track>, HelperError> {
    call("fetch_new_songs", json!({ "regionType": region_type }))
}

/// "Guess you like" — five fresh tracks per call.
#[export]
pub fn recommend_feed() -> Result<Vec<crate::models::Track>, HelperError> {
    call("fetch_recommend_feed", json!({}))
}

// MARK: - Lyrics and playback urls

/// A song's lyric. `word_timing` asks for the word-level track; `translation`
/// asks for the translation and romanisation when the service has them.
#[export]
pub fn lyric(
    song_mid: String,
    song_id: Option<i64>,
    word_timing: bool,
    translation: bool,
) -> Result<crate::models::Lyric, HelperError> {
    call(
        "fetch_lyric",
        json!({
            "songMid": song_mid,
            "songId": song_id,
            "wordTiming": word_timing,
            "translation": translation,
        }),
    )
}

/// A playable url for one track, probing the quality ladder best-first.
///
/// `preferred_quality` narrows the probe to one rung; leaving it unset walks the
/// ladder and returns the first grant. A track the account may not play comes
/// back with `playable: false` and the reason — not as an error.
#[export]
pub fn resolve_song_url(
    song_mid: String,
    media_mid: Option<String>,
    song_type: i64,
    preferred_quality: Option<String>,
) -> Result<crate::models::StreamResolution, HelperError> {
    call(
        "resolve_song_url",
        json!({
            "songMid": song_mid,
            "mediaMid": media_mid,
            "songType": song_type,
            "quality": preferred_quality,
        }),
    )
}

// MARK: - Choosing the platform profile per call

/// Send a request under an explicit platform profile.
///
/// The typed wrappers above use the host's configured default; this is the
/// documented way to reach the other profile for a single interface without
/// rebuilding. `params_json` is the same object the protocol layer takes, and
/// the result is the raw payload as JSON.
#[export]
pub fn call_with_platform(
    method: String,
    params_json: String,
    platform: String,
) -> Result<String, HelperError> {
    let parsed: Value = serde_json::from_str(&params_json)
        .map_err(|error| HelperError::InvalidRequest(error.to_string()))?;
    let profile = crate::Platform::parse(&platform)
        .ok_or_else(|| HelperError::InvalidRequest(format!("未知的平台档案：{platform}")))?;
    let mut object = match parsed {
        Value::Object(map) => map,
        _ => return Err(HelperError::InvalidRequest("params 必须是 JSON 对象".into())),
    };
    object.insert("platform".into(), Value::String(profile.as_str().into()));
    let value = call::<Value>(&method, Value::Object(object))?;
    serde_json::to_string(&value).map_err(|error| HelperError::Upstream(error.to_string()))
}

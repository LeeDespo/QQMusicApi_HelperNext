//! The values the component hands back.
//!
//! Every type is plain data with `serde` derives, which is what the language
//! binding generator needs as well: `#[data]` in BoltFFI maps structs with
//! primitive/String/Vec/Option fields directly, and these have nothing else.
//! Field names are camelCase to match the JSON the CLI adapter emits, so one
//! definition serves both surfaces.

use serde::{Deserialize, Serialize};

// `#[data]` is BoltFFI's marker for a type it can pass across the boundary: all
// of these are primitives, Strings, Options and Vecs, which is what it needs.
// The derive list stays because the same structs are the CLI adapter's models.
use boltffi::data;

/// What the component is enforcing after a `set_rate_limit`.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitConfigModel {
    pub enabled: bool,
    pub window_seconds: i64,
    pub max_requests: i64,
}

/// What the component is enforcing after a `set_breaker`.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BreakerConfigModel {
    pub enabled: bool,
    pub failure_threshold: i64,
    pub failure_window_seconds: i64,
    pub open_seconds: i64,
}

/// What the download engine is doing.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aria2Status {
    pub installed: bool,
    pub running: bool,
    pub binary: String,
    pub port: i64,
    pub version: Option<String>,
    pub active: i64,
    pub downloads: i64,
    pub waiting: i64,
    pub stopped: i64,
    pub download_speed: i64,
    pub options: Aria2OptionsModel,
}

/// The download engine's tunables, as the settings page shows them.
#[data]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aria2OptionsModel {
    pub split: i64,
    pub max_connection_per_server: i64,
    pub max_concurrent_downloads: i64,
    pub min_split_size_mib: i64,
    pub max_overall_download_limit_kib: i64,
    /// The RPC port. Applied when the engine next starts.
    pub port: i64,
}

/// One queued file: the gid from `aria2_add`, or its live state from `aria2_tell`.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aria2Download {
    pub gid: String,
    pub status: String,
    pub completed: i64,
    pub total: i64,
    pub speed: i64,
    pub path: String,
    pub error: String,
    pub error_code: i64,
}

/// The engine's task list, as the toolbar's download list shows it.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aria2TaskList {
    pub downloads: Vec<Aria2Task>,
    pub removed: Vec<String>,
}

/// One task.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aria2Task {
    pub gid: String,
    /// `active` / `waiting` / `paused` / `complete` / `error` / `removed`, in
    /// aria2's own vocabulary.
    pub status: String,
    pub completed: i64,
    pub total: i64,
    pub speed: i64,
    pub name: String,
    pub path: String,
    pub error: String,
}

/// A page of tracks, with the list's own size when the endpoint reports it.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    /// `None` when the endpoint reports no total, which is not the same as zero.
    pub total: Option<i64>,
}

/// One credited singer of a track.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Singer {
    pub mid: Option<String>,
    pub name: Option<String>,
}

/// A song, as the catalogue describes it.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub song_id: Option<i64>,
    /// The song's mid — the handle every other call takes.
    pub song_mid: String,
    pub media_mid: Option<String>,
    pub title: String,
    /// All credited singers, joined with `", "`.
    pub artist: String,
    pub album: Option<String>,
    pub album_mid: Option<String>,
    /// The numeric album id, which is what the album endpoints address.
    pub album_id: Option<i64>,
    /// The wire spells this `imageURL`, which `rename_all` alone would mangle.
    #[serde(rename = "imageURL")]
    pub image_url: Option<String>,
    pub duration: Option<i64>,
    /// `1` marks a track the account likely cannot play (subscription).
    pub pay_play: Option<i64>,
    pub singer_mid: Option<String>,
    pub singers: Option<Vec<Singer>>,
    /// Present when the source supplied it (artist "latest" ordering attaches it).
    pub release_date: Option<String>,
}

/// The account's "我喜欢", with the folder's own total.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LikedSongs {
    pub title: String,
    pub total: i64,
    pub tracks: Vec<Track>,
}

/// An album.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub album_mid: Option<String>,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
    pub artist: Option<String>,
    pub release_date: Option<String>,
}

/// A playlist (the account's own, or one found by search).
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub title: String,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
    pub creator: Option<String>,
    pub song_count: Option<i64>,
    pub play_count: Option<i64>,
}

/// A singer.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub singer_mid: String,
    pub name: String,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
    pub song_count: Option<i64>,
    pub album_count: Option<i64>,
    pub fan_count: Option<i64>,
}

/// Who is logged in, as the upstream sees it.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoginStatus {
    pub logged_in: bool,
    pub music_id: Option<i64>,
    pub nickname: Option<String>,
    pub vip_type: Option<i64>,
    pub expired: Option<bool>,
    /// Whether the playback ticket (`qm_keyst`) is present. Without it the CDN
    /// refuses even tracks the account may play.
    pub has_playback_key: Option<bool>,
}

impl LoginStatus {
    pub fn is_vip(&self) -> bool {
        self.vip_type.unwrap_or(0) > 0
    }
}

/// What this component is and what it serves.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentInfo {
    pub helper_version: String,
    pub protocol_version: i64,
    /// Kept for hosts that display a "library version"; this component replaces
    /// the library, so it names the protocol work it is built from.
    pub library_version: String,
    pub methods: Vec<String>,
}

/// The state of the two politeness mechanisms, for the host to display.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardStatus {
    /// `"closed"`, `"half-open"` or `"open"`.
    pub breaker: String,
    pub rate_limit: RateLimitUsage,
}

#[data]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitUsage {
    pub read: u32,
    pub interactive: u32,
    pub playback: u32,
    pub account: u32,
    pub write: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CLI adapter and the typed API must agree on names; this is the test
    /// that fails if one of them is renamed alone.
    #[test]
    fn models_round_trip_the_camel_case_wire_shape() {
        let track: Track = serde_json::from_value(serde_json::json!({
            "songId": 12,
            "songMid": "mid",
            "title": "标题",
            "artist": "甲, 乙",
            "albumId": 4321,
            "imageURL": "https://y.gtimg.cn/a.jpg",
            "payPlay": 0,
            "singers": [{"mid": "a", "name": "甲"}]
        }))
        .expect("decodes");
        assert_eq!(track.song_mid, "mid");
        assert_eq!(track.album_id, Some(4321));
        assert_eq!(track.image_url.as_deref(), Some("https://y.gtimg.cn/a.jpg"));
        assert_eq!(track.singers.as_ref().map(Vec::len), Some(1));
    }
}

/// A song's catalogue entry: the facts plus the prose the service publishes.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongDetail {
    pub song_mid: String,
    pub song_id: Option<i64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_mid: Option<String>,
    /// The 简介. Empty for most songs, which is an answer and not a failure.
    pub description: String,
    #[serde(alias = "genreTags")]
    pub genre: Vec<String>,
    pub language: Option<String>,
    pub company: Option<String>,
    pub release_date: Option<String>,
    pub duration: Option<i64>,
}

/// An album's catalogue entry.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetail {
    pub id: Option<i64>,
    pub album_mid: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub release_date: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub company: Option<String>,
    pub song_count: Option<i64>,
}

/// An artist's profile.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtistDetail {
    pub singer_mid: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
    pub foreign_name: Option<String>,
    pub region: Option<String>,
    pub genre: Vec<String>,
    pub song_count: Option<i64>,
    pub album_count: Option<i64>,
    pub fan_count: Option<i64>,
}

/// One ranking inside a group.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Toplist {
    pub id: i64,
    pub title: String,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
    pub update_time: Option<String>,
}

#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToplistGroup {
    pub title: String,
    pub toplists: Vec<Toplist>,
}

#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadioStation {
    pub id: i64,
    pub title: String,
    #[serde(rename = "coverURL")]
    pub cover_url: Option<String>,
}

#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadioGroup {
    pub title: String,
    pub stations: Vec<RadioStation>,
}

/// A lyric and the extra tracks the service offers with it. `word_lyric` is the
/// word-level one (the reason the helper channel ever existed).
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Lyric {
    pub lyric: Option<String>,
    pub translation: Option<String>,
    pub romanization: Option<String>,
    pub word_lyric: Option<String>,
}

/// A resolved playback url, or why there is none.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamResolution {
    pub song_mid: String,
    /// `flac` / `320` / `128` / `aac` when `playable`.
    pub quality: Option<String>,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub playable: bool,
    /// Present when nothing was granted: what each quality answered.
    pub reason: Option<String>,
}

/// A login QR code: what to draw, and what to poll it with.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoginQrCode {
    /// The `qrsig`: hand it back to `poll_login`.
    pub identifier: String,
    /// `"qq"` for the scan-with-QQ flow.
    pub login_type: String,
    pub mimetype: String,
    /// A PNG, base64-encoded — the shape a host can draw directly.
    pub image_base64: String,
}

/// One poll of a login QR code.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoginPoll {
    /// `SCAN` (not scanned yet), `CONF` (scanned, awaiting confirmation),
    /// `DONE`, `TIMEOUT` or `REFUSE`.
    pub event: String,
    pub logged_in: bool,
    /// Present once `logged_in`: who just logged in.
    pub login: Option<LoginStatus>,
}

/// A search page: the rows plus the catalogue's own total for the query.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackSearch {
    pub total: i64,
    pub tracks: Vec<Track>,
}

#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSearch {
    pub total: i64,
    pub artists: Vec<Artist>,
}

#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSearch {
    pub total: i64,
    pub albums: Vec<Album>,
}

#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSearch {
    pub total: i64,
    pub playlists: Vec<Playlist>,
}

/// One candidate the catalogue offers for a local item's cover.
///
/// The same shape serves a track, an artist and an album — the fields that do
/// not apply to a kind are simply absent, which is also how the app reads them.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkCandidate {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub artist_name: Option<String>,
    pub singer_mid: Option<String>,
    pub song_mid: Option<String>,
    pub album_mid: Option<String>,
    #[serde(rename = "imageURL")]
    pub image_url: Option<String>,
    pub duration: Option<i64>,
    pub release_date: Option<String>,
    /// A rank hint, not a verdict: the host scores the candidates itself.
    pub confidence: Option<f64>,
}

/// An artist's biography, as the local library's enrichment reads it.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtistBiography {
    pub artist_name: Option<String>,
    pub singer_mid: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "imageURL")]
    pub image_url: Option<String>,
    pub region: Option<String>,
    pub foreign_name: Option<String>,
    pub confidence: Option<f64>,
}

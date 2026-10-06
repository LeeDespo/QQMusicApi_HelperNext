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
use boltffi::export;
use serde_json::{json, Value};
use std::sync::OnceLock;

fn upstream() -> &'static Upstream {
    static INSTANCE: OnceLock<Upstream> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        crate::freeze_configuration();
        Upstream::new()
    })
}

/// The same shared upstream, for the port modules' typed wrappers.
///
/// The port layer adds protocol methods; it must not add a second HTTP agent,
/// limiter or breaker, so its wrappers go through this one instance.
pub(crate) fn shared_upstream() -> &'static Upstream {
    upstream()
}

fn store() -> CredentialStore {
    crate::freeze_configuration();
    CredentialStore::for_directory(&data_directory())
}

/// Run a method and parse its payload into `T`.
fn call<T: serde::de::DeserializeOwned>(method: &str, params: Value) -> Result<T, HelperError> {
    parse(dispatch_payload(method, &params)?, method)
}

/// Keep the protocol envelope intact for callers requesting raw JSON.
fn dispatch_payload(method: &str, params: &Value) -> Result<Value, HelperError> {
    let upstream = upstream();
    let credential = store().load();
    methods::dispatch(upstream, credential.as_ref(), method, params).map_err(HelperError::from)
}

/// Adapt the protocol's explicit envelopes to the public typed return values.
/// Page models keep their whole payload so their totals survive. An absent
/// envelope is an error, including for models containing only optional fields.
fn parse<T: serde::de::DeserializeOwned>(mut value: Value, method: &str) -> Result<T, HelperError> {
    let wrapper = match method {
        "fetch_liked_songs" => Some("likedSongs"),
        "fetch_login_status" | "get_login_status" | "poll_login" => Some("login"),
        "get_helper_info" => Some("helper"),
        "get_status" => Some("status"),
        "fetch_user_playlists" => Some("playlists"),
        "fetch_liked_albums" | "fetch_artist_albums" => Some("albums"),
        "fetch_followed_artists" => Some("artists"),
        "fetch_playlist_tracks"
        | "fetch_artist_songs"
        | "fetch_toplist_tracks"
        | "fetch_radio_tracks"
        | "fetch_new_songs"
        | "fetch_recommend_feed" => Some("tracks"),
        "fetch_song_detail" | "fetch_album_detail" | "fetch_artist_detail" => Some("detail"),
        "fetch_artist_biography" => Some("artistDetail"),
        "fetch_toplist_categories" => Some("toplistGroups"),
        "fetch_radio_stations" => Some("radioGroups"),
        "search_track_artwork" | "search_artist_artwork" | "search_album_artwork" => {
            Some("candidates")
        }
        "resolve_song_url" => Some("stream"),
        "start_login" => Some("qrcode"),
        "fetch_lyric" => Some("lyric"),
        "set_rate_limit" => Some("rateLimit"),
        "set_breaker" => Some("breaker"),
        "aria2_status" | "aria2_restart" | "aria2_configure" => Some("aria2"),
        "aria2_add" | "aria2_tell" => Some("download"),
        _ => None,
    };
    let word_lyric = if method == "fetch_lyric" {
        value.get("wordLyric").cloned()
    } else {
        None
    };
    let mut candidate = if let Some(key) = wrapper {
        value
            .as_object_mut()
            .and_then(|object| object.remove(key))
            .ok_or_else(|| HelperError::Upstream(format!("{method} 响应缺少 {key}")))?
    } else {
        value
    };
    if method == "fetch_lyric" {
        let object = candidate
            .as_object_mut()
            .ok_or_else(|| HelperError::Upstream("fetch_lyric 的 lyric 必须是对象".into()))?;
        if let Some(word_lyric) = word_lyric {
            object.insert("wordLyric".into(), word_lyric);
        }
    }
    if method == "aria2_add" {
        // Queueing reports only the gid; progress is obtained by aria2_tell.
        let object = candidate
            .as_object_mut()
            .ok_or_else(|| HelperError::Upstream("aria2_add 的 download 必须是对象".into()))?;
        for (key, default) in [
            ("status", json!("")),
            ("completed", json!(0)),
            ("total", json!(0)),
            ("speed", json!(0)),
            ("path", json!("")),
            ("error", json!("")),
            ("errorCode", json!(0)),
        ] {
            object.entry(key.to_string()).or_insert(default);
        }
    }
    if matches!(
        method,
        "aria2_list" | "aria2_pause" | "aria2_unpause" | "aria2_cancel"
    ) {
        if let Some(object) = candidate.as_object_mut() {
            object
                .entry("removed".to_string())
                .or_insert_with(|| json!([]));
        }
    }
    serde_json::from_value(candidate)
        .map_err(|error| HelperError::Upstream(format!("{method}: {error}")))
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

/// Import the same login with an optional encrypted UIN for account relations.
/// Kept separate so the existing two-argument macOS API remains source-compatible.
#[export]
pub fn import_credential_with_encrypt_uin(
    uin: String,
    qm_keyst: String,
    encrypt_uin: Option<String>,
) -> Result<(), HelperError> {
    let mut credential: Credential = methods::credential_from_params(&json!({
        "cookies": { "uin": uin, "qm_keyst": qm_keyst }
    }))
    .map_err(HelperError::from)?;
    credential.encrypted_uin = encrypt_uin
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
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

/// A row-offset page of playlist or liked-folder tracks, with the source total.
/// `dir_id=201` and `list_id=0` addresses the reserved 我喜欢 folder.
#[export]
pub fn playlist_tracks_page(
    list_id: i64,
    dir_id: Option<i64>,
    offset: u32,
    limit: u32,
) -> Result<crate::models::TrackPage, HelperError> {
    call(
        "fetch_playlist_tracks_page",
        json!({ "listId": list_id, "dirId": dir_id, "offset": offset, "limit": limit }),
    )
}

/// Add or remove a song by numeric id and retain the upstream result code.
#[export]
pub fn set_liked_by_id(
    song_id: i64,
    liked: bool,
) -> Result<crate::models::LikeReceipt, HelperError> {
    call(
        "set_liked_by_id",
        json!({ "songId": song_id, "liked": liked }),
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
pub fn followed_artists(page: u32, limit: u32) -> Result<Vec<crate::models::Artist>, HelperError> {
    call(
        "fetch_followed_artists",
        json!({ "page": page, "limit": limit }),
    )
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

    fn track() -> Value {
        json!({"songMid":"SONG","title":"歌曲","artist":"歌手","albumId":42,"duration":180})
    }

    #[test]
    fn typed_details_unwrap_entities_and_keep_optional_fields() {
        let album: crate::models::AlbumDetail = parse(json!({"detail": {
            "albumMid":"ALBUM","title":"专辑","id":42,"songCount":12,"coverURL":"https://example.test/album"
        }}), "fetch_album_detail").unwrap();
        assert_eq!(album.album_mid.as_deref(), Some("ALBUM"));
        assert_eq!(album.song_count, Some(12));
        let song: crate::models::SongDetail = parse(
            json!({"detail": {
                "songMid":"SONG","description":"简介","genreTags":["流行"],"title":"歌曲"
            }}),
            "fetch_song_detail",
        )
        .unwrap();
        assert_eq!(song.description, "简介");
        assert_eq!(song.genre, vec!["流行"]);
        let artist: crate::models::ArtistDetail = parse(
            json!({"detail": {
                "singerMid":"ARTIST","name":"歌手","description":"介绍","genre":["摇滚"]
            }}),
            "fetch_artist_detail",
        )
        .unwrap();
        assert_eq!(artist.singer_mid, "ARTIST");
        let biography: crate::models::ArtistBiography = parse(
            json!({"artistDetail": {
                "artistName":"歌手","description":"传记","confidence":0.8
            }}),
            "fetch_artist_biography",
        )
        .unwrap();
        assert_eq!(biography.description.as_deref(), Some("传记"));
        assert!(
            parse::<crate::models::AlbumDetail>(json!({"wrong":{}}), "fetch_album_detail").is_err()
        );
        assert!(
            parse::<crate::models::ArtistBiography>(json!({}), "fetch_artist_biography").is_err()
        );
    }

    #[test]
    fn track_lists_unwrap_while_album_and_search_pages_retain_totals() {
        for method in [
            "fetch_playlist_tracks",
            "fetch_artist_songs",
            "fetch_toplist_tracks",
            "fetch_radio_tracks",
            "fetch_new_songs",
            "fetch_recommend_feed",
        ] {
            let tracks: Vec<crate::models::Track> =
                parse(json!({"tracks":[track()],"total":99}), method).unwrap();
            assert_eq!(tracks[0].song_mid, "SONG", "{method}");
        }
        let page: crate::models::TrackPage = parse(
            json!({"tracks":[track()],"total":99,"nextOffset":203}),
            "fetch_album_tracks",
        )
        .unwrap();
        assert_eq!(page.total, Some(99));
        assert_eq!(page.next_offset, Some(203));
        let search: crate::models::TrackSearch =
            parse(json!({"tracks":[track()],"total":75}), "search_songs").unwrap();
        assert_eq!(search.total, 75);
        let albums = json!([{"id":42,"title":"专辑","albumMid":"ALBUM"}]);
        for method in ["fetch_liked_albums", "fetch_artist_albums"] {
            let parsed: Vec<crate::models::Album> =
                parse(json!({"albums":albums}), method).unwrap();
            assert_eq!(parsed[0].id, 42);
        }
        let parsed: crate::models::AlbumSearch =
            parse(json!({"albums":albums,"total":25}), "search_albums").unwrap();
        assert_eq!(parsed.total, 25);
        let artists = json!([{"singerMid":"ARTIST","name":"歌手"}]);
        let parsed: crate::models::ArtistSearch =
            parse(json!({"artists":artists,"total":12}), "search_artists").unwrap();
        assert_eq!(parsed.artists[0].singer_mid, "ARTIST");
        let parsed: crate::models::PlaylistSearch = parse(
            json!({"playlists":[{"id":7,"title":"歌单"}],"total":8}),
            "search_playlists",
        )
        .unwrap();
        assert_eq!(parsed.total, 8);
    }

    #[test]
    fn group_and_artwork_wrappers_preserve_their_items() {
        let groups: Vec<crate::models::ToplistGroup> = parse(
            json!({"toplistGroups":[{
                "title":"榜单","toplists":[{"id":4,"title":"热歌"}]
            }]}),
            "fetch_toplist_categories",
        )
        .unwrap();
        assert_eq!(groups[0].toplists[0].id, 4);
        let groups: Vec<crate::models::RadioGroup> = parse(
            json!({"radioGroups":[{
                "title":"电台","stations":[{"id":6,"title":"流行"}]
            }]}),
            "fetch_radio_stations",
        )
        .unwrap();
        assert_eq!(groups[0].stations[0].title, "流行");
        for method in [
            "search_track_artwork",
            "search_artist_artwork",
            "search_album_artwork",
        ] {
            let candidates: Vec<crate::models::ArtworkCandidate> = parse(
                json!({"candidates":[{
                    "imageURL":"https://example.test/image","confidence":0.9
                }]}),
                method,
            )
            .unwrap();
            assert_eq!(candidates[0].confidence, Some(0.9));
        }
    }

    #[test]
    fn lyric_merges_word_track_and_stream_retains_playability() {
        let lyric: crate::models::Lyric = parse(
            json!({"lyric":{
            "lyric":"[00:01]原文","translation":"译文","romanization":"roma",
            "qrcLines":[{"startMs":1000,"durationMs":500,"words":[{"text":"原","startMs":1000,"durationMs":250}]}],
            "romanLines":[{"startMs":1000,"durationMs":500,"words":[{"text":"yuan","startMs":1000,"durationMs":500}]}]
        },"wordLyric":"word track"}),
            "fetch_lyric",
        )
        .unwrap();
        assert_eq!(lyric.lyric.as_deref(), Some("[00:01]原文"));
        assert_eq!(lyric.translation.as_deref(), Some("译文"));
        assert_eq!(lyric.word_lyric.as_deref(), Some("word track"));
        assert_eq!(lyric.qrc_lines.as_ref().unwrap()[0].words[0].text, "原");
        assert_eq!(
            lyric.qrc_lines.as_ref().unwrap()[0].words[0].duration_ms,
            250
        );
        assert_eq!(lyric.roman_lines.as_ref().unwrap()[0].words[0].text, "yuan");
        let lyric: crate::models::Lyric = parse(
            json!({"lyric":{"lyric":null},"wordLyric":null}),
            "fetch_lyric",
        )
        .unwrap();
        assert_eq!(lyric.word_lyric, None);
        for playable in [true, false] {
            let stream: crate::models::StreamResolution = parse(json!({"stream":{
                "songMid":"SONG","playable":playable,"url":if playable {Some("https://example.test/song")} else {None},
                "reason":if playable {None} else {Some("no grant")}
            }}), "resolve_song_url").unwrap();
            assert_eq!(stream.playable, playable);
            assert_eq!(
                stream.reason.as_deref(),
                if playable { None } else { Some("no grant") }
            );
        }
    }

    #[test]
    fn login_status_qr_and_poll_have_distinct_envelopes() {
        for method in ["get_login_status", "fetch_login_status"] {
            let login: crate::models::LoginStatus =
                parse(json!({"login":{"loggedIn":false}}), method).unwrap();
            assert!(!login.logged_in);
        }
        let qr: crate::models::LoginQrCode = parse(
            json!({"qrcode":{
                "identifier":"QR","loginType":"qq","mimetype":"image/png","imageBase64":"image"
            }}),
            "start_login",
        )
        .unwrap();
        assert_eq!(qr.identifier, "QR");
        let poll: crate::models::LoginPoll = parse(
            json!({"login":{
                "event":"DONE","loggedIn":true,"login":{"loggedIn":true,"musicId":9}
            }}),
            "poll_login",
        )
        .unwrap();
        assert_eq!(poll.login.unwrap().music_id, Some(9));
    }

    #[test]
    fn local_configuration_and_download_payloads_match_typed_returns() {
        let rate: crate::models::RateLimitConfigModel = parse(
            json!({"rateLimit":{
                "enabled":true,"windowSeconds":10,"maxRequests":20
            }}),
            "set_rate_limit",
        )
        .unwrap();
        assert_eq!(rate.max_requests, 20);
        let breaker: crate::models::BreakerConfigModel = parse(
            json!({"breaker":{
                "enabled":true,"failureThreshold":3,"failureWindowSeconds":60,"openSeconds":30
            }}),
            "set_breaker",
        )
        .unwrap();
        assert_eq!(breaker.failure_threshold, 3);
        let guard: crate::models::GuardStatus = parse(json!({"status":{
            "breaker":"closed","rateLimit":{"read":1,"interactive":2,"playback":3,"account":4,"write":5}
        }}), "get_status").unwrap();
        assert_eq!(guard.rate_limit.playback, 3);
        let queued: crate::models::Aria2Download =
            parse(json!({"download":{"gid":"GID"}}), "aria2_add").unwrap();
        assert_eq!(queued.gid, "GID");
        assert_eq!(queued.total, 0);
        let download = json!({"gid":"GID","status":"active","completed":10,"total":100,
            "speed":2,"path":"/tmp/song","error":"","errorCode":0});
        let progress: crate::models::Aria2Download =
            parse(json!({"download":download}), "aria2_tell").unwrap();
        assert_eq!(progress.completed, 10);
        let task = json!({"gid":"GID","status":"paused","completed":10,"total":100,"speed":0,"name":"song","path":"/tmp/song","error":""});
        for method in ["aria2_list", "aria2_pause", "aria2_unpause", "aria2_cancel"] {
            let tasks: crate::models::Aria2TaskList =
                parse(json!({"downloads":[task]}), method).unwrap();
            assert_eq!(tasks.downloads[0].gid, "GID");
            assert!(tasks.removed.is_empty());
        }
        let canceled: crate::models::Aria2TaskList =
            parse(json!({"downloads":[],"removed":["GID"]}), "aria2_cancel").unwrap();
        assert_eq!(canceled.removed, vec!["GID"]);
        let status = serde_json::to_value(crate::models::Aria2Status {
            binary: "/tmp/aria2".into(),
            port: 6800,
            ..Default::default()
        })
        .unwrap();
        for method in ["aria2_status", "aria2_restart", "aria2_configure"] {
            let parsed: crate::models::Aria2Status =
                parse(json!({"aria2":status}), method).unwrap();
            assert_eq!(parsed.port, 6800);
        }
    }

    #[test]
    fn existing_account_wrappers_and_direct_payloads_keep_their_shapes() {
        let liked: crate::models::LikedSongs = parse(
            json!({"likedSongs":{
                "title":"我喜欢","total":5,"tracks":[track()]
            }}),
            "fetch_liked_songs",
        )
        .unwrap();
        assert_eq!(liked.total, 5);
        let playlists: Vec<crate::models::Playlist> = parse(
            json!({"playlists":[{"id":7,"title":"歌单"}]}),
            "fetch_user_playlists",
        )
        .unwrap();
        assert_eq!(playlists[0].id, 7);
        let artists: Vec<crate::models::Artist> = parse(
            json!({"artists":[{"singerMid":"ARTIST","name":"歌手"}]}),
            "fetch_followed_artists",
        )
        .unwrap();
        assert_eq!(artists[0].name, "歌手");
        let info: crate::models::ComponentInfo = parse(json!({"helper":{
            "helperVersion":"1","protocolVersion":1,"libraryVersion":"1","methods":["fetch_lyric"]
        }}), "get_helper_info").unwrap();
        assert_eq!(info.methods, vec!["fetch_lyric"]);
        let direct = json!({"exists":true});
        assert_eq!(
            parse::<Value>(direct.clone(), "has_ai_dictionary").unwrap(),
            direct
        );
    }

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
            ("set_rate_limit", "set_rate_limit"),
            ("set_breaker", "set_breaker"),
            ("aria2_tell", "aria2_tell"),
            ("aria2_cancel", "aria2_cancel"),
            ("aria2_unpause", "aria2_unpause"),
            ("aria2_pause", "aria2_pause"),
            ("aria2_list", "aria2_list"),
            ("aria2_add", "aria2_add"),
            ("aria2_configure", "aria2_configure"),
            ("aria2_restart", "aria2_restart"),
            ("aria2_status", "aria2_status"),
            ("get_login_status", "login_status"),
            ("fetch_liked_songs", "liked_songs"),
            ("fetch_playlist_tracks", "playlist_tracks"),
            ("fetch_playlist_tracks_page", "playlist_tracks_page"),
            ("fetch_user_playlists", "user_playlists"),
            ("fetch_liked_albums", "liked_albums"),
            ("fetch_followed_artists", "followed_artists"),
            ("fetch_song_detail", "song_detail"),
            ("fetch_album_detail", "album_detail"),
            ("fetch_album_tracks", "album_tracks"),
            ("fetch_artist_songs", "artist_songs"),
            ("fetch_artist_songs_page", "artist_songs_page"),
            ("fetch_artist_albums", "artist_albums"),
            ("fetch_artist_albums_page", "artist_albums_page"),
            ("fetch_artist_detail", "artist_detail"),
            ("fetch_artist_biography", "fetch_artist_biography"),
            ("search_track_artwork", "search_track_artwork"),
            ("search_artist_artwork", "search_artist_artwork"),
            ("search_album_artwork", "search_album_artwork"),
            ("fetch_toplist_categories", "toplist_categories"),
            ("fetch_toplist_tracks", "toplist_tracks"),
            ("fetch_radio_stations", "radio_stations"),
            ("fetch_radio_tracks", "radio_tracks"),
            ("fetch_radio_track_batch", "radio_track_batch"),
            ("fetch_new_songs", "new_songs"),
            ("fetch_recommend_feed", "recommend_feed"),
            ("fetch_lyric", "lyric"),
            ("resolve_song_url", "resolve_song_url"),
            ("set_liked_by_id", "set_liked_by_id"),
            ("search_songs", "search_songs"),
            ("search_artists", "search_artists"),
            ("search_albums", "search_albums"),
            ("search_playlists", "search_playlists"),
            ("start_login", "start_login"),
            ("poll_login", "poll_login"),
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
    call(
        "fetch_album_detail",
        json!({ "albumMid": album_mid, "albumId": album_id }),
    )
}

/// An album's tracks.
#[export]
pub fn album_tracks(
    album_mid: Option<String>,
    album_id: Option<i64>,
    offset: i64,
    limit: i64,
) -> Result<crate::models::TrackPage, HelperError> {
    call(
        "fetch_album_tracks",
        json!({ "albumMid": album_mid, "albumId": album_id, "offset": offset, "limit": limit }),
    )
}

/// An artist's songs. `sort` is `hot` or `latest` — both go to the upstream's
/// own global ordering (`order=1` / `order=2`); nothing is sorted locally.
#[export]
pub fn artist_songs(
    singer_mid: String,
    sort: String,
    page: i64,
    limit: i64,
) -> Result<Vec<crate::models::Track>, HelperError> {
    call(
        "fetch_artist_songs",
        json!({ "singerMid": singer_mid, "sort": sort, "page": page, "limit": limit }),
    )
}

/// An artist's globally ordered, row-offset song page with the source total.
#[export]
pub fn artist_songs_page(
    singer_mid: String,
    sort: String,
    offset: u32,
    limit: u32,
) -> Result<crate::models::TrackPage, HelperError> {
    call(
        "fetch_artist_songs_page",
        json!({ "singerMid": singer_mid, "sort": sort, "offset": offset, "limit": limit }),
    )
}

/// An artist's albums, same two sorts.
#[export]
pub fn artist_albums(
    singer_mid: String,
    sort: String,
    page: i64,
    limit: i64,
) -> Result<Vec<crate::models::Album>, HelperError> {
    call(
        "fetch_artist_albums",
        json!({ "singerMid": singer_mid, "sort": sort, "page": page, "limit": limit }),
    )
}

/// An artist's globally ordered album page, with total and song counts.
#[export]
pub fn artist_albums_page(
    singer_mid: String,
    sort: String,
    offset: u32,
    limit: u32,
) -> Result<crate::models::AlbumPage, HelperError> {
    call(
        "fetch_artist_albums_page",
        json!({ "singerMid": singer_mid, "sort": sort, "offset": offset, "limit": limit }),
    )
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
    call(
        "fetch_toplist_tracks",
        json!({ "topId": top_id, "offset": offset, "limit": limit }),
    )
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
    call(
        "fetch_radio_tracks",
        json!({ "stationId": station_id, "limit": limit, "firstPlay": first_play }),
    )
}

/// Fetch one fresh batch from a QQ Music infinite radio rotation.
#[export]
pub fn radio_track_batch(
    station_id: i64,
    first_play: bool,
) -> Result<crate::models::TrackPage, HelperError> {
    call(
        "fetch_radio_track_batch",
        json!({ "stationId": station_id, "firstPlay": first_play }),
    )
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
        _ => {
            return Err(HelperError::InvalidRequest(
                "params 必须是 JSON 对象".into(),
            ))
        }
    };
    object.insert("platform".into(), Value::String(profile.as_str().into()));
    let value = dispatch_payload(&method, &Value::Object(object))?;
    serde_json::to_string(&value).map_err(|error| HelperError::Upstream(error.to_string()))
}

/// Add to, or remove from, "我喜欢". The only write this component performs.
#[export]
pub fn set_liked(song_mid: String, song_type: i64, liked: bool) -> Result<(), HelperError> {
    let _: Value = call(
        "set_liked",
        json!({ "songMid": song_mid, "songType": song_type, "liked": liked }),
    )?;
    Ok(())
}

// MARK: - Logging in

/// Start a QQ scan-to-login: returns the code to draw and the identifier to
/// poll with.
#[export]
pub fn start_login() -> Result<crate::models::LoginQrCode, HelperError> {
    call("start_login", json!({}))
}

/// Ask what the scan has done; on the last step it stores the credential and
/// reports `logged_in`.
#[export]
pub fn poll_login(identifier: String) -> Result<crate::models::LoginPoll, HelperError> {
    call("poll_login", json!({ "identifier": identifier }))
}

// MARK: - Search

#[export]
pub fn search_songs(
    keyword: String,
    page: i64,
    limit: i64,
) -> Result<crate::models::TrackSearch, HelperError> {
    call(
        "search_songs",
        json!({ "keyword": keyword, "page": page, "limit": limit }),
    )
}

#[export]
pub fn search_artists(
    keyword: String,
    page: i64,
    limit: i64,
) -> Result<crate::models::ArtistSearch, HelperError> {
    call(
        "search_artists",
        json!({ "keyword": keyword, "page": page, "limit": limit }),
    )
}

#[export]
pub fn search_albums(
    keyword: String,
    page: i64,
    limit: i64,
) -> Result<crate::models::AlbumSearch, HelperError> {
    call(
        "search_albums",
        json!({ "keyword": keyword, "page": page, "limit": limit }),
    )
}

#[export]
pub fn search_playlists(
    keyword: String,
    page: i64,
    limit: i64,
) -> Result<crate::models::PlaylistSearch, HelperError> {
    call(
        "search_playlists",
        json!({ "keyword": keyword, "page": page, "limit": limit }),
    )
}

// MARK: - The local library's enrichment

/// Cover candidates for a local track (the host scores them).
#[export]
pub fn search_track_artwork(
    title: String,
    artist: String,
    album: String,
    limit: i64,
) -> Result<Vec<crate::models::ArtworkCandidate>, HelperError> {
    call(
        "search_track_artwork",
        json!({ "title": title, "artist": artist, "album": album, "limit": limit }),
    )
}

/// Cover candidates for a local artist.
#[export]
pub fn search_artist_artwork(
    name: String,
    limit: i64,
) -> Result<Vec<crate::models::ArtworkCandidate>, HelperError> {
    call(
        "search_artist_artwork",
        json!({ "name": name, "limit": limit }),
    )
}

/// Cover candidates for a local album.
#[export]
pub fn search_album_artwork(
    album: String,
    artist: String,
    limit: i64,
) -> Result<Vec<crate::models::ArtworkCandidate>, HelperError> {
    call(
        "search_album_artwork",
        json!({ "album": album, "artist": artist, "limit": limit }),
    )
}

/// Apply the user's request-rate ceiling.
///
/// A configuration push rather than a read: the app sends it on startup and
/// whenever the settings change, so the numbers survive a component restart.
#[export]
pub fn set_rate_limit(
    enabled: bool,
    window_seconds: i64,
    max_requests: i64,
) -> Result<crate::models::RateLimitConfigModel, HelperError> {
    call(
        "set_rate_limit",
        json!({
            "enabled": enabled,
            "windowSeconds": window_seconds,
            "maxRequests": max_requests,
        }),
    )
}

/// Apply the user's circuit-breaker numbers.
#[export]
pub fn set_breaker(
    enabled: bool,
    failure_threshold: i64,
    failure_window_seconds: i64,
    open_seconds: i64,
) -> Result<crate::models::BreakerConfigModel, HelperError> {
    call(
        "set_breaker",
        json!({
            "enabled": enabled,
            "failureThreshold": failure_threshold,
            "failureWindowSeconds": failure_window_seconds,
            "openSeconds": open_seconds,
        }),
    )
}

/// The download engine's state. `ensure` starts it when it is not up.
#[export]
pub fn aria2_status(ensure: bool) -> Result<crate::models::Aria2Status, HelperError> {
    call("aria2_status", json!({ "ensure": ensure }))
}

/// Restart the download engine.
#[export]
pub fn aria2_restart() -> Result<crate::models::Aria2Status, HelperError> {
    call("aria2_restart", json!({}))
}

/// Apply the download engine's numbers.
#[export]
pub fn aria2_configure(
    split: i64,
    max_connection_per_server: i64,
    max_concurrent_downloads: i64,
    min_split_size_mib: i64,
    max_overall_download_limit_kib: i64,
    port: i64,
) -> Result<crate::models::Aria2Status, HelperError> {
    call(
        "aria2_configure",
        json!({
            "split": split,
            "maxConnectionPerServer": max_connection_per_server,
            "maxConcurrentDownloads": max_concurrent_downloads,
            "minSplitSizeMiB": min_split_size_mib,
            "maxOverallDownloadLimitKiB": max_overall_download_limit_kib,
            "port": port,
        }),
    )
}

/// Queue one file.
#[export]
pub fn aria2_add(url: String, out: String) -> Result<crate::models::Aria2Download, HelperError> {
    call("aria2_add", json!({ "url": url, "out": out }))
}

/// One download's progress.
#[export]
pub fn aria2_tell(gid: String) -> Result<crate::models::Aria2Download, HelperError> {
    call("aria2_tell", json!({ "gid": gid }))
}

/// Every task the engine holds.
#[export]
pub fn aria2_list() -> Result<crate::models::Aria2TaskList, HelperError> {
    call("aria2_list", json!({}))
}

/// Pause one task (or all when `gid` is empty).
#[export]
pub fn aria2_pause(gid: Option<String>) -> Result<crate::models::Aria2TaskList, HelperError> {
    call("aria2_pause", json!({ "gid": gid }))
}

/// Resume one task (or all).
#[export]
pub fn aria2_unpause(gid: Option<String>) -> Result<crate::models::Aria2TaskList, HelperError> {
    call("aria2_unpause", json!({ "gid": gid }))
}

/// Cancel one task (or all), deleting the partial files.
#[export]
pub fn aria2_cancel(gid: Option<String>) -> Result<crate::models::Aria2TaskList, HelperError> {
    call("aria2_cancel", json!({ "gid": gid }))
}

/// An artist's biography.
#[export]
pub fn fetch_artist_biography(
    name: String,
    singer_mid: Option<String>,
) -> Result<crate::models::ArtistBiography, HelperError> {
    call(
        "fetch_artist_biography",
        json!({ "name": name, "singerMid": singer_mid }),
    )
}

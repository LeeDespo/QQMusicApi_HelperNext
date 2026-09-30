//! The endpoints this component serves, in the wire shape the app already
//! decodes.
//!
//! The component deliberately speaks the *same* JSON protocol as the Python
//! helper it replaces — one request per line, `{"id", "method", "params"}`, and a
//! reply carrying the same `id` back. Two reasons: the app's process client
//! already implements that transport (so the swap is a binary path, not a
//! rewrite), and any method that is not ported yet keeps working during the
//! migration.
//!
//! Payload mapping notes, endpoint by endpoint, live in `docs/qqmusic/25-*`;
//! every request shape below is copied from the library the old helper used.

use crate::credential::{credential_from_cookies, Credential};
use crate::guard::Class;
use crate::upstream::{first_array, first_int, first_object, first_text, Call, Platform, UpstreamError};
pub use crate::upstream::Upstream;
use serde_json::{json, Value};

pub const COMPONENT_VERSION: &str = "0.1.0";
/// The protocol the app speaks; unchanged from the Python helper.
pub const PROTOCOL_VERSION: i32 = 2;
/// The library the old helper shipped, reported so both components answer
/// `get_helper_info` with the same field set.
const LIBRARY_VERSION: &str = "qqmusic-api-python 0.7.3 (replaced by this component)";
/// "我喜欢" lives in the reserved folder with this id.
const LIKED_SONGS_DIRID: i64 = 201;

pub const METHODS: &[&str] = &[
    "get_helper_info",
    "get_login_status",
    "import_cookies",
    "logout",
    "fetch_liked_songs",
    "fetch_liked_albums",
    "fetch_user_playlists",
    "fetch_followed_artists",
    "fetch_playlist_tracks",
    "get_status",
    // Catalogue prose, an artist's works and an album's tracks.
    "fetch_song_detail",
    "fetch_album_detail",
    "fetch_album_tracks",
    "fetch_artist_songs",
    "fetch_artist_albums",
    "fetch_artist_detail",
    // Rankings, radio, new songs and the recommendation feed.
    "fetch_toplist_categories",
    "fetch_toplist_tracks",
    "fetch_radio_stations",
    "fetch_radio_tracks",
    "fetch_new_songs",
    "fetch_recommend_feed",
    // Lyrics and playback urls.
    "fetch_lyric",
    "resolve_song_url",
];

pub fn is_known(method: &str) -> bool {
    METHODS.contains(&method)
}

/// Payload for one method, returned as a JSON object the CLI hands straight back
/// and the typed API parses into a model. Endpoints live in `catalog`; this is
/// the name-to-endpoint table plus the small account reads that were here first.
fn catalog_dispatch(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    method: &str,
    params: &Value,
) -> Option<Result<Value, UpstreamError>> {
    let text = |key: &str| first_text(params, &[key]).unwrap_or_default();
    let int = |key: &str| first_int(params, &[key]);
    let result = match method {
        "fetch_song_detail" => crate::catalog::song_detail(upstream, credential, platform, &text("songMid")),
        "fetch_album_detail" => crate::catalog::album_detail(
            upstream,
            credential,
            platform,
            first_text(params, &["albumMid"]).as_deref(),
            int("albumId"),
        ),
        "fetch_album_tracks" => crate::catalog::album_tracks(
            upstream,
            credential,
            platform,
            first_text(params, &["albumMid"]).as_deref(),
            int("albumId"),
            int("offset").unwrap_or(0),
            round(int("limit"), 200, 1, 200),
        )
        .map(|tracks| json!({ "tracks": tracks })),
        "fetch_artist_songs" => crate::catalog::artist_songs(
            upstream,
            credential,
            platform,
            &text("singerMid"),
            &first_text(params, &["sort"]).unwrap_or_else(|| "hot".into()),
            int("page").unwrap_or(1),
            round(int("limit"), 50, 1, 100),
        )
        .map(|tracks| json!({ "tracks": tracks })),
        "fetch_artist_albums" => crate::catalog::artist_albums(
            upstream,
            credential,
            platform,
            &text("singerMid"),
            &first_text(params, &["sort"]).unwrap_or_else(|| "hot".into()),
            int("page").unwrap_or(1),
            round(int("limit"), 50, 1, 100),
        )
        .map(|albums| json!({ "albums": albums })),
        "fetch_artist_detail" => crate::catalog::artist_detail(upstream, credential, platform, &text("singerMid"))
            .map(|detail| json!({ "detail": detail })),
        "fetch_toplist_categories" => crate::catalog::toplist_categories(upstream, credential, platform)
            .map(|groups| json!({ "toplistGroups": groups })),
        "fetch_toplist_tracks" => crate::catalog::toplist_tracks(
            upstream,
            credential,
            platform,
            int("topId").unwrap_or(0),
            int("offset").unwrap_or(0),
            round(int("limit"), 100, 1, 300),
        )
        .map(|tracks| json!({ "tracks": tracks })),
        "fetch_radio_stations" => crate::catalog::radio_stations(upstream, credential, platform)
            .map(|groups| json!({ "radioGroups": groups })),
        "fetch_radio_tracks" => crate::catalog::radio_tracks(
            upstream,
            credential,
            platform,
            int("stationId").unwrap_or(0),
            round(int("limit"), 20, 1, 50),
            params.get("firstPlay").and_then(Value::as_bool).unwrap_or(true),
        )
        .map(|tracks| json!({ "tracks": tracks })),
        "fetch_new_songs" => crate::catalog::new_songs(
            upstream,
            credential,
            platform,
            int("regionType").unwrap_or(0),
        )
        .map(|tracks| json!({ "tracks": tracks })),
        "fetch_recommend_feed" => crate::catalog::recommend_feed(upstream, credential, platform)
            .map(|tracks| json!({ "tracks": tracks })),
        "fetch_lyric" => crate::catalog::lyric(
            upstream,
            credential,
            platform,
            &text("songMid"),
            int("songId"),
            params.get("wordTiming").and_then(Value::as_bool).unwrap_or(true),
            params.get("translation").and_then(Value::as_bool).unwrap_or(true),
        )
        .map(|lyric| json!({ "lyric": lyric })),
        "resolve_song_url" => crate::catalog::stream_url(
            upstream,
            credential,
            platform,
            &text("songMid"),
            first_text(params, &["mediaMid"]).as_deref(),
            int("songType").unwrap_or(0),
            first_text(params, &["quality"]).as_deref(),
        )
        .map(|stream| json!({ "stream": stream })),
        _ => return None,
    };
    Some(result)
}

fn round(value: Option<i64>, default: i64, low: i64, high: i64) -> i64 {
    value.unwrap_or(default).clamp(low, high)
}

/// The platform profile this request asks for: `params.platform` when given
/// (per-call choice), otherwise the host's configured default.
///
/// Exposed through the protocol rather than only through the typed API, so a
/// host driving the component as a process can choose per call too.
pub fn platform_for(params: &Value, default: Platform) -> Platform {
    first_text(params, &["platform"])
        .and_then(|value| Platform::parse(&value))
        .unwrap_or(default)
}

/// Dispatch one method. `credential` is the account as currently stored.
pub fn dispatch(
    upstream: &Upstream,
    credential: Option<&Credential>,
    method: &str,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let account = credential.cloned().unwrap_or_default();
    let platform = platform_for(params, crate::upstream::Platform::default());
    if let Some(result) = catalog_dispatch(upstream, &account, platform, method, params) {
        return result;
    }
    match method {
        "get_helper_info" => Ok(json!({
            "helper": {
                "helperVersion": COMPONENT_VERSION,
                "protocolVersion": PROTOCOL_VERSION,
                "libraryVersion": LIBRARY_VERSION,
                "credentialDir": true,
                "methods": METHODS,
            }
        })),
        "get_login_status" => Ok(json!({ "login": login_status(upstream, &account)? })),
        // Observability for the two polite mechanisms: "am I being throttled by
        // my own limiter, or is the upstream refusing me?" is otherwise
        // unanswerable from outside.
        "get_status" => Ok(json!({
            "status": {
                "breaker": match upstream.breaker.state() {
                    crate::guard::BreakerState::Closed => "closed",
                    crate::guard::BreakerState::HalfOpen => "half-open",
                    crate::guard::BreakerState::Open { .. } => "open",
                },
                "rateLimit": {
                    "read": upstream.limiter.usage(Class::Read),
                    "interactive": upstream.limiter.usage(Class::Interactive),
                    "playback": upstream.limiter.usage(Class::Playback),
                    "account": upstream.limiter.usage(Class::Account),
                    "write": upstream.limiter.usage(Class::Write),
                },
            }
        })),
        // Handled by the entry point, which owns the credential file: the
        // component never puts a credential in a JSON reply.
        "import_cookies" => Err(UpstreamError::Upstream("import_cookies 由入口处理".into())),
        "logout" => Ok(json!({ "login": json!({ "loggedIn": false }) })),
        "fetch_liked_songs" => Ok(json!({ "likedSongs": liked_songs(upstream, &account, params)? })),
        "fetch_liked_albums" => Ok(json!({ "albums": liked_albums(upstream, &account, params)? })),
        "fetch_user_playlists" => Ok(json!({ "playlists": user_playlists(upstream, &account, params)? })),
        "fetch_followed_artists" => Ok(json!({ "artists": followed_artists(upstream, &account, params)? })),
        "fetch_playlist_tracks" => Ok(json!({ "tracks": playlist_tracks(upstream, &account, params)? })),
        other => Err(UpstreamError::Upstream(format!("不支持的方法：{other}"))),
    }
}

/// A credential built from imported cookies, for the caller to persist.
pub fn credential_from_params(params: &Value) -> Result<Credential, UpstreamError> {
    let cookies = params
        .get("cookies")
        .ok_or_else(|| UpstreamError::Upstream("缺少 cookies".into()))?;
    credential_from_cookies(cookies)
        .ok_or_else(|| UpstreamError::Upstream("cookie 里没有 qm_keyst 或 uin，无法登录".into()))
}

/// Who is logged in, straight from the upstream rather than from the file, so an
/// expired session is visible as such.
fn login_status(upstream: &Upstream, credential: &Credential) -> Result<Value, UpstreamError> {
    if !credential.is_usable() {
        return Ok(json!({ "loggedIn": false }));
    }
    let data = match upstream.call(
        credential,
        Class::Account,
        Call {
            module: "music.UserInfo.userInfoServer",
            method: "GetLoginUserInfo",
            param: json!({}),
        },
    ) {
        Ok(data) => data,
        Err(UpstreamError::Upstream(_)) => {
            // A rejected credential is an answer, not a failure: report it as
            // "not logged in" so the app shows the login entry again.
            return Ok(json!({ "loggedIn": false, "hasPlaybackKey": !credential.music_key.is_empty() }));
        }
        Err(error) => return Err(error),
    };
    // The profile really is under `info`, and the nickname really is `nick` —
    // verified against a live response, not guessed from the library's model.
    let profile = first_object(&data, &["info", "user", "profile"]).unwrap_or(&data);
    Ok(json!({
        "loggedIn": true,
        "musicId": first_int(profile, &["musicid", "musicId", "uin"]).or_else(|| credential.music_id.parse().ok()),
        "nickname": first_text(profile, &["nick", "nickname", "name"]),
        "vipType": first_int(profile, &["viptype", "vipType", "vip"]).unwrap_or(0),
        "expired": false,
        "hasPlaybackKey": !credential.music_key.is_empty(),
    }))
}

/// 我喜欢, one page at a time. The folder reports its own total, which is what
/// lets the app page without guessing and know when it has everything.
fn liked_songs(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let page = first_int(params, &["page"]).unwrap_or(1).max(1);
    let limit = first_int(params, &["limit"]).unwrap_or(50).clamp(1, 100);
    let data = upstream.call(
        credential,
        Class::Account,
        Call {
            module: "music.srfDissInfo.DissInfo",
            method: "CgiGetDiss",
            param: json!({
                "disstid": 0,
                "dirid": LIKED_SONGS_DIRID,
                "tag": true,
                "song_begin": limit * (page - 1),
                "song_num": limit,
                "userinfo": true,
                "orderlist": true,
            }),
        },
    )?;
    let info = first_object(&data, &["dirinfo"]).cloned().unwrap_or(json!({}));
    let tracks = decoded_tracks(&data);
    Ok(json!({
        "title": first_text(&info, &["title"]).unwrap_or_else(|| "我喜欢".into()),
        "total": first_int(&info, &["songnum", "song_num", "total"]).unwrap_or(tracks.len() as i64),
        "tracks": tracks,
    }))
}

/// A playlist's tracks, addressed by its numeric id.
///
/// `dirinfo.songnum` is the list's real total, which is the only way the app can
/// tell "100 rows is everything" from "100 rows is the first page" — the Python
/// helper's own paging route could not, which is why the app's playlist pages
/// used to stop at 100.
fn playlist_tracks(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Vec<Value>, UpstreamError> {
    require_login(credential)?;
    let disstid = first_int(params, &["songlistId", "disstid", "id"])
        .or_else(|| first_int(params, &["topId"]))
        .ok_or_else(|| UpstreamError::Upstream("缺少 songlistId".into()))?;
    let limit = first_int(params, &["limit"]).unwrap_or(100).clamp(1, 200);
    let offset = first_int(params, &["offset", "song_begin"]).unwrap_or(0).max(0);
    let data = upstream.call(
        credential,
        Class::Account,
        Call {
            module: "music.srfDissInfo.DissInfo",
            method: "CgiGetDiss",
            param: json!({
                "disstid": disstid,
                "dirid": 0,
                "tag": true,
                "song_begin": offset,
                "song_num": limit,
                "userinfo": true,
                "orderlist": true,
            }),
        },
    )?;
    Ok(decoded_tracks(&data))
}

/// The account's own playlists (created and favorited), through the legacy fcgi.
fn user_playlists(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Vec<Value>, UpstreamError> {
    require_login(credential)?;
    let limit = first_int(params, &["limit"]).unwrap_or(100).clamp(1, 100);
    let data = upstream.profile_assets(credential, 3, limit as u32)?;
    let items = first_array(&data, &["cdlist", "disslist", "list"]).cloned().unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|item| {
            // The reserved folders (the liked-songs folder answers `dirid: 201`
            // with no `dissid`) are skipped, as in the app's web path.
            let id = first_int(item, &["dissid", "tid", "id"])?;
            if id <= 0 {
                return None;
            }
            Some(json!({
                "source": "qqmusic",
                "id": id,
                "title": first_text(item, &["dissname", "title", "name"]).unwrap_or_else(|| "未命名歌单".into()),
                "coverURL": normalized_artwork_url(first_text(item, &["logo", "picurl"]).as_deref()),
                "creator": first_text(item, &["nickname", "creator"]).unwrap_or_default(),
                "songCount": first_int(item, &["songnum", "song_cnt"]),
                "playCount": first_int(item, &["listennum", "play_cnt"]),
            }))
        })
        .collect())
}

/// Favorited albums (`reqtype: 2` on the same legacy endpoint).
fn liked_albums(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Vec<Value>, UpstreamError> {
    require_login(credential)?;
    let limit = first_int(params, &["limit"]).unwrap_or(30).clamp(1, 100);
    let data = upstream.profile_assets(credential, 2, limit as u32)?;
    let items = first_array(&data, &["albumlist", "cdlist", "list"]).cloned().unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|item| {
            let mid = first_text(item, &["albummid", "albumMid", "mid"]);
            let id = first_int(item, &["albumid", "albumId", "id"])?;
            Some(json!({
                "source": "qqmusic",
                "id": id,
                "title": first_text(item, &["albumname", "albumName", "name", "title"]).unwrap_or_default(),
                "albumMid": mid,
                "coverURL": normalized_artwork_url(mid.as_deref().map(album_cover_url).as_deref()),
                "artist": first_text(item, &["singername", "singerName", "singer"]),
                "releaseDate": album_release_date(item),
            }))
        })
        .collect())
}

/// The singers the account follows. `HostUin` is the *encrypted* uin.
fn followed_artists(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Vec<Value>, UpstreamError> {
    require_login(credential)?;
    if credential.encrypted_uin.is_empty() {
        return Err(UpstreamError::Upstream("凭据里没有 encrypt_uin".into()));
    }
    let limit = first_int(params, &["limit"]).unwrap_or(30).clamp(1, 100);
    let page = first_int(params, &["page"]).unwrap_or(1).max(1);
    let data = upstream.call(
        credential,
        Class::Account,
        Call {
            module: "music.concern.RelationList",
            method: "GetFollowSingerList",
            param: json!({
                "HostUin": credential.encrypted_uin,
                "From": (page - 1) * limit,
                "Size": limit,
            }),
        },
    )?;
    // The key really is `List` with a capital L — the library's model calls it
    // `users`, and reading the wrong name cost a round of "this feature does not
    // exist" once already.
    let items = first_array(&data, &["List", "list", "users"]).cloned().unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|item| {
            let mid = first_text(item, &["MID", "mid"])?;
            let name = first_text(item, &["Name", "name"]).unwrap_or_else(|| "未知歌手".into());
            Some(json!({
                "source": "qqmusic",
                "singerMid": mid,
                "name": name,
                "coverURL": normalized_artwork_url(first_text(item, &["AvatarUrl", "avatarUrl"]).as_deref()),
                "fanCount": first_int(item, &["FanNum", "fanNum"]),
            }))
        })
        .collect())
}

fn require_login(credential: &Credential) -> Result<(), UpstreamError> {
    if credential.is_usable() {
        Ok(())
    } else {
        Err(UpstreamError::Upstream("需要登录后才能读取".into()))
    }
}

/// Map one upstream song entry onto the track shape the app decodes.
///
/// Field names differ per endpoint (`mid`/`songmid`, singers as a list, sizes
/// keyed by code), so the mapping is written against the widest set and every
/// accessor takes alternatives — the same approach the Python helper's
/// `_track_payload` used.
pub fn decoded_tracks(data: &Value) -> Vec<Value> {
    // Candidate key spellings matter more than they look: the same list of
    // tracks arrives as `songlist` (我喜歡), `songList` (an artist's songs, an
    // album's tracks), `tracks` (the recommendation feed) and `songInfoList`
    // (a ranking). A missing spelling drops every row silently — which is
    // exactly what happened once, and why real responses get dumped rather than
    // modelled from memory.
    let items = first_array(
        data,
        &["songlist", "songList", "songs", "list", "tracks", "songInfoList"],
    )
    .cloned()
    .unwrap_or_default();
    items.iter().filter_map(decode_track).collect()
}

fn decode_track(item: &Value) -> Option<Value> {
    let track = first_object(item, &["track", "song", "songInfo"]).unwrap_or(item);
    let song_mid = first_text(track, &["mid", "songMid", "songmid"])?;
    let album = first_object(track, &["album", "albumInfo"]);
    let album_mid = album.and_then(|album| first_text(album, &["mid", "albumMid", "albummid"]));
    let singers: Vec<Value> = first_array(track, &["singer"])
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|singer| {
            let mid = first_text(singer, &["mid"]);
            let name = first_text(singer, &["name"]);
            if mid.is_none() && name.is_none() {
                return None;
            }
            Some(json!({ "mid": mid, "name": name }))
        })
        .collect();
    let artist = singers
        .iter()
        .filter_map(|singer| singer.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(", ");
    let pay = first_object(track, &["pay", "payInfo"]);
    let pay_play = pay
        .and_then(|pay| first_int(pay, &["pay_play", "payPlay"]))
        .or_else(|| first_int(track, &["pay_play", "payPlay"]));

    Some(json!({
        "source": "qqmusic",
        "songId": first_int(track, &["id", "songId", "songid"]),
        "songMid": song_mid,
        "mediaMid": first_int(track, &["media_mid"]).map(|_| ()).and(first_text(track, &["media_mid", "mediaMid"])),
        "title": first_text(track, &["name", "title", "songname"]).unwrap_or_else(|| "未知歌曲".into()),
        "artist": if artist.is_empty() { "未知艺人".to_string() } else { artist },
        "album": album.and_then(|album| first_text(album, &["name", "albumName"])),
        "albumMid": album_mid.clone(),
        "albumId": album.and_then(|album| first_int(album, &["id", "albumId"])),
        "imageURL": normalized_artwork_url(album_mid.as_deref().map(album_cover_url).as_deref()),
        "duration": first_int(track, &["interval", "duration"]),
        "payPlay": pay_play,
        "singerMid": singers.first().and_then(|singer| singer.get("mid").and_then(Value::as_str)),
        "singers": singers,
    }))
}

/// The legacy favourited-albums endpoint answers `pubtime` as a Unix timestamp
/// while everything else here answers a `YYYY-MM-DD` string; the app displays
/// this field directly, so it is normalised here rather than in the UI.
///
/// The timestamp is **Beijing midnight**: checked against eight favourited
/// albums, every value was exactly 16:00 UTC (e.g. 流浪地球 → 2019-02-04T16:00Z).
/// Rendering it in UTC would make every album read one day early, so the +08:00
/// offset is applied here — that is the date the service itself shows.
const QQ_MUSIC_UTC_OFFSET_SECONDS: i64 = 8 * 3600;

fn album_release_date(item: &Value) -> Option<String> {
    if let Some(text) = first_text(item, &["publishDate", "time_public"]) {
        return Some(text);
    }
    let seconds = first_int(item, &["pubtime", "publishDate"])?;
    if seconds <= 0 {
        return None;
    }
    Some(civil_date_from_unix(seconds + QQ_MUSIC_UTC_OFFSET_SECONDS))
}

/// Days-to-civil conversion (Howard Hinnant's algorithm), so the component needs
/// no date library for one field.
fn civil_date_from_unix(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn album_cover_url(mid: &str) -> String {
    format!("https://y.gtimg.cn/music/photo_new/T002R800x800M000{mid}.jpg")
}

/// Force artwork URLs onto HTTPS.
///
/// The upstream hands back `http://y.gtimg.cn/...` and the app has no ATS
/// exception, so an http cover is refused outright and stays blank.
pub fn normalized_artwork_url(value: Option<&str>) -> Option<String> {
    let value = value.filter(|value| !value.is_empty())?;
    if let Some(rest) = value.strip_prefix("http://") {
        return Some(format!("https://{rest}"));
    }
    if let Some(rest) = value.strip_prefix("//") {
        return Some(format!("https://{}", rest.trim_start_matches('/')));
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_urls_are_forced_onto_https() {
        assert_eq!(
            normalized_artwork_url(Some("http://y.gtimg.cn/a.jpg")).unwrap(),
            "https://y.gtimg.cn/a.jpg"
        );
        assert_eq!(
            normalized_artwork_url(Some("//qpic.y.qq.com/a.jpg")).unwrap(),
            "https://qpic.y.qq.com/a.jpg"
        );
        assert!(normalized_artwork_url(None).is_none());
    }

    #[test]
    fn a_track_maps_every_field_the_app_reads() {
        let raw = json!({
            "mid": "song-1",
            "name": "合唱",
            "interval": 215,
            "album": {"id": 4321, "mid": "album-mid", "name": "专辑名"},
            "singer": [
                {"mid": "mid-a", "name": "甲"},
                {"mid": "mid-b", "name": "乙"}
            ],
            "pay": {"pay_play": 0}
        });
        let track = decode_track(&raw).expect("decodes");
        assert_eq!(track["songMid"], "song-1");
        assert_eq!(track["artist"], "甲, 乙");
        assert_eq!(track["albumId"], 4321);
        assert_eq!(track["singerMid"], "mid-a");
        assert_eq!(track["singers"].as_array().unwrap().len(), 2);
        assert_eq!(track["duration"], 215);
        assert_eq!(
            track["imageURL"],
            "https://y.gtimg.cn/music/photo_new/T002R800x800M000album-mid.jpg"
        );
    }

    #[test]
    fn timestamps_become_display_dates() {
        assert_eq!(civil_date_from_unix(0), "1970-01-01");
        assert_eq!(civil_date_from_unix(1_190_131_200), "2007-09-18");
        // …and the release date of that same album, which is Beijing midnight and
        // therefore the 19th, which is what the service shows.
        assert_eq!(
            album_release_date(&json!({"pubtime": 1_190_131_200})).unwrap(),
            "2007-09-19"
        );
        assert_eq!(
            album_release_date(&json!({"pubtime": 1_549_296_000})).unwrap(),
            "2019-02-05",
            "流浪地球: 2019-02-04T16:00Z is the 5th in Beijing"
        );
        assert_eq!(
            album_release_date(&json!({"publishDate": "2007-09-20"})).unwrap(),
            "2007-09-20"
        );
    }

    #[test]
    fn helper_info_reports_the_protocol_the_app_speaks() {
        let upstream = Upstream::new();
        let value = dispatch(&upstream, None, "get_helper_info", &json!({})).expect("ok");
        assert_eq!(value["helper"]["helperVersion"], COMPONENT_VERSION);
        assert_eq!(value["helper"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(value["helper"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m == "fetch_liked_songs"));
    }

    #[test]
    fn account_reads_refuse_without_a_credential() {
        let upstream = Upstream::new();
        let error = dispatch(&upstream, None, "fetch_liked_songs", &json!({})).unwrap_err();
        assert!(error.to_string().contains("登录"));
    }

    #[test]
    fn a_cookie_import_produces_a_credential_for_the_caller_to_store() {
        let credential = credential_from_params(
            &json!({"cookies": {"uin": "1234567890", "qm_keyst": "KEY"}}),
        )
        .expect("a complete cookie set is a login");
        assert_eq!(credential.music_id, "1234567890");
        let missing = credential_from_params(&json!({"cookies": {}}));
        assert!(missing.is_err(), "a cookie set without qm_keyst is not a login");
    }
}

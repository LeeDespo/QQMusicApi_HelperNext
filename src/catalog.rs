//! The catalogue endpoints: song and album prose, an artist's works, the
//! rankings and radio lists, lyrics and playback urls.
//!
//! Each function is one request plus the parsing the upstream needs. The shapes
//! were taken from QQMusicApi's own protocol code and then confirmed against a
//! live account; `docs/endpoints.md` records the request each one sends and
//! `docs/parsing.md` the response traps. Read those before changing anything
//! here — several of the field paths look wrong until you know why they are
//! right (the ranking list living in `songInfoList`, the album's `pubtime`
//! being Beijing midnight, the lyric's four base64 fields).

use crate::credential::Credential;
use crate::upstream::{first_array, first_int, first_object, first_text, Call, Platform, Upstream, UpstreamError};
use serde_json::{json, Value};

/// Streaming CDN prefix stripped from the upstream's relative `purl`.
const STREAM_CDN: &str = "https://isure.stream.qqmusic.qq.com/";

/// Quality ladder, best first: `(label, filename prefix, extension)`.
///
/// The upstream grants what the account is entitled to per file, so a track the
/// account may play at 320 kbps answers for `M800` and refuses `F000`; probing
/// from the top and stopping at the first grant is both the cheapest and the
/// only correct way to learn what is actually available.
const QUALITY_LADDER: &[(&str, &str, &str)] = &[
    ("flac", "F000", ".flac"),
    ("320", "M800", ".mp3"),
    ("128", "M500", ".mp3"),
    ("aac", "C400", ".m4a"),
];

/// `result` codes the CDN returns per requested file.
const RESULT_OK: i64 = 0;
const RESULT_NO_PERMISSION: i64 = 104003;
const RESULT_VKEY_FAILED: i64 = 104004;
const RESULT_DEVICE_RESTRICTED: i64 = 104013;

/// The provenance the Python helper stamped on every enrichment payload. The
/// app reads these keys (and refuses a detail whose confidence is too low), so
/// they are part of the protocol, not decoration.
const SOURCE: &str = "qqmusic";

/// The profile a search has to run under.
///
/// Not a style choice: only the android envelope carries the device identity the
/// search endpoint wants, and every other profile answers `meta.sum = 0` — an
/// empty catalogue that looks exactly like "no such song". The detail reads
/// below reach for the search whenever they were given a name instead of a mid,
/// so they must ask under this profile even though the read itself does not.
const SEARCH_PLATFORM: Platform = Platform::Android;

/// `2026-09-30T12:34:56Z` — the shape `_utc_now_iso` produced, which the app's
/// date decoder accepts.
fn utc_now_iso() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = civil_from_unix(seconds as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days-since-epoch to a calendar date (Howard Hinnant's `civil_from_days`).
///
/// Written out rather than pulled in: the component exists to keep its
/// dependency surface small, and this is the one date it has to print.
fn civil_from_unix(seconds: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day, time / 3600, (time % 3600) / 60, time % 60)
}

/// The four provenance keys, merged into a payload.
fn with_provenance(mut payload: Value, confidence: f64) -> Value {
    if let Some(object) = payload.as_object_mut() {
        object.insert("source".into(), json!(SOURCE));
        object.insert("metadataSource".into(), json!(SOURCE));
        object.insert("metadataFetchedAt".into(), json!(utc_now_iso()));
        object.insert("metadataConfidence".into(), json!(confidence));
        object.insert("confidence".into(), json!(confidence));
    }
    payload
}

/// The top search hit's field, or an empty string.
fn candidate_text(candidate: &Value, key: &str) -> String {
    candidate
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A candidate's confidence, defaulting the way the Python helper did.
fn candidate_confidence(candidate: &Value, fallback: f64) -> f64 {
    candidate
        .get("confidence")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .unwrap_or(fallback)
}

fn require_login(credential: &Credential) -> Result<(), UpstreamError> {
    if credential.is_usable() {
        Ok(())
    } else {
        Err(UpstreamError::Upstream("需要登录后才能读取".into()))
    }
}

/// The `content[].value` strings of one `info` group (the upstream nests each
/// topic as `{title, type, content: [{value}, …]}`).
fn content_group(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|group| group.get("content"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| first_text(item, &["value"]))
                .collect()
        })
        .unwrap_or_default()
}

// MARK: - Song and album prose

/// What the caller knows about the song it wants prose for.
///
/// The app asks for this both from the online pages (a mid in hand) and from the
/// local library's enrichment (a title/artist/album off a file's tags and no mid
/// at all), so a name-only request resolves the mid through the search first —
/// exactly what the Python helper did, and the reason this method cannot simply
/// demand a mid.
#[derive(Debug, Clone, Default)]
pub struct SongDetailQuery<'a> {
    pub song_mid: Option<&'a str>,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    /// Sent by the app and ignored here, exactly as the Python helper ignored
    /// it: the catalogue's search does not rank by duration.
    #[allow(dead_code)]
    pub duration: Option<i64>,
}

/// The catalogue's own prose about a song, plus the facts the same request
/// carries (`info.intro`, `info.company`, `info.genre`, `info.lan`,
/// `info.pub_time`).
pub fn song_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    query: SongDetailQuery<'_>,
) -> Result<Value, UpstreamError> {
    let mut song_mid = query.song_mid.unwrap_or_default().trim().to_string();
    let mut matched_title = query.title.trim().to_string();
    let mut matched_artist = query.artist.trim().to_string();
    let mut matched_album = query.album.trim().to_string();
    let mut image_url = String::new();
    let mut album_mid = String::new();
    let mut confidence = 0.90;

    if song_mid.is_empty()
        && !(matched_title.is_empty() && matched_artist.is_empty() && matched_album.is_empty())
    {
        let candidates = search_track_artwork(
            upstream,
            credential,
            SEARCH_PLATFORM,
            &matched_title,
            &matched_artist,
            &matched_album,
            1,
        )?;
        if let Some(top) = candidates.first() {
            song_mid = candidate_text(top, "songMid");
            album_mid = candidate_text(top, "albumMid");
            let title = candidate_text(top, "title");
            let artist = candidate_text(top, "artist");
            let album = candidate_text(top, "album");
            if !title.is_empty() {
                matched_title = title;
            }
            if !artist.is_empty() {
                matched_artist = artist;
            }
            if !album.is_empty() {
                matched_album = album;
            }
            image_url = candidate_text(top, "imageURL");
            confidence = candidate_confidence(top, confidence);
        }
    }
    if song_mid.is_empty() {
        // Two different situations: nothing was given to search with, or the
        // search ran and matched nothing. The second one is a miss, not a bug,
        // and saying so saves the next person the same debugging round.
        return Err(UpstreamError::Upstream(
            if matched_title.is_empty() && matched_artist.is_empty() && matched_album.is_empty() {
                "songMid 或 title/artist/album 至少要有一个".into()
            } else {
                format!("找不到匹配的歌曲：{}", [matched_title.as_str(), matched_artist.as_str(), matched_album.as_str()].iter().filter(|part| !part.is_empty()).cloned().collect::<Vec<_>>().join(" "))
            },
        ));
    }

    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.pf_song_detail_svr",
            method: "get_song_detail_yqq",
            param: json!({ "song_mid": song_mid }),
        },
    )?;
    let info = first_object(&data, &["info"]).cloned().unwrap_or(json!({}));
    let track = first_object(&data, &["track_info", "track"]).cloned().unwrap_or(json!({}));
    let album = first_object(&track, &["album"]).cloned().unwrap_or(json!({}));
    if album_mid.is_empty() {
        album_mid = first_text(&album, &["mid", "albumMid"]).unwrap_or_default();
    }
    let release_date = content_group(&info, "pub_time")
        .first()
        .cloned()
        .or_else(|| first_text(&track, &["time_public", "timePublic"]));
    let cover_url = if !image_url.is_empty() {
        image_url.clone()
    } else if !album_mid.is_empty() {
        crate::methods::album_cover_url(&album_mid)
    } else {
        String::new()
    };
    let cover = crate::methods::normalized_artwork_url(
        Some(cover_url.as_str()).filter(|value| !value.is_empty()),
    );
    // The intro is a list of paragraphs; most songs have none at all, which is
    // an answer rather than a failure.
    let genres: Vec<String> = content_group(&info, "genre")
        .into_iter()
        .flat_map(split_tags)
        .collect();

    Ok(with_provenance(
        json!({
            "title": strip_highlight(first_text(&track, &["name", "title"]).unwrap_or(matched_title)),
            "artist": strip_highlight(first_text(&track, &["singer_name", "singer"]).unwrap_or(matched_artist)),
            "album": strip_highlight(first_text(&album, &["name", "title"]).unwrap_or(matched_album)),
            "songMid": song_mid,
            "albumMid": album_mid,
            "imageURL": cover,
            "description": content_group(&info, "intro").join("\n"),
            "genreTags": genres,
            "language": content_group(&info, "lan").first().cloned(),
            "labelOrCompany": content_group(&info, "company").first().cloned(),
            "releaseDate": release_date,
            "duration": first_int(&track, &["interval", "duration", "durationSec"]),
            // Kept for the online pages, which were reading these.
            "songId": first_int(&track, &["id", "songId"]),
        }),
        confidence,
    ))
}

/// What the caller knows about the album it wants prose for.
#[derive(Debug, Clone, Default)]
pub struct AlbumDetailQuery<'a> {
    pub album_mid: Option<&'a str>,
    pub album_id: Option<i64>,
    pub album: &'a str,
    pub artist: &'a str,
}

/// An album's own prose and metadata.
pub fn album_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    query: AlbumDetailQuery<'_>,
) -> Result<Value, UpstreamError> {
    let mut album_mid = query.album_mid.unwrap_or_default().trim().to_string();
    let mut matched_album = query.album.trim().to_string();
    let mut matched_artist = query.artist.trim().to_string();
    let mut image_url = String::new();
    let mut confidence = 0.90;

    // A local album has a title and an artist and no mid — resolve one.
    if album_mid.is_empty() && !(matched_album.is_empty() && matched_artist.is_empty()) {
        let candidates = search_album_artwork(
            upstream,
            credential,
            SEARCH_PLATFORM,
            &matched_album,
            &matched_artist,
            1,
        )?;
        if let Some(top) = candidates.first() {
            album_mid = candidate_text(top, "albumMid");
            let album = candidate_text(top, "album");
            let artist = candidate_text(top, "artist");
            if !album.is_empty() {
                matched_album = album;
            }
            if !artist.is_empty() {
                matched_artist = artist;
            }
            image_url = candidate_text(top, "imageURL");
            confidence = candidate_confidence(top, confidence);
        }
    }

    let mut param = serde_json::Map::new();
    match (album_mid.as_str(), query.album_id) {
        (mid, _) if !mid.is_empty() => {
            param.insert("albumMId".into(), json!(mid));
        }
        (_, Some(id)) => {
            param.insert("albumId".into(), json!(id));
        }
        _ => {
            return Err(UpstreamError::Upstream(
                if matched_album.is_empty() && matched_artist.is_empty() {
                    "需要 albumMid、albumId 或 album/artist".into()
                } else {
                    format!("找不到匹配的专辑：{} {}", matched_album, matched_artist)
                        .trim()
                        .to_string()
                },
            ))
        }
    }
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.musichallAlbum.AlbumInfoServer",
            method: "GetAlbumDetail",
            param: Value::Object(param),
        },
    )?;
    // `GetAlbumDetail` answers with the album under several possible names
    // depending on whether it was addressed by id or mid.
    let album = first_object(&data, &["basicInfo", "albumInfo", "album", "data"])
        .cloned()
        .unwrap_or(data.clone());
    if album_mid.is_empty() {
        album_mid = first_text(&album, &["albumMid", "albumMId", "mid"]).unwrap_or_default();
    }
    // The upstream usually gives a cover url; when it does not, the album mid
    // builds one (the same pattern every other cover uses).
    let cover = {
        let explicit = first_text(&album, &["pic", "cover", "coverURL"]);
        let built =
            (!album_mid.is_empty()).then(|| crate::methods::album_cover_url(&album_mid));
        crate::methods::normalized_artwork_url(
            explicit
                .or(built)
                .or(Some(image_url.clone()).filter(|value| !value.is_empty()))
                .as_deref(),
        )
    };
    let release_date = first_text(&album, &["publishDate", "time_public", "pubTime"]);

    Ok(with_provenance(
        json!({
            "album": strip_highlight(first_text(&album, &["albumName", "name", "title"]).unwrap_or(matched_album)),
            "artist": strip_highlight(first_text(&album, &["singerName", "singer_name", "artist"]).unwrap_or(matched_artist)),
            "albumMid": album_mid,
            "imageURL": cover,
            "description": first_text(&album, &["desc", "description", "intro"]),
            "releaseDate": release_date,
            "releaseYear": release_date.as_deref().and_then(release_year),
            "albumType": first_text(&album, &["album_type", "albumType"]),
            "genreTags": first_text(&album, &["genre", "genreName", "tag"])
                .map(split_tags)
                .unwrap_or_default(),
            "language": first_text(&album, &["lan", "language"]),
            "labelOrCompany": first_text(&album, &["company", "label"]),
            // Kept for the online pages, which were reading these.
            "id": first_int(&album, &["albumId", "id"]),
            "coverURL": cover,
            "title": strip_highlight(first_text(&album, &["albumName", "name", "title"]).unwrap_or_default()),
            "songCount": first_int(&album, &["songNum", "songnum", "totalNum"]),
        }),
        confidence,
    ))
}

/// The year out of a release date, however the upstream spells it.
fn release_year(date: &str) -> Option<i64> {
    let digits: String = date
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    (digits.len() >= 4)
        .then(|| digits[..4].parse().ok())
        .flatten()
}

/// The tracks of an album, addressed by mid or numeric id.
pub fn album_tracks(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    album_mid: Option<&str>,
    album_id: Option<i64>,
    offset: i64,
    limit: i64,
) -> Result<(Vec<Value>, Option<i64>), UpstreamError> {
    let mut param = serde_json::Map::new();
    match (album_mid, album_id) {
        (Some(mid), _) if !mid.trim().is_empty() => {
            param.insert("albumMid".into(), json!(mid));
        }
        (_, Some(id)) => {
            param.insert("albumId".into(), json!(id));
        }
        _ => return Err(UpstreamError::Upstream("需要 albumMid 或 albumId".into())),
    }
    param.insert("begin".into(), json!(offset));
    param.insert("num".into(), json!(limit));
    param.insert("order".into(), json!(0));
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.musichallAlbum.AlbumSongList",
            method: "GetAlbumSongList",
            param: Value::Object(param),
        },
    )?;
    // The response reports the album's size, so a caller can page through it and
    // can tell whether "select all" means all of it.
    let total = first_int(&data, &["totalNum", "total"]);
    Ok((crate::methods::decoded_tracks(&data), total))
}

// MARK: - An artist's works

/// An artist's songs. `sort` is `hot` (upstream order) or `latest`, which is
/// computed here: the upstream ignores its ordering parameter, so the only way
/// "最新" can mean anything is to sort the page by each track's album release
/// date.
pub fn artist_songs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    singer_mid: &str,
    sort: &str,
    page: i64,
    limit: i64,
) -> Result<Vec<Value>, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "musichall.song_list_server",
            method: "GetSingerSongList",
            param: json!({
                "singerMid": singer_mid,
                "order": 1,
                "number": limit,
                "begin": (page.max(1) - 1) * limit,
            }),
        },
    )?;
    let items = first_array(&data, &["songlist", "songList", "list"])
        .cloned()
        .unwrap_or_default();
    let mut tracks = crate::methods::decoded_tracks(&data);
    if sort.eq_ignore_ascii_case("latest") {
        // Attach each track's release date so the ordering is explainable, then
        // sort. Tracks with no date sink to the end rather than being dropped.
        for (track, item) in tracks.iter_mut().zip(items.iter()) {
            let album = first_object(item, &["album", "albumInfo"]);
            if let Some(date) = album.and_then(|album| first_text(album, &["time_public", "publishDate"])) {
                if let Some(object) = track.as_object_mut() {
                    object.insert("releaseDate".into(), json!(date));
                }
            }
        }
        tracks.sort_by(|a, b| {
            let key = |value: &Value| first_text(value, &["releaseDate"]).unwrap_or_default();
            key(b).cmp(&key(a))
        });
    }
    Ok(tracks)
}

/// An artist's albums, with the same two sorts.
pub fn artist_albums(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    singer_mid: &str,
    sort: &str,
    page: i64,
    limit: i64,
) -> Result<Vec<Value>, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.musichallAlbum.AlbumListServer",
            method: "GetAlbumList",
            param: json!({
                "singerMid": singer_mid,
                "order": 1,
                "number": limit,
                "begin": (page.max(1) - 1) * limit,
            }),
        },
    )?;
    let items = first_array(&data, &["albumList", "album_list", "list"])
        .cloned()
        .unwrap_or_default();
    let mut albums: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let mid = first_text(item, &["albumMid", "albumMid", "mid"]);
            // `albumID` with a capital ID is the upstream's own spelling here.
            let id = first_int(item, &["albumID", "albumId", "id"])?;
            Some(json!({
                "id": id,
                "title": first_text(item, &["albumName", "name", "title"]).unwrap_or_default(),
                "albumMid": mid,
                "coverURL": crate::methods::normalized_artwork_url(
                    mid.as_deref().map(crate::methods::album_cover_url).as_deref()
                ),
                "artist": first_text(item, &["singerName", "singer_name"]),
                "releaseDate": first_text(item, &["publishDate", "time_public"]),
            }))
        })
        .collect();
    if sort.eq_ignore_ascii_case("latest") {
        albums.sort_by(|a, b| {
            let key = |value: &Value| first_text(value, &["releaseDate"]).unwrap_or_default();
            key(b).cmp(&key(a))
        });
    }
    Ok(albums)
}

/// An artist's profile.
///
/// Read from the *homepage header* — `music.UnifiedHomepage.UnifiedHomepageSrv`
/// with `{"SingerMid": mid}` — because `GetSingerDetail` refuses both parameter
/// spellings it is documented with (10006, then 104400 with an empty list).
/// The header answers `data.Info.Singer` plus `data.Info.BaseInfo`, and it
/// reports a non-zero `code` while carrying usable data, so a code of its own is
/// not treated as failure here.
///
/// The biography is **not** in this response. `GetSingerDetail`'s wiki payload is
/// where the Python library reads it from, and that request still needs its
/// required parameters worked out (see `docs/endpoints.md`), so `description`
/// comes back empty until then rather than pretending to have been read.
pub fn artist_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    name: &str,
    singer_mid: Option<&str>,
) -> Result<Value, UpstreamError> {
    let mut mid = singer_mid.unwrap_or_default().trim().to_string();
    let mut matched_name = name.trim().to_string();
    let mut matched_region = String::new();
    let mut matched_foreign_name = String::new();
    let mut matched_genres: Vec<String> = Vec::new();
    let mut image_url = String::new();
    let mut confidence = 0.90;

    // The local library knows an artist by name; the header needs a mid.
    if mid.is_empty() && !matched_name.is_empty() {
        let candidates =
            search_artist_artwork(upstream, credential, SEARCH_PLATFORM, &matched_name, 1)?;
        if let Some(top) = candidates.first() {
            mid = candidate_text(top, "singerMid");
            let name = candidate_text(top, "artistName");
            if !name.is_empty() {
                matched_name = name;
            }
            matched_region = candidate_text(top, "region");
            matched_foreign_name = candidate_text(top, "foreignName");
            matched_genres = top
                .get("genreTags")
                .and_then(Value::as_array)
                .map(|tags| {
                    tags.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            image_url = candidate_text(top, "imageURL");
            confidence = candidate_confidence(top, confidence);
        }
    }
    if mid.is_empty() {
        return Err(UpstreamError::Upstream("singerMid 或 name 至少要有一个".into()));
    }

    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.UnifiedHomepage.UnifiedHomepageSrv",
            method: "GetHomepageHeader",
            param: json!({ "SingerMid": mid }),
        },
    )?;
    let info = first_object(&data, &["Info", "info"]).cloned().unwrap_or(json!({}));
    let singer = first_object(&info, &["Singer", "singer"]).cloned().unwrap_or(json!({}));
    let base = first_object(&info, &["BaseInfo", "baseInfo"]).cloned().unwrap_or(json!({}));
    // The header can answer `10000` with a *shaped but empty* singer (every
    // field blank, only the stats filled in). Returning that as a profile would
    // put 未知歌手 on screen for an artist that exists, so it is reported as the
    // missing-device-identity case it looks like (see `docs/parsing.md`).
    let header_name = first_text(&singer, &["Name", "name"]);
    if header_name.is_none() && first_text(&singer, &["SingerMid", "mid"]).is_none() {
        return Err(UpstreamError::Upstream(
            "上游返回了空的歌手资料（与搜索、推荐同因：可能缺设备标识）".into(),
        ));
    }
    let cover = {
        let explicit = first_text(&singer, &["SingerPic", "Pic", "pic"]);
        let built = first_text(&singer, &["SingerMid", "Mid", "mid"])
            .map(|mid| crate::methods::singer_cover_url(&mid));
        crate::methods::normalized_artwork_url(
            explicit
                .or(built)
                .or(Some(image_url.clone()).filter(|value| !value.is_empty()))
                .as_deref(),
        )
    };
    // The upstream wraps matched words in `<em>` on some routes; a name is shown,
    // not parsed, so the markup is stripped here rather than on screen.
    let artist_name = strip_highlight(header_name.unwrap_or(matched_name));
    let description = first_text(&singer, &["Desc", "desc", "Description"]).unwrap_or_default();
    let foreign_name = first_text(&singer, &["ForeignName", "foreign_name", "OtherName"])
        .or(Some(matched_foreign_name).filter(|value| !value.is_empty()));
    let region = first_text(&info, &["IP", "Country", "country", "Area", "area"])
        .or(Some(matched_region).filter(|value| !value.is_empty()));
    let genres = first_text(&base, &["Genre", "genre"])
        .map(split_tags)
        .filter(|tags| !tags.is_empty())
        .unwrap_or(matched_genres);

    Ok(with_provenance(
        json!({
            // The key the app's enrichment decodes…
            "artistName": artist_name,
            "singerMid": mid,
            "imageURL": cover,
            "description": description,
            "genreTags": genres,
            "region": region,
            "foreignName": foreign_name,
            // …and the ones the online artist page was reading.
            "name": artist_name,
            "coverURL": cover,
            "genre": genres,
            "songCount": first_int(&singer, &["SongCount", "songCount", "SongNum"])
                .or_else(|| first_int(&base, &["SongCount", "songNum"])),
            "albumCount": first_int(&singer, &["AlbumCount", "albumCount", "AlbumNum"]),
            // The header keeps the counts at the top of `Info`, not inside `Singer`.
            "fanCount": first_int(&info, &["FansNum", "Fans", "fans"])
                .or_else(|| first_int(&singer, &["Fans", "fans"])),
            "followCount": first_int(&info, &["FollowNum"]),
        }),
        confidence,
    ))
}

fn split_tags(value: String) -> Vec<String> {
    value
        .split([';', ',', '/', '|'])
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect()
}

// MARK: - Rankings, radio, new songs, recommendations

/// The ranking groups, each with the rankings it contains.
pub fn toplist_categories(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
) -> Result<Vec<Value>, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.musicToplist.Toplist",
            method: "GetAll",
            param: json!({}),
        },
    )?;
    let groups = first_array(&data, &["topList", "group", "groups", "list"])
        .cloned()
        .unwrap_or_default();
    Ok(groups
        .iter()
        .filter_map(|group| {
            let title = first_text(group, &["title", "groupName", "name"]).unwrap_or_default();
            // The group's own key is `toplist`, lower-case l.
            let toplists: Vec<Value> = first_array(group, &["toplist", "topList", "list", "topLists"])
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|item| {
                    let id = first_int(item, &["topId", "id", "toplistId"])?;
                    let name = first_text(item, &["topTitle", "title", "name"]).unwrap_or_default();
                    Some(json!({
                        "id": id,
                        // `name` is the key the app decodes, and it is not
                        // optional there; `title` stays because the online pages
                        // read it.
                        "name": name,
                        "title": name,
                        "source": "qqmusic",
                        "coverURL": crate::methods::normalized_artwork_url(
                            first_text(item, &["picUrl", "cover", "coverURL"]).as_deref()
                        ),
                        "updateTime": first_text(item, &["updateTime", "update_time"]),
                    }))
                })
                .collect();
            if toplists.is_empty() {
                return None;
            }
            Some(json!({
                "id": first_int(group, &["id", "groupId", "topId"]),
                "name": title,
                "title": title,
                "source": "qqmusic",
                "toplists": toplists,
            }))
        })
        .collect())
}

/// One ranking's tracks.
///
/// The list is under `songInfoList` — **not** `data.data.song`, which holds the
/// display rows (rank/title/cover) and no song mid at all. The total is
/// `totalNum`.
pub fn toplist_tracks(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    top_id: i64,
    offset: i64,
    limit: i64,
) -> Result<(Vec<Value>, Option<i64>), UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.musicToplist.Toplist",
            method: "GetDetail",
            param: json!({ "topId": top_id, "offset": offset, "num": limit }),
        },
    )?;
    // `totalNum` is how many the ranking holds (300 for the big ones), so the
    // caller knows whether another page exists instead of guessing from the
    // page it just received.
    let total = first_int(&data, &["totalNum", "total"])
        .or_else(|| first_object(&data, &["data"]).and_then(|inner| first_int(inner, &["totalNum", "total"])));
    Ok((crate::methods::decoded_tracks(&data), total))
}

/// The radio groups, each with its stations.
pub fn radio_stations(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
) -> Result<Vec<Value>, UpstreamError> {
    let uin = if credential.music_id.is_empty() { "0".to_string() } else { credential.music_id.clone() };
    let data = upstream.call_with(
        credential,
        crate::Class::Interactive,
        platform,
        Call {
            module: "pf.radiosvr",
            method: "GetRadiolist",
            param: json!({ "uin": uin }),
        },
    )?;
    let groups = first_array(&data, &["radio_list", "list", "groups"])
        .cloned()
        .unwrap_or_default();
    Ok(groups
        .iter()
        .filter_map(|group| {
            let stations: Vec<Value> = first_array(group, &["list", "radios"])
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|item| {
                    let id = first_int(item, &["id"])?;
                    Some(json!({
                        "id": id,
                        "title": first_text(item, &["title", "name"]).unwrap_or_default(),
                        "coverURL": crate::methods::normalized_artwork_url(
                            first_text(item, &["pic_url", "picUrl"]).as_deref()
                        ),
                        "listenerCount": first_int(item, &["listener_count", "listenerCount"]),
                        "source": "qqmusic",
                    }))
                })
                .collect();
            if stations.is_empty() {
                return None;
            }
            let title = first_text(group, &["title", "name"]).unwrap_or_default();
            Some(json!({
                "id": first_int(group, &["id", "groupId", "radioId"]),
                "name": title,
                "title": title,
                "source": "qqmusic",
                "stations": stations,
            }))
        })
        .collect())
}

/// A station's next songs. A station is endless: the same request answers with a
/// fresh rotation each time.
pub fn radio_tracks(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    station_id: i64,
    limit: i64,
    first_play: bool,
) -> Result<Vec<Value>, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Interactive,
        platform,
        Call {
            module: "pf.radiosvr",
            method: "GetRadiosonglist",
            param: json!({
                "id": station_id,
                "firstplay": if first_play { 1 } else { 0 },
                "num": limit,
            }),
        },
    )?;
    Ok(crate::methods::decoded_tracks(&data))
}

/// New songs for a region.
///
/// The region code is the upstream's own `type`: 0 最新, 1 内地, 2 港台, 3 欧美,
/// 4 日本, 5 韩国.
pub fn new_songs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    region_type: i64,
) -> Result<Vec<Value>, UpstreamError> {
    // The upstream's own `type`: 0 最新, 1 内地, 2 港台, 3 欧美, 4 日本, 5 韩国.
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "newsong.NewSongServer",
            method: "get_new_song_info",
            param: json!({ "type": region_type }),
        },
    )?;
    Ok(crate::methods::decoded_tracks(&data))
}

/// "Guess you like" — the recommendation the upstream returns for this account.
pub fn recommend_feed(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
) -> Result<Vec<Value>, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Interactive,
        platform,
        Call {
            module: "music.radioProxy.MbTrackRadioSvr",
            method: "get_radio_track",
            param: json!({ "id": 99, "num": 5, "from": 0, "scene": 0, "song_ids": [] }),
        },
    )?;
    Ok(crate::methods::decoded_tracks(&data))
}

// MARK: - Lyrics

/// A lyric, in plaintext.
///
/// Read through the legacy `c.y.qq.com` route with `nobase64=1`, which answers
/// readable LRC directly. The `musicu.fcg` route
/// (`music.musichallSong.PlayLyricInfo`) is **not** used: its payload is a
/// custom Triple-DES variant, not the base64 plaintext its `crypt: 0` echo
/// suggests, and porting that cipher is a liability when a plaintext route
/// exists. The word-level (`qrc`) track is only available on the encrypted
/// route, so `word_lyric` comes back empty — `docs/parsing.md` records this.
pub fn lyric(
    upstream: &Upstream,
    credential: &Credential,
    _platform: Platform,
    song_mid: &str,
    _song_id: Option<i64>,
    _with_word_timing: bool,
    with_translation: bool,
) -> Result<Value, UpstreamError> {
    if song_mid.trim().is_empty() {
        return Err(UpstreamError::Upstream("缺少 songMid".into()));
    }
    let url = format!(
        "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={song_mid}&g_tk={gtk}\
&format=json&inCharset=utf8&outCharset=utf-8&nobase64=1&platform=yqq.json&needNewCode=1",
        gtk = credential.g_tk()
    );
    let data = upstream.get_fcgi(credential, crate::Class::Playback, &url)?;
    let text = |key: &str| -> Option<String> {
        first_text(&data, &[key])
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    Ok(json!({
        "lyric": text("lyric"),
        "translation": if with_translation { text("trans") } else { None },
        "romanization": Option::<String>::None,
        "wordLyric": Option::<String>::None,
    }))
}

// MARK: - Playback url

/// Resolve a playable url for one track, probing the quality ladder.
///
/// The filename is built from the *media* mid when the track has one — the
/// upstream's own naming — and from the song mid twice otherwise (the older
/// shape). The answer's `midurlinfo[].result` says whether the account may have
/// this file: 0 granted, 104003 no permission, 104004 no ticket, 104013 device
/// restricted.
pub fn stream_url(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    song_mid: &str,
    media_mid: Option<&str>,
    song_type: i64,
    preferred_quality: Option<&str>,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    if song_mid.trim().is_empty() {
        return Err(UpstreamError::Upstream("缺少 songMid".into()));
    }
    let ladder = preferred_quality
        .and_then(|wanted| {
            QUALITY_LADDER
                .iter()
                .find(|(label, _, _)| label.eq_ignore_ascii_case(wanted))
        })
        .map(|entry| std::slice::from_ref(entry))
        .unwrap_or(QUALITY_LADDER);

    let mut refusals: Vec<String> = Vec::new();
    let mut tried: Vec<String> = Vec::new();
    for (label, prefix, extension) in ladder.iter().copied() {
        let filename = match media_mid.filter(|mid| !mid.trim().is_empty()) {
            Some(mid) => format!("{prefix}{mid}{extension}"),
            None => format!("{prefix}{song_mid}{song_mid}{extension}"),
        };
        let data = upstream.call_with(
            credential,
            crate::Class::Playback,
            platform,
            Call {
                module: "music.vkey.GetVkey",
                method: "UrlGetVkey",
                param: json!({
                    "uin": credential.music_id,
                    "filename": [filename],
                    "guid": guid(),
                    "songmid": [song_mid],
                    "songtype": [song_type],
                    "ctx": 0,
                }),
            },
        )?;
        let info = first_array(&data, &["midurlinfo", "data"])
            .and_then(|list| list.first())
            .cloned()
            .unwrap_or(json!({}));
        let result = first_int(&info, &["result"]).unwrap_or(RESULT_VKEY_FAILED);
        let purl = first_text(&info, &["purl", "url"]).unwrap_or_default();
        if result == RESULT_OK && !purl.is_empty() {
            return Ok(json!({
                "source": SOURCE,
                "songMid": song_mid,
                "mediaMid": media_mid.unwrap_or_default(),
                "quality": label,
                "extension": extension.trim_start_matches('.'),
                "filename": filename,
                "url": if purl.starts_with("http") { purl.clone() } else { format!("{STREAM_CDN}{}", purl.trim_start_matches('/')) },
                "expiration": first_int(&data, &["expiration"]).unwrap_or(7200),
                "playable": true,
                "tried": tried,
            }));
        }
        tried.push(format!("{label}:{result}"));
        refusals.push(format!("{label}:{}", describe_result(result)));
    }
    // Not playable is an answer, not a failure: the app reads `restriction` for
    // the reason and `tried` for the log, and treats a missing url as "no".
    Ok(json!({
        "source": SOURCE,
        "songMid": song_mid,
        "mediaMid": media_mid.unwrap_or_default(),
        "url": "",
        "quality": "",
        "playable": false,
        "restriction": classify_restriction(&tried),
        "tried": tried,
        "reason": format!("上游没有授予任何可用音质（{}）", refusals.join("、")),
    }))
}

/// Map the per-tier result codes onto the one word the app shows.
fn classify_restriction(tried: &[String]) -> &'static str {
    let codes: Vec<&str> = tried
        .iter()
        .filter_map(|entry| entry.split(':').nth(1))
        .collect();
    if codes.is_empty() {
        return "unavailable";
    }
    if codes
        .iter()
        .all(|code| *code == RESULT_NO_PERMISSION.to_string())
    {
        "paid_required"
    } else if codes
        .iter()
        .all(|code| *code == RESULT_DEVICE_RESTRICTED.to_string())
    {
        "device_restricted"
    } else if codes.iter().all(|code| *code == RESULT_VKEY_FAILED.to_string()) {
        "ticket_required"
    } else {
        "unavailable"
    }
}

fn describe_result(code: i64) -> String {
    match code {
        RESULT_OK => "已授权但地址为空".into(),
        RESULT_NO_PERMISSION => "无权限（可能需要会员）".into(),
        RESULT_VKEY_FAILED => "取票失败".into(),
        RESULT_DEVICE_RESTRICTED => "设备受限".into(),
        other => format!("错误码 {other}"),
    }
}

/// `guid` is the upstream's per-request device id; a random 10-digit number is
/// what the library sends and what the CDN accepts.
fn guid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("{:010}", (nanos % 9_000_000_000) + 1_000_000_000)
}

// MARK: - Helpers

/// Decode one base64 text field, treating anything unusable as absent.
///
/// Written out rather than pulled in as a dependency: it is twenty lines, and
/// the component exists to keep its dependency surface small.
fn decode_base64_text(value: Option<&Value>) -> Option<String> {
    let encoded = value?.as_str()?.trim();
    if encoded.is_empty() {
        return None;
    }
    let mut buffer: Vec<u8> = Vec::with_capacity(encoded.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits = 0u32;
    for byte in encoded.bytes() {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue, // newlines and stray whitespace
        } as u32;
        accumulator = (accumulator << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buffer.push((accumulator >> bits) as u8);
        }
    }
    let text = String::from_utf8(buffer).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_fields_decode_and_blanks_are_absent() {
        // "你好" in base64.
        assert_eq!(
            decode_base64_text(Some(&json!("5L2g5aW9"))).as_deref(),
            Some("你好")
        );
        assert!(decode_base64_text(Some(&json!(""))).is_none());
        assert!(decode_base64_text(None).is_none());
    }

    #[test]
    fn the_quality_ladder_is_ordered_best_first() {
        assert_eq!(QUALITY_LADDER[0].0, "flac");
        assert_eq!(QUALITY_LADDER.last().unwrap().0, "aac");
        let mut labels: Vec<&str> = QUALITY_LADDER.iter().map(|entry| entry.0).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), QUALITY_LADDER.len(), "no duplicates");
    }

    #[test]
    fn search_highlight_markup_is_stripped() {
        assert_eq!(strip_highlight("百听不厌的<em>周杰伦</em>".to_string()), "百听不厌的周杰伦");
        assert_eq!(strip_highlight("无标记".to_string()), "无标记");
    }

    #[test]
    fn tag_lists_split_on_any_of_the_upstreams_separators() {
        assert_eq!(split_tags("Rock; Pop/Blues".into()), vec!["Rock", "Pop", "Blues"]);
        assert!(split_tags("  ".into()).is_empty());
    }

    #[test]
    fn a_song_detail_without_prose_is_still_an_answer() {
        let info = json!({"company": {"content": [{"value": "某唱片"}]}});
        assert!(content_group(&info, "intro").is_empty());
        assert_eq!(content_group(&info, "company"), vec!["某唱片"]);
    }
}

/// Add to, or remove from, "我喜欢".
///
/// The only write this component performs. It takes the song's *numeric* id
/// (which every track payload carries) and its type; a caller holding only a
/// mid resolves the id through [`song_detail`] first.
pub fn set_liked(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    song_id: i64,
    song_mid: Option<&str>,
    song_type: i64,
    liked: bool,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    // The host knows a track by its mid — that is what every list row and the
    // player carry — while this endpoint addresses songs by their *numeric* id.
    // Resolving it here rather than at each call site is what keeps every like
    // button in the app working from the same one-line call.
    let song_id = if song_id > 0 {
        song_id
    } else if let Some(mid) = song_mid.filter(|mid| !mid.trim().is_empty()) {
        let detail = song_detail(
            upstream,
            credential,
            platform,
            SongDetailQuery {
                song_mid: Some(mid),
                ..Default::default()
            },
        )?;
        detail
            .get("songId")
            .and_then(Value::as_i64)
            .filter(|id| *id > 0)
            .ok_or_else(|| UpstreamError::Upstream(format!("找不到这首歌的数字 id：{mid}")))?
    } else {
        return Err(UpstreamError::Upstream("需要 songId 或 songMid".into()));
    };
    upstream.call_with(
        credential,
        crate::Class::Write,
        platform,
        Call {
            module: "music.musicasset.PlaylistDetailWrite",
            method: if liked { "AddSonglist" } else { "DelSonglist" },
            param: json!({
                "dirId": 201,
                "tid": 0,
                "bFmtUtf8": true,
                "v_songInfo": [{ "songId": song_id, "songType": song_type }],
            }),
        },
    )?;
    Ok(json!({ "songId": song_id, "liked": liked }))
}

// MARK: - Search

/// Which kind of content a search asks for.
///
/// The numbers are the upstream's own `search_type`, and each kind answers under
/// a different key in `body` — the reason this is one function with a mapping
/// rather than four near-identical ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Songs,
    Artists,
    Albums,
    Playlists,
}

impl SearchKind {
    fn search_type(self) -> i64 {
        match self {
            SearchKind::Songs => 0,
            SearchKind::Artists => 1,
            SearchKind::Albums => 2,
            SearchKind::Playlists => 3,
        }
    }

    fn body_key(self) -> &'static str {
        match self {
            SearchKind::Songs => "item_song",
            SearchKind::Artists => "singer",
            SearchKind::Albums => "item_album",
            SearchKind::Playlists => "item_songlist",
        }
    }

    /// What the results are called in the payload this returns.
    pub fn payload_key(self) -> &'static str {
        match self {
            SearchKind::Songs => "tracks",
            SearchKind::Artists => "artists",
            SearchKind::Albums => "albums",
            SearchKind::Playlists => "playlists",
        }
    }
}

/// Search the catalogue. Needs the android profile *and* the device session: it
/// answers `meta.sum = 0` for everything without them.
pub fn search(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    kind: SearchKind,
    keyword: &str,
    page: i64,
    limit: i64,
) -> Result<Value, UpstreamError> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(json!({ kind.payload_key(): [], "total": 0 }));
    }
    let data = upstream.call_with(
        credential,
        crate::Class::Interactive,
        platform,
        Call {
            module: "music.search.SearchCgiService",
            method: "DoSearchForQQMusicMobile",
            param: json!({
                "searchid": search_id(),
                "query": keyword,
                "search_type": kind.search_type(),
                "num_per_page": limit,
                "page_num": page.max(1),
                "highlight": false,
                "grp": true,
                "selectors": {},
                "vec_selectors": [],
            }),
        },
    )?;
    let body = first_object(&data, &["body"]).cloned().unwrap_or(json!({}));
    let items = first_array(&body, &[kind.body_key()])
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        // Zero results and "the upstream refused to search" look identical in
        // the mapped payload, so the raw answer is worth having in the log when
        // someone asks why a title found nothing. stderr only: stdout is the
        // protocol.
        let raw = serde_json::to_string(&data).unwrap_or_default();
        eprintln!(
            "[qqmusic-helper-next] search returned nothing keyword={keyword} raw={}",
            &raw[..raw.len().min(400)]
        );
    }
    let total = first_object(&data, &["meta"])
        .and_then(|meta| first_int(meta, &["sum", "total"]))
        .unwrap_or(items.len() as i64);

    let mapped: Vec<Value> = match kind {
        SearchKind::Songs => crate::methods::decoded_tracks(&json!({ "list": items })),
        SearchKind::Artists => items.iter().filter_map(map_artist).collect(),
        SearchKind::Albums => items.iter().filter_map(map_album).collect(),
        SearchKind::Playlists => items.iter().filter_map(map_playlist).collect(),
    };
    Ok(json!({ kind.payload_key(): mapped, "total": total }))
}

/// The search call wants a session id; the upstream does not validate its shape,
/// only that it looks like one.
fn search_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!("{:019}", rng.gen_range(1_000_000_000_000_000_000u64..9_999_999_999_999_999_999))
}

fn map_artist(item: &Value) -> Option<Value> {
    let mid = first_text(item, &["singerMID", "singerMid", "mid", "MID"])?;
    Some(json!({
        "singerMid": mid,
        "name": strip_highlight(
            first_text(item, &["singerName", "name", "Name"]).unwrap_or_else(|| "未知歌手".into())
        ),
        "coverURL": crate::methods::normalized_artwork_url(
            first_text(item, &["singerPic", "pic", "avatarUrl"]).as_deref()
        ),
        "songCount": first_int(item, &["songNum", "songnum"]),
        "albumCount": first_int(item, &["albumNum", "albumnum"]),
        "fanCount": first_int(item, &["fansNum", "fanNum", "fans"]),
    }))
}

fn map_album(item: &Value) -> Option<Value> {
    let mid = first_text(item, &["albumMID", "albumMid", "albummid", "mid"]);
    let id = first_int(item, &["albumID", "albumId", "id"])?;
    Some(json!({
        "id": id,
        "title": strip_highlight(
            first_text(item, &["albumName", "albumname", "name", "title"]).unwrap_or_default()
        ),
        "albumMid": mid,
        "coverURL": crate::methods::normalized_artwork_url(
            first_text(item, &["pic", "coverURL"])
                .or_else(|| mid.clone().map(|mid| crate::methods::album_cover_url(&mid)))
                .as_deref()
        ),
        "artist": first_text(item, &["singerName", "singername", "singer"]),
        "releaseDate": first_text(item, &["publish_date", "publishDate", "time_public"]),
    }))
}

/// Search results wrap the matched words in `<em>` even with `highlight: false`
/// on some of the playlist routes, so the markup is stripped here (the library
/// does the same before showing a title).
fn strip_highlight(value: String) -> String {
    value.replace("<em>", "").replace("</em>", "")
}

fn map_playlist(item: &Value) -> Option<Value> {
    let id = first_int(item, &["dissid", "dissId", "id"])?;
    Some(json!({
        "id": id,
        "title": strip_highlight(
            first_text(item, &["dissname", "dissName", "title", "name"]).unwrap_or_default()
        ),
        // `logo` is where this route puts the cover (verified against a live
        // reply: `imgurl`/`picurl`/`cover` are all absent here), and it arrives as
        // `http://qpic.y.qq.com/...` — normalisation is what makes it load at all.
        "coverURL": crate::methods::normalized_artwork_url(
            first_text(item, &["logo", "imgurl", "picurl", "cover"]).as_deref()
        ),
        "creator": first_text(item, &["creator", "nickname", "nick"]),
        "songCount": first_int(item, &["songnum", "songCount"]),
        "playCount": first_int(item, &["listennum", "playCount"]),
    }))
}

// MARK: - Artwork and biography matching (the local library's enrichment)

/// The confidence the Python helper assigned by rank, kept identical so the
/// app's own scoring sees the same numbers it always has.
fn rank_confidence(index: usize) -> f64 {
    (0.86 - index as f64 * 0.04).max(0.50)
}

/// Candidate covers for a local track: the catalogue is searched for the title,
/// artist and album together, and each hit carries its cover.
///
/// The *app* decides which candidate wins (it compares titles, artists and
/// durations itself); this only has to offer the right rows with a rank hint.
pub fn search_track_artwork(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    title: &str,
    artist: &str,
    album: &str,
    limit: i64,
) -> Result<Vec<Value>, UpstreamError> {
    let query = [title, artist, album]
        .iter()
        .filter(|part| !part.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let result = search(upstream, credential, platform, SearchKind::Songs, &query, 1, limit.clamp(1, 10))?;
    let rows = result.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(rows
        .iter()
        .enumerate()
        .map(|(index, track)| {
            json!({
                "source": "qqmusic",
                "title": track.get("title"),
                "artist": track.get("artist"),
                "album": track.get("album"),
                "songMid": track.get("songMid"),
                "albumMid": track.get("albumMid"),
                "imageURL": track.get("imageURL"),
                "duration": track.get("duration"),
                "confidence": rank_confidence(index),
            })
        })
        .collect())
}

/// Candidate covers for a local artist.
pub fn search_artist_artwork(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    name: &str,
    limit: i64,
) -> Result<Vec<Value>, UpstreamError> {
    if name.trim().is_empty() {
        return Ok(Vec::new());
    }
    let result = search(upstream, credential, platform, SearchKind::Artists, name, 1, limit.clamp(1, 10))?;
    let rows = result.get("artists").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(rows
        .iter()
        .enumerate()
        .map(|(index, artist)| {
            json!({
                "source": "qqmusic",
                "artistName": artist.get("name"),
                "singerMid": artist.get("singerMid"),
                "imageURL": artist.get("coverURL"),
                "genreTags": [],
                "region": artist.get("region"),
                "confidence": rank_confidence(index),
            })
        })
        .collect())
}

/// Candidate covers for a local album.
pub fn search_album_artwork(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    album: &str,
    artist: &str,
    limit: i64,
) -> Result<Vec<Value>, UpstreamError> {
    let query = [album, artist]
        .iter()
        .filter(|part| !part.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let result = search(upstream, credential, platform, SearchKind::Albums, &query, 1, limit.clamp(1, 10))?;
    let rows = result.get("albums").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(rows
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            json!({
                "source": "qqmusic",
                "album": entry.get("title"),
                "artist": entry.get("artist"),
                "albumMid": entry.get("albumMid"),
                "imageURL": entry.get("coverURL"),
                "releaseDate": entry.get("releaseDate"),
                "confidence": rank_confidence(index),
            })
        })
        .collect())
}

/// An artist's biography, for the local library's enrichment.
///
/// Read through the homepage header, which is where the name and portrait live;
/// the prose comes back empty until that endpoint's wiki payload is worked out
/// (`docs/endpoints.md` §3), and an empty description here means "the catalogue
/// has none", which the app already handles.
pub fn artist_biography(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    name: &str,
    singer_mid: Option<&str>,
) -> Result<Value, UpstreamError> {
    // `artist_detail` owns the name → mid resolution (a biography is asked for by
    // name when the local library has nothing else), so this is the same read
    // under a payload shaped for the artist page.
    let detail = artist_detail(upstream, credential, platform, name, singer_mid)?;
    let take = |key: &str| detail.get(key).cloned().unwrap_or(Value::Null);
    Ok(json!({
        "source": SOURCE,
        "artistName": take("artistName"),
        "singerMid": take("singerMid"),
        "description": take("description"),
        "imageURL": take("imageURL"),
        "genreTags": take("genreTags"),
        "region": take("region"),
        "foreignName": take("foreignName"),
        "metadataSource": SOURCE,
        "metadataFetchedAt": utc_now_iso(),
        "metadataConfidence": detail.get("confidence").cloned().unwrap_or(json!(0.9)),
        "confidence": detail.get("confidence").cloned().unwrap_or(json!(0.9)),
    }))
}

#[cfg(test)]
mod artwork_tests {
    use super::*;

    #[test]
    fn rank_confidence_starts_high_and_never_falls_past_half() {
        assert!((rank_confidence(0) - 0.86).abs() < 1e-9);
        assert!((rank_confidence(1) - 0.82).abs() < 1e-9);
        assert!((rank_confidence(50) - 0.50).abs() < 1e-9);
    }

    #[test]
    fn the_timestamp_is_the_shape_the_app_decodes() {
        let stamp = utc_now_iso();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
        // Known instants, so the calendar arithmetic is pinned and not merely
        // "something that looks like a date".
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_unix(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        assert_eq!(civil_from_unix(1_782_000_000), (2026, 6, 21, 0, 0, 0));
    }

    #[test]
    fn a_release_year_is_read_however_the_date_is_spelled() {
        assert_eq!(release_year("2003-07-31"), Some(2003));
        assert_eq!(release_year("2003"), Some(2003));
        assert_eq!(release_year("发行于 2003 年"), Some(2003));
        assert_eq!(release_year(""), None);
        assert_eq!(release_year("未知"), None);
    }

    #[test]
    fn restriction_says_what_the_account_lacks() {
        assert_eq!(
            classify_restriction(&["flac:104003".into(), "128:104003".into()]),
            "paid_required"
        );
        assert_eq!(classify_restriction(&["flac:104013".into()]), "device_restricted");
        assert_eq!(classify_restriction(&["flac:104004".into()]), "ticket_required");
        assert_eq!(classify_restriction(&["flac:104003".into(), "128:0".into()]), "unavailable");
        assert_eq!(classify_restriction(&[]), "unavailable");
    }

    #[test]
    fn enrichment_payloads_carry_what_the_app_reads() {
        let payload = with_provenance(json!({ "title": "晴天" }), 0.82);
        assert_eq!(payload["source"], "qqmusic");
        assert_eq!(payload["metadataSource"], "qqmusic");
        assert_eq!(payload["confidence"], 0.82);
        assert_eq!(payload["metadataConfidence"], 0.82);
        assert!(payload["metadataFetchedAt"].as_str().unwrap().ends_with('Z'));
    }
}

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

/// The catalogue's own prose about a song, plus the facts the same request
/// carries (`info.intro`, `info.company`, `info.genre`, `info.lan`,
/// `info.pub_time`).
pub fn song_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    song_mid: &str,
) -> Result<Value, UpstreamError> {
    if song_mid.trim().is_empty() {
        return Err(UpstreamError::Upstream("缺少 songMid".into()));
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
    let track = first_object(&data, &["track_info"]).cloned().unwrap_or(json!({}));
    let album = first_object(&track, &["album"]).cloned().unwrap_or(json!({}));
    Ok(json!({
        "songMid": song_mid,
        "songId": first_int(&track, &["id", "songId"]),
        "title": first_text(&track, &["name", "title"]),
        "artist": first_text(&track, &["singer_name"]),
        "album": first_text(&album, &["name"]),
        "albumMid": first_text(&album, &["mid"]),
        // The intro is a list of paragraphs; most songs have none at all, which
        // is an answer rather than a failure.
        "description": content_group(&info, "intro").join("\n"),
        "genre": content_group(&info, "genre"),
        "language": content_group(&info, "lan").first().cloned(),
        "company": content_group(&info, "company").first().cloned(),
        "releaseDate": content_group(&info, "pub_time").first().cloned(),
        "duration": first_int(&track, &["interval"]),
    }))
}

/// An album's own prose and metadata.
pub fn album_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    album_mid: Option<&str>,
    album_id: Option<i64>,
) -> Result<Value, UpstreamError> {
    let mut param = serde_json::Map::new();
    match (album_mid, album_id) {
        (Some(mid), _) if !mid.trim().is_empty() => {
            param.insert("albumMId".into(), json!(mid));
        }
        (_, Some(id)) => {
            param.insert("albumId".into(), json!(id));
        }
        _ => return Err(UpstreamError::Upstream("需要 albumMid 或 albumId".into())),
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
    // The upstream usually gives a cover url; when it does not, the album mid
    // builds one (the same pattern every other cover uses).
    let cover = {
        let explicit = first_text(&album, &["pic", "cover", "coverURL"]);
        let built = first_text(&album, &["albumMid", "albumMId", "mid"])
            .map(|mid| crate::methods::album_cover_url(&mid));
        crate::methods::normalized_artwork_url(explicit.or(built).as_deref())
    };
    Ok(json!({
        "id": first_int(&album, &["albumId", "id"]),
        "albumMid": first_text(&album, &["albumMid", "albumMId", "mid"]),
        "title": first_text(&album, &["albumName", "name", "title"]),
        "artist": first_text(&album, &["singerName", "singer_name", "artist"]),
        "coverURL": cover,
        "description": first_text(&album, &["desc", "description", "intro"]),
        "releaseDate": first_text(&album, &["publishDate", "time_public", "pubTime"]),
        "genre": first_text(&album, &["genre", "genreName"]),
        "language": first_text(&album, &["lan", "language"]),
        "company": first_text(&album, &["company", "label"]),
        "songCount": first_int(&album, &["songNum", "songnum", "totalNum"]),
    }))
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
) -> Result<Vec<Value>, UpstreamError> {
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
    Ok(crate::methods::decoded_tracks(&data))
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
            let id = first_int(item, &["albumId", "id"])?;
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

/// An artist's profile and biography.
pub fn artist_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    singer_mid: &str,
) -> Result<Value, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Read,
        platform,
        Call {
            module: "music.musichallSinger.SingerInfoInter",
            method: "GetSingerDetail",
            param: json!({
                "singer_mids": [singer_mid],
                "ex_singer": true,
                "wiki_singer": true,
                "group_singer": true,
                "pic": true,
                "photos": true,
            }),
        },
    )?;
    let singer = first_array(&data, &["singer_list", "singerList", "list"])
        .and_then(|list| list.first())
        .cloned()
        .unwrap_or(data.clone());
    let basic = first_object(&singer, &["basic_info", "basicInfo", "info"]).cloned().unwrap_or(singer.clone());
    // The biography arrives under `wiki` (a wiki XML blob) or `desc`.
    let biography = first_text(&singer, &["desc", "description"])
        .or_else(|| first_object(&singer, &["wiki"]).and_then(|wiki| first_text(wiki, &["desc", "content", "introduction"])))
        .unwrap_or_default();
    Ok(json!({
        "singerMid": singer_mid,
        "name": first_text(&basic, &["name", "singer_name", "singerName"]).unwrap_or_else(|| "未知歌手".into()),
        "description": biography,
        "coverURL": crate::methods::normalized_artwork_url(
            first_text(&basic, &["pic", "photo", "picURL"]).as_deref()
        ),
        "foreignName": first_text(&basic, &["foreign_name", "foreignName", "other_name"]),
        "region": first_text(&basic, &["country", "area", "region"]),
        "genre": first_text(&basic, &["genre", "tag"]).map(split_tags).unwrap_or_default(),
        "songCount": first_int(&basic, &["song_count", "songNum"]),
        "albumCount": first_int(&basic, &["album_count", "albumNum"]),
        "fanCount": first_int(&basic, &["fans", "fanNum", "fansNum"]),
    }))
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
            let toplists: Vec<Value> = first_array(group, &["topList", "list", "topLists"])
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|item| {
                    let id = first_int(item, &["topId", "id", "toplistId"])?;
                    Some(json!({
                        "id": id,
                        "title": first_text(item, &["topTitle", "title", "name"]).unwrap_or_default(),
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
            Some(json!({ "title": title, "toplists": toplists }))
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
) -> Result<Vec<Value>, UpstreamError> {
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
    Ok(crate::methods::decoded_tracks(&data))
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
                    }))
                })
                .collect();
            if stations.is_empty() {
                return None;
            }
            Some(json!({
                "title": first_text(group, &["title", "name"]).unwrap_or_default(),
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

/// A lyric, with the extra tracks the upstream offers alongside it.
///
/// All four fields arrive base64-encoded even in plaintext mode, and `qrc` is
/// the word-level one — the reason the helper channel existed at all.
pub fn lyric(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    song_mid: &str,
    song_id: Option<i64>,
    with_word_timing: bool,
    with_translation: bool,
) -> Result<Value, UpstreamError> {
    let data = upstream.call_with(
        credential,
        crate::Class::Playback,
        platform,
        Call {
            module: "music.musichallSong.PlayLyricInfo",
            method: "GetPlayLyricInfo",
            param: json!({
                "songMID": song_mid,
                "songID": song_id.unwrap_or(0),
                "format": "json",
                "crypt": 0,
                "qrc": if with_word_timing { 1 } else { 0 },
                "trans": if with_translation { 1 } else { 0 },
                "roma": if with_translation { 1 } else { 0 },
            }),
        },
    )?;
    Ok(json!({
        "lyric": decode_base64_text(data.get("lyric")),
        "translation": decode_base64_text(data.get("trans")),
        "romanization": decode_base64_text(data.get("roma")),
        "wordLyric": decode_base64_text(data.get("qrc")),
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
                "songMid": song_mid,
                "quality": label,
                "filename": filename,
                "url": if purl.starts_with("http") { purl.clone() } else { format!("{STREAM_CDN}{purl}") },
                "playable": true,
            }));
        }
        refusals.push(format!("{label}:{}", describe_result(result)));
    }
    Ok(json!({
        "songMid": song_mid,
        "playable": false,
        "reason": format!("上游没有授予任何可用音质（{}）", refusals.join("、")),
    }))
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

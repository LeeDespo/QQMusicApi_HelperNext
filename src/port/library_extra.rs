//! 曲库补充读取: 新碟, 助唱标注, 多风格翻译, AI 词典与指定用户喜欢的歌曲.
//!
//! 请求以本地参考 `modules/album.py`, `modules/lyric.py`, `modules/user.py`
//! 为准. 所有端点均无 require_login, 使用调用方的平台档案与当前凭据.
//! 多风格歌词照参考先尝试 QRC 解密, 解密失败保留原文. 列表的缺省值
//! 照参考模型; 用户喜欢的歌曲必须有 songlist/dirinfo 形状, 空数组是合法页.

use crate::credential::Credential;
use crate::models::{Singer, Track};
use crate::upstream::{
    first_array, first_int, first_object, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 本模块支持的协议方法.
pub const METHODS: &[&str] = &[
    "fetch_new_albums",
    "fetch_singing_annotations",
    "fetch_multi_style_lyrics",
    "has_ai_dictionary",
    "fetch_ai_dictionary",
    "fetch_user_liked_songs",
];

/// 新碟摘要, 扩展参考基础 Album 字段.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewAlbumItem {
    pub id: Option<i64>,
    pub mid: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub time_public: Option<String>,
    pub pmid: Option<String>,
    pub singers: Option<Vec<Singer>>,
    pub release_time: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    pub area: Option<i64>,
    pub genre: Option<i64>,
    pub language: Option<i64>,
}

/// 当前地区的新碟总数与当前页.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewAlbumResponse {
    pub total: Option<i64>,
    pub albums: Option<Vec<NewAlbumItem>>,
}

/// 助唱标注可用性, 本接口不返回歌词正文.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingingAnnotationsResponse {
    pub has_singing_annotations_lyric: Option<bool>,
}

/// 多风格翻译歌词的一种风格.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MultiStyleLyricItem {
    pub style: Option<i64>,
    pub style_name: Option<String>,
    pub lyric: Option<String>,
    pub timestamp: Option<i64>,
}

/// 多风格翻译列表, 无翻译时为空数组.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MultiStyleLyricsResponse {
    pub lyrics: Option<Vec<MultiStyleLyricItem>>,
}

/// AI 歌词词典可用性.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiDictionaryExistsResponse {
    pub exists: Option<bool>,
}

/// AI 词典中的短语与解释, 时间戳保留参考的字符串类型.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiDictionaryItem {
    pub phrase: Option<String>,
    pub explain: Option<String>,
    pub lyric_text: Option<String>,
    pub trans_lyric_text: Option<String>,
    pub lyric_timestamp: Option<String>,
}

/// AI 歌词词典列表.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiDictionaryResponse {
    pub dict_list: Option<Vec<AiDictionaryItem>>,
}

/// 喜欢歌曲目录的创建者.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LikedSongsCreator {
    pub musicid: Option<i64>,
    pub nick: Option<String>,
    pub headurl: Option<String>,
    pub encrypt_uin: Option<String>,
}

/// 喜欢歌曲的目录元数据, 对应参考 SonglistInfo.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LikedSongsInfo {
    pub id: Option<i64>,
    pub dirid: Option<i64>,
    pub title: Option<String>,
    pub picurl: Option<String>,
    pub desc: Option<String>,
    pub songnum: Option<i64>,
    pub listennum: Option<i64>,
    pub creator: Option<LikedSongsCreator>,
}

/// 指定用户喜欢的歌曲一页, 歌曲沿用组件既有 Track 模型.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserLikedSongsResponse {
    pub code: Option<i64>,
    pub subcode: Option<i64>,
    pub msg: Option<String>,
    pub info: Option<LikedSongsInfo>,
    pub size: Option<i64>,
    pub songs: Option<Vec<Track>>,
    pub total: Option<i64>,
    pub hasmore: Option<i64>,
}

fn text(raw: &Value, keys: &[&str]) -> String {
    first_text(raw, keys).unwrap_or_default()
}

fn integer(raw: &Value, keys: &[&str]) -> i64 {
    first_int(raw, keys).unwrap_or(0)
}

/// 上游布尔兼容数值及常见字符串表示.
fn boolean(raw: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| match raw.get(*key) {
            Some(Value::Bool(value)) => Some(*value),
            Some(Value::Number(value)) => value.as_i64().map(|value| value != 0),
            Some(Value::String(value)) => match value.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(false)
}

fn shape_error(field: &str) -> UpstreamError {
    UpstreamError::Upstream(format!("上游响应形状不对: 缺少或无效 {field}"))
}

/// reference default_factory 只允许缺失时补空, null/错误类型仍是形状故障.
fn default_list<'a>(raw: &'a Value, keys: &[&str]) -> Result<&'a [Value], UpstreamError> {
    for key in keys {
        if let Some(value) = raw.get(*key) {
            return value
                .as_array()
                .map(Vec::as_slice)
                .ok_or_else(|| shape_error(key));
        }
    }
    Ok(&[])
}

/// 实测无附加歌词/词典时, 上游以 JSON null 表示无内容. 仅这两项归一为空列表.
fn optional_feature_list<'a>(raw: &'a Value, keys: &[&str]) -> Result<&'a [Value], UpstreamError> {
    if keys
        .iter()
        .find_map(|key| raw.get(*key))
        .is_some_and(Value::is_null)
    {
        return Ok(&[]);
    }
    default_list(raw, keys)
}

fn new_albums_payload(data: &Value) -> Result<Value, UpstreamError> {
    let albums = default_list(data, &["albums"])?
        .iter()
        .map(|raw| {
            let singers = default_list(raw, &["singers"])?.iter().map(|singer| json!({
            "mid": text(singer, &["mid", "singerMid", "singerMID", "SingerMid", "singer_mid"]),
            "name": text(singer, &["name", "singerName", "singer_name"]),
        })).collect::<Vec<_>>();
            Ok(json!({
                "id": integer(raw, &["id", "albumID"]),
                "mid": text(raw, &["mid", "albumMid", "albumMID", "albummid"]),
                "name": text(raw, &["name", "albumName"]),
                "title": text(raw, &["title", "albumName", "name"]),
                "subtitle": text(raw, &["subtitle", "albumTranName"]),
                "timePublic": text(raw, &["time_public", "publish_date", "publishDate"]),
                "pmid": text(raw, &["pmid", "logo"]),
                "singers": singers,
                "releaseTime": text(raw, &["release_time", "releaseTime"]),
                "type": integer(raw, &["type"]),
                "area": integer(raw, &["area"]),
                "genre": integer(raw, &["genre"]),
                "language": integer(raw, &["language"]),
            }))
        })
        .collect::<Result<Vec<_>, UpstreamError>>()?;
    Ok(json!({"total": integer(data, &["total"]), "albums": albums}))
}

fn multi_style_payload(data: &Value) -> Result<Value, UpstreamError> {
    let lyrics = optional_feature_list(data, &["lyrics"])?
        .iter()
        .map(|raw| {
            let style = first_int(raw, &["style"]).ok_or_else(|| shape_error("lyrics[].style"))?;
            let style_name = raw
                .get("styleName")
                .or_else(|| raw.get("style_name"))
                .and_then(Value::as_str)
                .ok_or_else(|| shape_error("lyrics[].styleName"))?;
            let lyric = raw
                .get("lyric")
                .and_then(Value::as_str)
                .ok_or_else(|| shape_error("lyrics[].lyric"))?;
            // Python suppresses decrypt errors, so valid plain text stays intact.
            let lyric = crate::qrc::decrypt_hex(lyric).unwrap_or_else(|_| lyric.to_string());
            Ok(
                json!({"style": style, "styleName": style_name, "lyric": lyric,
            "timestamp": integer(raw, &["timestamp"])}),
            )
        })
        .collect::<Result<Vec<_>, UpstreamError>>()?;
    Ok(json!({"lyrics": lyrics}))
}

fn ai_dictionary_payload(data: &Value) -> Result<Value, UpstreamError> {
    let items = optional_feature_list(data, &["dictList", "dict_list"])?
        .iter()
        .map(|raw| {
            json!({
                "phrase": text(raw, &["phrase"]), "explain": text(raw, &["explain"]),
                "lyricText": text(raw, &["lyric_text", "lyricText"]),
                "transLyricText": text(raw, &["trans_lyric_text", "transLyricText"]),
                "lyricTimestamp": text(raw, &["lyric_timestamp", "lyricTimestamp"]),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"dictList": items}))
}

fn liked_songs_payload(data: &Value) -> Result<Value, UpstreamError> {
    first_array(data, &["songlist"]).ok_or_else(|| shape_error("songlist"))?;
    let info = first_object(data, &["dirinfo"]).ok_or_else(|| shape_error("dirinfo"))?;
    let creator = first_object(info, &["creator"]).ok_or_else(|| shape_error("dirinfo.creator"))?;
    let creator_musicid =
        first_int(creator, &["musicid"]).ok_or_else(|| shape_error("dirinfo.creator.musicid"))?;
    let size = first_int(data, &["songlist_size"]).ok_or_else(|| shape_error("songlist_size"))?;
    // docs/parsing.md: dirinfo.songnum is the canonical count on CgiGetDiss.
    let total = first_int(info, &["songnum", "songNum", "song_cnt"])
        .or_else(|| first_int(data, &["total_song_num"]))
        .ok_or_else(|| shape_error("dirinfo.songnum/total_song_num"))?;
    Ok(json!({
        "code": integer(data, &["code"]), "subcode": integer(data, &["subcode", "subCode"]),
        "msg": text(data, &["msg"]), "size": size,
        "songs": crate::methods::decoded_tracks(data), "total": total,
        "hasmore": integer(data, &["hasmore", "hasMore"]),
        "info": {
            "id": integer(info, &["id", "tid", "dissid"]), "dirid": integer(info, &["dirid"]),
            "title": text(info, &["title"]),
            "picurl": crate::methods::normalized_artwork_url(first_text(info, &["picurl"]).as_deref()).unwrap_or_default(),
            "desc": text(info, &["desc", "description"]), "songnum": total,
            "listennum": integer(info, &["listennum", "playCnt", "play_cnt"]),
            "creator": {"musicid": creator_musicid, "nick": text(creator, &["nick"]),
                "headurl": crate::methods::normalized_artwork_url(first_text(creator, &["headurl"]).as_deref()).unwrap_or_default(),
                "encryptUin": text(creator, &["encrypt_uin", "encryptUin"]) }
        }
    }))
}

fn pagination(params: &Value, default_num: i64) -> (i64, i64) {
    (
        first_int(params, &["page"]).unwrap_or(1),
        first_int(params, &["num", "limit", "size"]).unwrap_or(default_num),
    )
}

fn new_album_params(params: &Value) -> Value {
    let (page, num) = pagination(params, 20);
    json!({"area": first_int(params, &["area"]).unwrap_or(1), "num": num, "start": num * (page - 1)})
}

fn song_params(params: &Value, annotations: bool) -> Result<Value, UpstreamError> {
    let songid = first_int(
        params,
        &["songId", "songID", "songid", "song_id", "id", "value"],
    )
    .ok_or_else(|| UpstreamError::Upstream("缺少数字 songId".into()))?;
    let mut result = json!({"songID": songid});
    if annotations {
        result["needNum"] = json!(false);
    }
    Ok(result)
}

fn liked_songs_params(params: &Value) -> Result<Value, UpstreamError> {
    let euin = first_text(params, &["euin", "encHostUin", "enc_host_uin"])
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| UpstreamError::Upstream("缺少目标用户 euin".into()))?;
    let (page, num) = pagination(params, 10);
    // Reference preserve_bool defaults false, so True becomes 1 on the wire.
    Ok(
        json!({"disstid": 0, "dirid": 201, "tag": 1, "song_begin": num * (page - 1),
        "song_num": num, "userinfo": 1, "orderlist": 1, "enc_host_uin": euin}),
    )
}

/// 按协议方法读取一页或单曲扩展资料.
pub fn dispatch(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    method: &str,
    params: &Value,
) -> Option<Result<Value, UpstreamError>> {
    if !METHODS.contains(&method) {
        return None;
    }
    Some((|| {
        let call = match method {
            "fetch_new_albums" => Call {
                module: "newalbum.NewAlbumServer",
                method: "get_new_album_info",
                param: new_album_params(params),
            },
            "fetch_user_liked_songs" => Call {
                module: "music.srfDissInfo.DissInfo",
                method: "CgiGetDiss",
                param: liked_songs_params(params)?,
            },
            _ => Call {
                module: "music.musichallSong.PlayLyricInfo",
                method: match method {
                    "fetch_singing_annotations" => "GetSingingAnnotationsInfo",
                    "fetch_multi_style_lyrics" => "BatchGetMultiStyleTransLyric",
                    "has_ai_dictionary" => "IsAIDictExists",
                    "fetch_ai_dictionary" => "GetAIDictInfo",
                    _ => unreachable!(),
                },
                param: song_params(params, method == "fetch_singing_annotations")?,
            },
        };
        let class = if method == "fetch_user_liked_songs" {
            Class::Account
        } else {
            Class::Read
        };
        let data = upstream.call_with(credential, class, platform, call)?;
        match method {
            "fetch_new_albums" => new_albums_payload(&data),
            "fetch_singing_annotations" => Ok(
                json!({"hasSingingAnnotationsLyric": boolean(&data, &["hasSingingAnnotationsLyric"])}),
            ),
            "fetch_multi_style_lyrics" => multi_style_payload(&data),
            "has_ai_dictionary" => Ok(json!({"exists": boolean(&data, &["exists"])})),
            "fetch_ai_dictionary" => ai_dictionary_payload(&data),
            "fetch_user_liked_songs" => liked_songs_payload(&data),
            _ => unreachable!(),
        }
    })())
}

/// 新碟上架一页, 默认地区 1, 20 张, 第 1 页.
#[export]
pub fn fetch_new_albums(
    area: Option<i64>,
    num: Option<i64>,
    page: Option<i64>,
) -> Result<NewAlbumResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(value) = area {
        params["area"] = json!(value);
    }
    if let Some(value) = num {
        params["num"] = json!(value);
    }
    if let Some(value) = page {
        params["page"] = json!(value);
    }
    crate::port::call("fetch_new_albums", params)
}

/// 检查歌曲是否提供助唱标注歌词.
#[export]
pub fn fetch_singing_annotations(
    song_id: i64,
) -> Result<SingingAnnotationsResponse, crate::HelperError> {
    crate::port::call("fetch_singing_annotations", json!({"songId": song_id}))
}

/// 获取歌曲的多风格翻译歌词.
#[export]
pub fn fetch_multi_style_lyrics(
    song_id: i64,
) -> Result<MultiStyleLyricsResponse, crate::HelperError> {
    crate::port::call("fetch_multi_style_lyrics", json!({"songId": song_id}))
}

/// 检查歌曲是否有 AI 词典.
#[export]
pub fn has_ai_dictionary(song_id: i64) -> Result<AiDictionaryExistsResponse, crate::HelperError> {
    crate::port::call("has_ai_dictionary", json!({"songId": song_id}))
}

/// 获取歌曲的 AI 词典.
#[export]
pub fn fetch_ai_dictionary(song_id: i64) -> Result<AiDictionaryResponse, crate::HelperError> {
    crate::port::call("fetch_ai_dictionary", json!({"songId": song_id}))
}

/// 获取指定加密 UIN 用户喜欢的歌曲, 默认第 1 页, 10 首.
#[export]
pub fn fetch_user_liked_songs(
    euin: String,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserLikedSongsResponse, crate::HelperError> {
    let mut params = json!({"euin": euin});
    if let Some(value) = page {
        params["page"] = json!(value);
    }
    if let Some(value) = num {
        params["num"] = json!(value);
    }
    crate::port::call("fetch_user_liked_songs", params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_and_target_user_match_reference() {
        assert_eq!(
            new_album_params(&json!({})),
            json!({"area":1,"num":20,"start":0})
        );
        assert_eq!(
            new_album_params(&json!({"area":3,"num":7,"page":4})),
            json!({"area":3,"num":7,"start":21})
        );
        assert_eq!(
            liked_songs_params(&json!({"euin":"OTHER","page":3,"num":5})).unwrap(),
            json!({
                "disstid":0,"dirid":201,"tag":1,"song_begin":10,"song_num":5,"userinfo":1,"orderlist":1,"enc_host_uin":"OTHER"
            })
        );
        assert!(liked_songs_params(&json!({"euin":" "})).is_err());
        assert!(liked_songs_params(&json!({})).is_err());
        assert_eq!(
            song_params(&json!({"songId":"42"}), true).unwrap(),
            json!({"songID":42,"needNum":false})
        );
        assert_eq!(
            song_params(&json!({"songid":42}), false).unwrap(),
            json!({"songID":42})
        );
        assert!(song_params(&json!({"songMid":"MID"}), false).is_err());
    }

    #[test]
    fn new_albums_preserve_release_metadata_and_aliases() {
        let payload = new_albums_payload(&json!({"total":100,"albums":[{
            "albumID":7,"albumMID":"ALBUM","albumName":"新碟","albumTranName":"副标题",
            "publishDate":"2026-10-01","logo":"MEDIA_ID","release_time":"2026-10-02",
            "singers":[{"singerMID":"SINGER","singerName":"歌手"}],"type":2,"area":1,"genre":3,"language":4
        }]})).unwrap();
        let parsed: NewAlbumResponse = serde_json::from_value(payload.clone()).unwrap();
        let album = &parsed.albums.unwrap()[0];
        assert_eq!(parsed.total, Some(100));
        assert_eq!(album.id, Some(7));
        assert_eq!(album.title.as_deref(), Some("新碟"));
        assert_eq!(album.time_public.as_deref(), Some("2026-10-01"));
        assert_eq!(album.release_time.as_deref(), Some("2026-10-02"));
        assert_eq!(album.pmid.as_deref(), Some("MEDIA_ID"));
        assert_eq!(
            album.singers.as_ref().unwrap()[0].mid.as_deref(),
            Some("SINGER")
        );
        assert_eq!(payload["albums"][0]["type"], 2);
        assert_eq!(
            new_albums_payload(&json!({})).unwrap(),
            json!({"total":0,"albums":[]})
        );
        assert!(new_albums_payload(&json!({"albums":null})).is_err());
    }

    #[test]
    fn optional_lyric_features_have_reference_defaults() {
        assert!(!boolean(&json!({}), &["exists"]));
        assert!(boolean(&json!({"exists":"1"}), &["exists"]));
        assert!(!boolean(
            &json!({"hasSingingAnnotationsLyric":0}),
            &["hasSingingAnnotationsLyric"]
        ));
        assert_eq!(
            multi_style_payload(&json!({})).unwrap(),
            json!({"lyrics":[]})
        );
        let payload = multi_style_payload(
            &json!({"lyrics":[{"style":2,"styleName":"诗意","lyric":"[00:01]明文"}]}),
        )
        .unwrap();
        let parsed: MultiStyleLyricsResponse = serde_json::from_value(payload).unwrap();
        let item = &parsed.lyrics.unwrap()[0];
        assert_eq!(item.lyric.as_deref(), Some("[00:01]明文"));
        assert_eq!(item.timestamp, Some(0));
        assert!(multi_style_payload(&json!({"lyrics":[{"style":1}]})).is_err());
        assert_eq!(
            multi_style_payload(&json!({"lyrics":null})).unwrap(),
            json!({"lyrics":[]})
        );
        assert_eq!(
            ai_dictionary_payload(&json!({"dictList":null})).unwrap(),
            json!({"dictList":[]})
        );
        assert!(multi_style_payload(&json!({"lyrics":{}})).is_err());
    }

    #[test]
    fn encrypted_multi_style_lyrics_use_the_existing_qrc_decoder() {
        let cipher = include_str!("../../tests/fixtures/qrc/qrc-vector.hex").trim();
        let payload = multi_style_payload(&json!({"lyrics":[{
            "style":1,"styleName":"译文","lyric":cipher,"timestamp":99
        }]}))
        .unwrap();
        let decrypted = payload["lyrics"][0]["lyric"].as_str().unwrap();
        assert!(decrypted.contains("<QrcInfos>"));
        assert_eq!(decrypted.chars().count(), 7188);
        assert_eq!(payload["lyrics"][0]["timestamp"], 99);
    }

    #[test]
    fn dictionary_maps_snake_case_and_keeps_timestamp_text() {
        let payload = ai_dictionary_payload(&json!({"dictList":[{"phrase":"word","explain":"释义",
            "lyric_text":"line","trans_lyric_text":"歌词","lyric_timestamp":"[00:01.20]"},{}]}))
        .unwrap();
        let parsed: AiDictionaryResponse = serde_json::from_value(payload.clone()).unwrap();
        let items = parsed.dict_list.unwrap();
        assert_eq!(items[0].lyric_timestamp.as_deref(), Some("[00:01.20]"));
        assert_eq!(payload["dictList"][0]["transLyricText"], "歌词");
        assert_eq!(items[1].phrase.as_deref(), Some(""));
        assert_eq!(
            ai_dictionary_payload(&json!({})).unwrap(),
            json!({"dictList":[]})
        );
    }

    #[test]
    fn liked_song_page_keeps_directory_count_and_creator() {
        let payload = liked_songs_payload(&json!({"dirinfo":{"dirid":201,"title":"我喜欢",
            "songnum":123,"picurl":"http://y.gtimg.cn/cover.jpg","creator":{"musicid":9,
            "nick":"甲","headurl":"//y.gtimg.cn/head.jpg","encrypt_uin":"OTHER"}},
            "songlist_size":1,"total_song_num":1,"hasmore":1,
            "songlist":[{"id":42,"mid":"SONG","name":"歌曲","singer":[{"name":"歌手"}]}]}))
        .unwrap();
        let parsed: UserLikedSongsResponse = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(parsed.total, Some(123));
        assert_eq!(parsed.size, Some(1));
        assert_eq!(parsed.songs.unwrap()[0].song_mid, "SONG");
        assert_eq!(payload["info"]["creator"]["encryptUin"], "OTHER");
        assert_eq!(payload["info"]["picurl"], "https://y.gtimg.cn/cover.jpg");
        assert_eq!(
            payload["info"]["creator"]["headurl"],
            "https://y.gtimg.cn/head.jpg"
        );
        let empty = json!({"dirinfo":{"songnum":0,"creator":{"musicid":9}},"songlist":[],"songlist_size":0});
        assert_eq!(liked_songs_payload(&empty).unwrap()["songs"], json!([]));
        assert!(liked_songs_payload(&json!({"songlist":[]})).is_err());
        assert!(liked_songs_payload(&json!({"dirinfo":{"songnum":0}})).is_err());
    }
}

//! 歌曲关联：相似歌曲、标签、相关歌单、相关 MV、乐谱（含签名路）。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/song.py` 的
//! `get_similar_song` / `get_labels` / `get_related_songlist` / `get_related_mv` /
//! `get_sheet` / `has_sheet`，回值字段名照 `models/song.py` 的同名模型
//! （`GetSimilarSongResponse` / `GetSongLabelsResponse` / `GetRelatedSonglistResponse` /
//! `GetRelatedMvResponse` / `GetSheetResponse` / `HasSheetMusicResponse`，camelCase）。
//!
//! 上游三个模块：
//! * `music.recommend.TrackRelationServer` —— 相似歌曲、标签、相关歌单（三个都在这里）；
//! * `MvService.MvInfoProServer` —— 相关 MV（与 `mv.rs` 的列表同一个模块）；
//! * `music.mir.SheetMusicSvr` —— 曲谱（`GetMoreSheetMusic` / `GetChongChongSheetMusic`
//!   与 `HasSheetMusic`）。
//!
//! # 平台档案
//!
//! 六个端点参考都没标 `platform`，按本层约定默认 Web；`dispatch` 收到调用方传入的
//! platform 时原样尊重。乐谱两个端点用参考的 `override_comm=True`（匿名 h5 comm），
//! 档案对它们没有实际影响。
//!
//! # 签名路
//!
//! 乐谱两个端点都走 `musics.fcg` 签名路（工作单要求；`signed.rs` 与 `upstream.rs`
//! 也是按这条路准备的）。参考实现里只有虫虫钢琴那一档显式标了 `sign=True`，
//! 默认档与 `has_sheet` 没有标——但签名对它们不是负担：信封、module/method、参数
//! 一个字不改，只是 URL 上多带一个 `zzc`。两个端点都照参考传 `override_comm=True`
//! 的匿名 h5 comm（`uin ""`、`g_tk` 字面量 5381），即
//! [`crate::port::signed::anonymous_h5_comm`]；虫虫钢琴那一档还带 `platform: "h5"`。
//!
//! # 10007
//!
//! `10007` 是「这首歌没有曲谱」，参考把它列进两个乐谱 meta 的
//! `allow_error_codes` 并照常解析回值。`Upstream::call_signed` 对
//! 「10007 + 非空 data」直接回数据；只有「10007 + 空 data」会被它收成错误，
//! 这里把它还原成空答案（见 [`no_sheet_code`]）——「没有曲谱」不是故障。
//!
//! # 空值判据
//!
//! 六个接口在参考里都没有 `require_login`，读的是公开曲库的关联数据：
//! **空列表是答案**（这首歌就是没有相似歌曲 / 标签 / 相关歌单 / 相关 MV / 曲谱），
//! 照常返回，不报错。缺 `songid` / `mid` 是调用方的错误，在发请求之前报错。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::models::Track;
use crate::upstream::{
    first_array, first_int, first_object, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "fetch_similar_songs",
    "fetch_song_labels",
    "fetch_related_playlists",
    "fetch_related_mvs",
    "fetch_sheet_music",
    "has_sheet_music",
];

/// 曲谱来源类型（参考 `get_sheet` 的 `ttype`）：0=用户上传，1=引擎/AI 曲谱，2=虫虫钢琴。
const SHEET_TYPE_USER: i64 = 0;
/// 引擎/AI 曲谱。
const SHEET_TYPE_AI: i64 = 1;
/// 虫虫钢琴：单独一个 method，且 comm 里多一个 `platform: "h5"`。
const SHEET_TYPE_CHONGCHONG: i64 = 2;
/// 参考写死一次拉取的范围 `begin 0 / end 100`。
const SHEET_BEGIN: i64 = 0;
const SHEET_END: i64 = 100;
/// AI 曲谱的 `scoreType`（参考的 `-473 if ttype == 1 else -1`）。
const SHEET_SCORE_TYPE_AI: i64 = -473;
const SHEET_SCORE_TYPE_DEFAULT: i64 = -1;
/// 相关 MV 固定带的 `songtype`（参考逐字写的 1）。
const RELATED_MV_SONG_TYPE: i64 = 1;

// MARK: - 模型

/// 参考 `SimilarSongGroup`：一组相似歌曲推荐卡片.
///
/// 参考的 `song` 字段用 jsonpath `$.songs[*].track` 从每个分组自己的
/// `songs` 里抽出曲目，上游给的是 `{"track": {...}}` 或曲目本体；这里交给组件
/// 既有的曲目解码（`methods::decoded_tracks`，它认这两种形状）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SimilarSongGroup {
    /// 推荐分组的标题模板（参考字段是 snake_case，上游也就给这个拼写）.
    pub title_template: Option<String>,
    /// 标题模板里的实际内容.
    pub title_content: Option<String>,
    /// 当前推荐分组下的歌曲列表.
    pub song: Option<Vec<Track>>,
}

/// 参考 `GetSimilarSongResponse`：相似歌曲推荐响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetSimilarSongResponse {
    /// 本次推荐附带的歌曲标签列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub tag: Option<String>,
    /// 按卡片分组组织的相似歌曲结果.
    pub song: Option<Vec<SimilarSongGroup>>,
}

/// 参考 `SongLabel`：歌曲标签项.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongLabel {
    /// 标签 ID.
    pub id: Option<i64>,
    /// 标签文本（上游键 `tagTxt`）.
    pub tag_txt: Option<String>,
    /// 标签图标地址（上游键 `tagIcon`）.
    pub tag_icon: Option<String>,
    /// 标签跳转链接（上游键 `tagUrl`）.
    pub tag_url: Option<String>,
    /// 标签类型（上游键 `tagType`）.
    pub tag_type: Option<i64>,
    /// 标签所属分类.
    pub species: Option<i64>,
}

/// 参考 `GetSongLabelsResponse`：获取歌曲标签结果.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetSongLabelsResponse {
    /// 歌曲标签列表.
    pub labels: Option<Vec<SongLabel>>,
}

/// 参考 `RelatedPlaylist`（继承 `SongList`）：歌曲详情页关联歌单中的单个歌单摘要.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelatedPlaylist {
    /// 歌单数字 ID（参考别名 `tid`/`dissid`）.
    pub id: Option<i64>,
    /// 目录 ID（参考别名 `dirId`）.
    pub dirid: Option<i64>,
    /// 歌单标题（参考别名 `dissname`/`name`/`dirName`）.
    pub title: Option<String>,
    /// 歌单封面地址（参考别名 `cover`/`logo`/`picUrl`）.
    pub picurl: Option<String>,
    /// 歌单简介（参考别名 `description`）.
    pub desc: Option<String>,
    /// 歌曲数量（参考别名 `songNum`/`song_cnt`）.
    pub songnum: Option<i64>,
    /// 播放量（参考别名 `playCnt`/`play_cnt`）.
    pub listennum: Option<i64>,
    /// 歌单创建者名称.
    pub creator: Option<String>,
}

/// 参考 `GetRelatedSonglistResponse`：歌曲关联歌单响应.
///
/// 参考的 `songlist` 用 jsonpath `$.vecPlaylistNew[*].playlists[*]` 把分组拍平；
/// 上游只在每组名下放 `playlists`。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetRelatedSonglistResponse {
    /// 是否还有更多结果（上游键 `hasMore`）.
    pub has_more: Option<i64>,
    /// 按推荐分组展开后的相关歌单列表.
    pub songlist: Option<Vec<RelatedPlaylist>>,
}

/// 参考 `RelatedMv.MVSinger`：关联 MV 中的歌手摘要（基础 `Singer` 多一个 `picurl`）.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelatedMvSinger {
    /// 歌手数字 ID.
    pub id: Option<i64>,
    /// 歌手 Media MID.
    pub mid: Option<String>,
    /// 歌手名称.
    pub name: Option<String>,
    /// 歌手展示标题（参考回退到名称）.
    pub title: Option<String>,
    /// 歌手类型（参考字段名 `type`，别名 `SingerType`/`vt`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// 与歌手关联的用户 ID.
    pub uin: Option<i64>,
    /// 图片 Media ID.
    pub pmid: Option<String>,
    /// 歌手头像地址.
    pub picurl: Option<String>,
}

/// 参考 `RelatedMv`（继承 `MV`）：歌曲详情页关联 MV 的摘要信息.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelatedMv {
    /// MV 数字 ID（参考别名 `sid`/`mvid`/`singerId`）.
    pub id: Option<i64>,
    /// MV VID.
    pub vid: Option<String>,
    /// MV 类型（参考别名 `vt`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// MV 名称（参考别名 `mvname`/`title`）.
    pub name: Option<String>,
    /// MV 展示标题（参考别名 `title_main`/`name`）.
    pub title: Option<String>,
    /// MV 封面.
    pub picurl: Option<String>,
    /// MV 播放量.
    pub playcnt: Option<i64>,
    /// MV 关联歌手列表.
    pub singers: Option<Vec<RelatedMvSinger>>,
}

/// 参考 `GetRelatedMvResponse`：歌曲关联 MV 响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetRelatedMvResponse {
    /// 是否还有更多结果（上游键是小写的 `hasmore`）.
    pub has_more: Option<i64>,
    /// 当前返回的相关 MV 列表（上游键 `list`）.
    pub mv: Option<Vec<RelatedMv>>,
}

/// 参考 `SheetMusic`：曲谱项.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SheetMusic {
    /// 曲谱 MID（上游键 `scoreMID`）.
    pub score_mid: Option<String>,
    /// 曲谱名称.
    pub score_name: Option<String>,
    /// 曲谱图片列表（上游键 `picURLs`）.
    pub pic_urls: Option<Vec<String>>,
    /// 曲谱版本说明.
    pub version: Option<String>,
    /// 调号.
    pub tonality: Option<i64>,
    /// 曲谱类型.
    pub score_type: Option<i64>,
    /// 曲谱类型文本（上游键 `strScoreType`）.
    pub score_type_text: Option<String>,
    /// 上传者.
    pub uploader: Option<String>,
    /// 浏览量（上游键 `viewFrequency`）.
    pub view_frequency: Option<i64>,
    /// 第二调号值.
    pub tonality2: Option<i64>,
    /// 作者.
    pub author: Option<String>,
    /// 作曲.
    pub composer: Option<String>,
    /// 作词.
    pub lyricist: Option<String>,
    /// 演唱者.
    pub singer: Option<String>,
    /// 演奏者.
    pub performer: Option<String>,
    /// 关联歌曲 MID（上游键 `songMID`）.
    pub song_mid: Option<String>,
    /// 曲谱副标题（上游键 `subName`）.
    pub sub_name: Option<String>,
    /// 曲谱详情链接.
    pub url: Option<String>,
    /// 专辑链接（上游键 `albumURL`）.
    pub album_url: Option<String>,
    /// 乐器类型（上游键 `insType`）.
    pub ins_type: Option<i64>,
    /// 乐器类型文本（上游键 `strInsType`）.
    pub ins_type_text: Option<String>,
    /// 乐器封面（上游键 `coverURL`）.
    pub cover_url: Option<String>,
    /// 难度.
    pub difficulty: Option<String>,
    /// 曲谱文件地址（上游键 `sheetFile`）.
    pub sheet_file: Option<String>,
}

/// 参考 `GetSheetResponse`：歌曲相关曲谱响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetSheetResponse {
    /// 当前返回的曲谱列表；没有曲谱时是空列表（参考的 `NoneToEmptyList` 也把 null 收成空）.
    pub result: Option<Vec<SheetMusic>>,
    /// 各曲谱类型对应的数量聚合（上游键 `totalMap`）.
    #[serde(default, deserialize_with = "int_map")]
    pub total_map: Option<HashMap<String, i64>>,
}

/// 参考 `HasSheetMusicResponse`：检查歌曲曲谱存在状态响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HasSheetMusicResponse {
    /// 是否有 AI 生成曲谱（尤克里里等）.
    pub has_guitar: Option<bool>,
    /// 是否有更多来源的曲谱.
    pub has_more: Option<bool>,
    /// 是否有六线谱/吉他谱（参考字段名 `has_ldy`，上游键 `hasLDY`）.
    pub has_ldy: Option<bool>,
    /// 是否有标准五线谱/曲谱（参考字段名 `has_qrcx`，上游键 `hasQRCX`）.
    pub has_qrcx: Option<bool>,
    /// 是否有虫虫钢琴谱.
    pub has_chong_chong: Option<bool>,
}

// MARK: - 参考模型里 `dict` / 任意 JSON 形状的字段

/// 把 Rust 侧存的 JSON 文本按真 JSON 写出（宿主在 JSON 协议上看到的是对象/数组）。
///
/// 与 `comment.rs` / `mv.rs` 的同名助手同源：每个领域文件自包含，这份是刻意复制的。
fn raw_json_serialize<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(text) => {
            let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            parsed.serialize(serializer)
        }
        None => serializer.serialize_none(),
    }
}

/// 把上游给的任意 JSON 收成文本；`null`/缺失即 `None`。
fn raw_json_deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Null => None,
        other => Some(other.to_string()),
    })
}

/// 数字或数字字符串 → 整数。
fn coerce_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 曲谱类型的数量聚合：上游给数字也给数字字符串（pydantic 的 `dict[str, int]` 会转），
/// 非数字的值丢掉——那是上游形状变了，不是「这种曲谱有 0 份」。
fn int_map<'de, D>(deserializer: D) -> Result<Option<HashMap<String, i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Object(entries) => Some(
            entries
                .into_iter()
                .filter_map(|(key, value)| coerce_int(&value).map(|number| (key, number)))
                .collect(),
        ),
        _ => None,
    })
}

/// 宽松的布尔：上游可能给 `true`、`1` 或 `"1"`（pydantic 也会这么转）。
fn bool_field(raw: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| match raw.get(*key) {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::Number(number)) => Some(number.as_i64().unwrap_or(0) != 0),
        Some(Value::String(text)) => match text.as_str() {
            "true" | "True" | "1" => Some(true),
            "false" | "False" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

// MARK: - 回值搬运（上游形状 → 参考模型形状）

/// 取第一个出现的键，原样保留（不透传形状的字段用）。
fn raw_field(raw: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .filter(|found| !found.is_null())
        .cloned()
}

/// 一组字符串（上游有时给数字或 null）。
fn string_list(raw: &Value, keys: &[&str]) -> Option<Vec<String>> {
    first_array(raw, keys).map(|items| {
        items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .collect()
    })
}

/// 单个相似歌曲分组（参考 `SimilarSongGroup`）。
///
/// 参考的两个标题字段没有别名，上游也就给 snake_case；这里把 camelCase 也收进来，
/// 免得换一个上游版本整组读空。
fn similar_song_group_payload(raw: &Value) -> Value {
    let tracks = first_array(raw, &["songs"]).cloned().unwrap_or_default();
    json!({
        "titleTemplate": first_text(raw, &["title_template", "titleTemplate"]),
        "titleContent": first_text(raw, &["title_content", "titleContent"]),
        "song": crate::methods::decoded_tracks(&json!({ "songList": tracks })),
    })
}

/// 相似歌曲回值（参考 `GetSimilarSongResponse`）：分组在 `vecSongNew` 上，
/// 每组的曲目来自组内的 `songs[*].track`。
fn similar_songs_payload(data: &Value) -> Value {
    let groups = first_array(data, &["vecSongNew"])
        .cloned()
        .unwrap_or_default();
    json!({
        // 参考的 `tag` 缺省是空列表：`NoneToEmptyList` 同样把 null 收成 []。
        "tag": raw_field(data, &["songTagInfoList", "songTagInfo"]).unwrap_or(Value::Array(Vec::new())),
        "song": groups.iter().map(similar_song_group_payload).collect::<Vec<_>>(),
    })
}

/// 单个歌曲标签（参考 `SongLabel`）。
fn song_label_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id"]),
        "tagTxt": first_text(raw, &["tagTxt", "tag_txt"]),
        "tagIcon": first_text(raw, &["tagIcon", "tag_icon"]),
        "tagUrl": first_text(raw, &["tagUrl", "tag_url"]),
        "tagType": first_int(raw, &["tagType", "tag_type"]),
        "species": first_int(raw, &["species"]),
    })
}

/// 标签回值（参考 `GetSongLabelsResponse`）：列表在 `labels` 上。
fn song_labels_payload(data: &Value) -> Value {
    json!({
        "labels": first_array(data, &["labels"])
            .map(|items| items.iter().map(song_label_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

/// 单个关联歌单（参考 `RelatedPlaylist`）。
fn related_playlist_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "tid", "dissid"]),
        "dirid": first_int(raw, &["dirid", "dirId"]),
        "title": first_text(raw, &["title", "dissname", "name", "dirName"]),
        "picurl": first_text(raw, &["picurl", "cover", "logo", "picUrl"]),
        "desc": first_text(raw, &["desc", "description"]),
        "songnum": first_int(raw, &["songnum", "songNum", "song_cnt"]),
        "listennum": first_int(raw, &["listennum", "playCnt", "play_cnt"]),
        "creator": first_text(raw, &["creator"]),
    })
}

/// 相关歌单回值（参考 `GetRelatedSonglistResponse`）。
///
/// 参考的 jsonpath `$.vecPlaylistNew[*].playlists[*]` 把分组拍平；上游换成扁平
/// 列表时（没有 `vecPlaylistNew`）退回按 `songlist`/`playlists` 直接读。
fn related_songlist_payload(data: &Value) -> Value {
    let mut playlists: Vec<Value> = Vec::new();
    if let Some(groups) = first_array(data, &["vecPlaylistNew"]) {
        for group in groups {
            if let Some(items) = first_array(group, &["playlists", "playlist"]) {
                playlists.extend(items.iter().map(related_playlist_payload));
            }
        }
    } else if let Some(items) = first_array(data, &["songlist", "songList", "playlists"]) {
        playlists.extend(items.iter().map(related_playlist_payload));
    }
    json!({
        "hasMore": first_int(data, &["hasMore", "has_more"]),
        "songlist": playlists,
    })
}

/// 单个关联 MV 的歌手（参考 `RelatedMv.MVSinger`，别名表照基础 `Singer`）。
fn mv_singer_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "singerID", "singerId", "SingerID", "singer_id"]),
        "mid": first_text(raw, &["mid", "singerMid", "singerMID", "SingerMid", "singer_mid"]),
        "name": first_text(raw, &["name", "singerName", "singer_name"]),
        "title": first_text(raw, &["title", "singerName", "name"]),
        "type": first_int(raw, &["type", "SingerType", "vt"]),
        "uin": first_int(raw, &["uin"]),
        "pmid": first_text(raw, &["pmid", "singerPmid", "singer_pmid", "pic_mid"]),
        "picurl": first_text(raw, &["picurl", "picUrl"]),
    })
}

/// 单个关联 MV（参考 `RelatedMv`）。
fn related_mv_item_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "sid", "mvid", "singerId"]),
        "vid": first_text(raw, &["vid"]),
        "type": first_int(raw, &["type", "vt"]),
        "name": first_text(raw, &["name", "mvname", "title"]),
        "title": first_text(raw, &["title", "title_main", "name"]),
        "picurl": first_text(raw, &["picurl", "picUrl"]),
        "playcnt": first_int(raw, &["playcnt"]),
        "singers": first_array(raw, &["singers"])
            .map(|items| items.iter().map(mv_singer_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

/// 相关 MV 回值（参考 `GetRelatedMvResponse`）：`hasmore` 是小写，列表在 `list` 上。
fn related_mv_payload(data: &Value) -> Value {
    json!({
        "hasMore": first_int(data, &["hasmore", "hasMore"]),
        "mv": first_array(data, &["list", "List"])
            .map(|items| items.iter().map(related_mv_item_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

/// 单份曲谱（参考 `SheetMusic`）。
///
/// 键名照参考模型的字段名（规则：camelCase 之后与参考模型对齐），上游自己的
/// 拼写（`scoreMID`/`picURLs`/`albumURL`…）只出现在候选键里。
fn sheet_music_payload(raw: &Value) -> Value {
    json!({
        "scoreMid": first_text(raw, &["scoreMID", "scoreMid"]),
        "scoreName": first_text(raw, &["scoreName"]),
        // 参考的 `NoneToEmptyList`：null 也收成空列表。
        "picUrls": string_list(raw, &["picURLs", "picUrls"]).unwrap_or_default(),
        "version": first_text(raw, &["version"]),
        "tonality": first_int(raw, &["tonality"]),
        "scoreType": first_int(raw, &["scoreType"]),
        "scoreTypeText": first_text(raw, &["strScoreType", "scoreTypeText"]),
        "uploader": first_text(raw, &["uploader"]),
        "viewFrequency": first_int(raw, &["viewFrequency"]),
        "tonality2": first_int(raw, &["tonality2"]),
        "author": first_text(raw, &["author"]),
        "composer": first_text(raw, &["composer"]),
        "lyricist": first_text(raw, &["lyricist"]),
        "singer": first_text(raw, &["singer"]),
        "performer": first_text(raw, &["performer"]),
        "songMid": first_text(raw, &["songMID", "songMid"]),
        "subName": first_text(raw, &["subName"]),
        "url": first_text(raw, &["url"]),
        "albumUrl": first_text(raw, &["albumURL", "albumUrl"]),
        "insType": first_int(raw, &["insType"]),
        "insTypeText": first_text(raw, &["strInsType", "insTypeText"]),
        "coverUrl": first_text(raw, &["coverURL", "coverUrl"]),
        "difficulty": first_text(raw, &["difficulty"]),
        "sheetFile": first_text(raw, &["sheetFile"]),
    })
}

/// 曲谱回值（参考 `GetSheetResponse`）：列表在 `result`，聚合在 `totalMap`。
///
/// `result` 为 null / 缺失都是空列表——「这首歌没有曲谱」就是答案。`totalMap` 在
/// 参考里是必填的 `dict[str, int]`，缺失同样是空映射（与 [`empty_sheet_payload`]
/// 一致，省得同一语义有两个形状）。
fn sheet_payload(data: &Value) -> Value {
    json!({
        "result": first_array(data, &["result"])
            .map(|items| items.iter().map(sheet_music_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
        "totalMap": first_object(data, &["totalMap", "total_map"]).cloned().unwrap_or_else(|| json!({})),
    })
}

/// 「没有曲谱」的空答案：与上游给了 `result: null / totalMap: {}` 时同形。
fn empty_sheet_payload() -> Value {
    json!({ "result": [], "totalMap": {} })
}

/// `call_signed` 把「10007 + 空 data」收成了错误（见 `upstream.rs` 的实现：只有
/// 非空 data 才算端点应答）。参考把 10007 定义成「这首歌没有曲谱」并在两个乐谱
/// meta 上放行，所以在这一条路上把它还原成空答案。
///
/// 错误信息由 `upstream.rs` 拼成 `上游返回错误（{code}）：{msg}`，这里把括号里的
/// 数字解析出来比对——比对整段前缀更不容易被 msg 里的同名数字误伤。
fn no_sheet_code(error: &UpstreamError) -> bool {
    let UpstreamError::Upstream(message) = error else {
        return false;
    };
    let Some((_, rest)) = message.split_once("（") else {
        return false;
    };
    let Some((code, _)) = rest.split_once("）") else {
        return false;
    };
    code.trim().parse::<i64>() == Ok(10007)
}

/// 曲谱存在状态回值（参考 `HasSheetMusicResponse`）。
///
/// 键名照参考模型字段名 camelCase 之后的样子（`has_ldy` → `hasLdy`），上游
/// 自己那套大写缩写（`hasLDY`/`hasQRCX`）只出现在候选键里。
fn has_sheet_payload(data: &Value) -> Value {
    json!({
        "hasGuitar": bool_field(data, &["hasGuitar"]),
        "hasMore": bool_field(data, &["hasMore"]),
        "hasLdy": bool_field(data, &["hasLDY", "hasLdy"]),
        "hasQrcx": bool_field(data, &["hasQRCX", "hasQrcx"]),
        "hasChongChong": bool_field(data, &["hasChongChong", "hasChongchong"]),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 歌曲 ID：参考三个接口都收 `int`，这里也认数字字符串（协议层可能给字符串）。
fn song_id_of(params: &Value) -> Result<i64, UpstreamError> {
    first_int(params, &["songid", "songId", "song_id", "id"])
        .ok_or_else(|| UpstreamError::Upstream("缺少歌曲 ID（songid）".into()))
}

/// 上次请求的相关歌单 ID 列表（参考的 `last`，协议参数名 `vecPlaylist`）。
///
/// 缺省是空列表（参考的 `last or []`）；给了非整数是调用方的错误，在发请求前报错。
fn playlist_ids_of(params: &Value) -> Result<Vec<i64>, UpstreamError> {
    let items = first_array(params, &["vecPlaylist", "last", "ids"])
        .cloned()
        .unwrap_or_default();
    let mut ids = Vec::with_capacity(items.len());
    for item in &items {
        let number = coerce_int(item)
            .ok_or_else(|| UpstreamError::Upstream(format!("vecPlaylist 里有非整数：{item}")))?;
        ids.push(number);
    }
    Ok(ids)
}

/// 上一个 MV 的 VID（参考的 `last_mvid or 0`）：没给时是**数字 0**，不是字符串。
///
/// 「换一批」的游标在参考里是 `RelatedMv.id`（数字），所以数字也原样放行；
/// 字符串 VID 照参考包成字符串。
fn last_mvid_of(params: &Value) -> Value {
    match params.get("lastmvid").or_else(|| params.get("lastMvid")) {
        Some(Value::Number(number)) if number.as_i64().is_some() => Value::Number(number.clone()),
        _ => first_text(params, &["lastmvid", "lastMvid", "last_mvid"])
            .map(|text| json!(text))
            .unwrap_or_else(|| json!(0)),
    }
}

/// 歌曲 MID：乐谱两个端点用它。
fn require_mid(params: &Value) -> Result<String, UpstreamError> {
    first_text(params, &["mid", "songMid", "songmid"])
        .ok_or_else(|| UpstreamError::Upstream("缺少歌曲 MID（mid）".into()))
}

/// 相似歌曲 / 标签参数（参考两个方法都是 `{"songid": songid}`，数字）。
fn song_id_params(song_id: i64) -> Value {
    json!({ "songid": song_id })
}

/// 相关歌单参数（参考 `get_related_songlist`）：`vecPlaylist` 空时是 `[]`。
fn related_songlist_params(song_id: i64, last: Vec<i64>) -> Value {
    json!({ "songid": song_id, "vecPlaylist": last })
}

/// 相关 MV 参数（参考 `get_related_mv`）：`songid` 显式 `str()`，`songtype` 固定 1。
fn related_mv_params(song_id: i64, last_mvid: Value) -> Value {
    json!({
        "songid": song_id.to_string(),
        "songtype": RELATED_MV_SONG_TYPE,
        "lastmvid": last_mvid,
    })
}

/// 曲谱请求（参考 `get_sheet` 的两个分支）：返回 `(method, param, comm)`。
///
/// * `ttype == 2`（虫虫钢琴）走 `GetChongChongSheetMusic`，comm 里带
///   `platform: "h5"`；
/// * 其余走 `GetMoreSheetMusic`，`scoreType` 在 `ttype == 1` 时是 -473，否则 -1。
///
/// 两个分支的 `begin`/`end`/comm 其余字段都照参考逐字抄（`override_comm=True`）。
fn sheet_request(mid: &str, ttype: i64) -> (&'static str, Value, Value) {
    if ttype == SHEET_TYPE_CHONGCHONG {
        (
            "GetChongChongSheetMusic",
            json!({
                "songMid": mid,
                "begin": SHEET_BEGIN,
                "end": SHEET_END,
                "scoreType": SHEET_SCORE_TYPE_DEFAULT,
                "ttype": SHEET_TYPE_AI,
            }),
            crate::port::signed::anonymous_h5_comm(Some("h5")),
        )
    } else {
        let score_type = if ttype == SHEET_TYPE_AI {
            SHEET_SCORE_TYPE_AI
        } else {
            SHEET_SCORE_TYPE_DEFAULT
        };
        (
            "GetMoreSheetMusic",
            json!({
                "songMid": mid,
                "begin": SHEET_BEGIN,
                "end": SHEET_END,
                "scoreType": score_type,
                "ttype": ttype,
            }),
            crate::port::signed::anonymous_h5_comm(None),
        )
    }
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
        "fetch_similar_songs" => similar_songs(upstream, credential, platform, params),
        "fetch_song_labels" => song_labels(upstream, credential, platform, params),
        "fetch_related_playlists" => related_playlists(upstream, credential, platform, params),
        "fetch_related_mvs" => related_mvs(upstream, credential, platform, params),
        "fetch_sheet_music" => sheet_music(upstream, credential, platform, params),
        "has_sheet_music" => has_sheet(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 相似歌曲（`music.recommend.TrackRelationServer / GetSimilarSongs`）。
///
/// 空分组列表是答案：上游对这首歌没有推荐。
fn similar_songs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let song_id = song_id_of(params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.recommend.TrackRelationServer",
            method: "GetSimilarSongs",
            param: song_id_params(song_id),
        },
    )?;
    Ok(similar_songs_payload(&data))
}

/// 歌曲标签（`music.recommend.TrackRelationServer / GetSongLabels`）。
///
/// 空标签列表是答案。
fn song_labels(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let song_id = song_id_of(params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.recommend.TrackRelationServer",
            method: "GetSongLabels",
            param: song_id_params(song_id),
        },
    )?;
    Ok(song_labels_payload(&data))
}

/// 相关歌单（`music.recommend.TrackRelationServer / GetRelatedPlaylist`）。
///
/// `vecPlaylist` 是「换一批」的游标（参考的 BatchRefreshStrategy）：把上一批的
/// 歌单 ID 回传，上游就避开它们。空列表是答案。
fn related_playlists(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let song_id = song_id_of(params)?;
    let param = related_songlist_params(song_id, playlist_ids_of(params)?);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.recommend.TrackRelationServer",
            method: "GetRelatedPlaylist",
            param,
        },
    )?;
    Ok(related_songlist_payload(&data))
}

/// 相关 MV（`MvService.MvInfoProServer / GetSongRelatedMv`）。
///
/// `lastmvid` 是「换一批」的游标（参考的 BatchRefreshStrategy）：回传上一批最后
/// 一个 MV 的 VID；没给时是数字 0。空列表是答案。
fn related_mvs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let song_id = song_id_of(params)?;
    let param = related_mv_params(song_id, last_mvid_of(params));
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "MvService.MvInfoProServer",
            method: "GetSongRelatedMv",
            param,
        },
    )?;
    Ok(related_mv_payload(&data))
}

/// 曲谱（`music.mir.SheetMusicSvr`，签名路）。
///
/// 10007（没有曲谱）是合法答案：`call_signed` 对「10007 + 非空 data」本来就直接
/// 回数据，只有「10007 + 空 data」会变成错误，这里还原成空答案。
fn sheet_music(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let mid = require_mid(params)?;
    let ttype = first_int(params, &["ttype"]).unwrap_or(SHEET_TYPE_USER);
    let (method, param, comm) = sheet_request(&mid, ttype);
    let result = upstream.call_signed(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.mir.SheetMusicSvr",
            method,
            param,
        },
        &[],
        Some(comm),
    );
    match result {
        Ok(data) => Ok(sheet_payload(&data)),
        Err(error) if no_sheet_code(&error) => Ok(empty_sheet_payload()),
        Err(error) => Err(error),
    }
}

/// 曲谱存在状态（`music.mir.SheetMusicSvr / HasSheetMusic`，签名路）。
///
/// 回值全是布尔开关，没有「空」这一说；这里不需要 10007 的还原——参考没把它列进
/// `has_sheet` 的放行码。
fn has_sheet(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let mid = require_mid(params)?;
    let data = upstream.call_signed(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.mir.SheetMusicSvr",
            method: "HasSheetMusic",
            param: json!({ "songMid": mid }),
        },
        &[],
        Some(crate::port::signed::anonymous_h5_comm(None)),
    )?;
    Ok(has_sheet_payload(&data))
}

// MARK: - 宿主包装

/// 相似歌曲；回值按推荐分组给出，每组一个标题与一组曲目。
#[export]
pub fn fetch_similar_songs(song_id: i64) -> Result<GetSimilarSongResponse, crate::HelperError> {
    crate::port::call("fetch_similar_songs", json!({ "songid": song_id }))
}

/// 歌曲标签；空列表表示这首歌没有标签。
#[export]
pub fn fetch_song_labels(song_id: i64) -> Result<GetSongLabelsResponse, crate::HelperError> {
    crate::port::call("fetch_song_labels", json!({ "songid": song_id }))
}

/// 相关歌单；`last` 传上一批的歌单 ID 列表可以「换一批」，缺省是空列表。
#[export]
pub fn fetch_related_playlists(
    song_id: i64,
    last: Option<Vec<i64>>,
) -> Result<GetRelatedSonglistResponse, crate::HelperError> {
    crate::port::call(
        "fetch_related_playlists",
        json!({ "songid": song_id, "last": last.unwrap_or_default() }),
    )
}

/// 相关 MV；`last_mvid` 传上一批最后一个 MV 的 VID 可以「换一批」，缺省发 0。
#[export]
pub fn fetch_related_mvs(
    song_id: i64,
    last_mvid: Option<String>,
) -> Result<GetRelatedMvResponse, crate::HelperError> {
    let mut params = json!({ "songid": song_id });
    if let Some(last_mvid) = last_mvid {
        params["lastMvid"] = json!(last_mvid);
    }
    crate::port::call("fetch_related_mvs", params)
}

/// 曲谱；`ttype` 0=用户上传、1=引擎/AI、2=虫虫钢琴，缺省 0。
/// 没有曲谱时 `result` 是空列表（10007 不是错误）。
#[export]
pub fn fetch_sheet_music(
    mid: String,
    ttype: Option<i64>,
) -> Result<GetSheetResponse, crate::HelperError> {
    let mut params = json!({ "mid": mid });
    if let Some(ttype) = ttype {
        params["ttype"] = json!(ttype);
    }
    crate::port::call("fetch_sheet_music", params)
}

/// 检查歌曲是否有曲谱；回值给出五种来源的布尔开关。
#[export]
pub fn has_sheet_music(mid: String) -> Result<HasSheetMusicResponse, crate::HelperError> {
    crate::port::call("has_sheet_music", json!({ "mid": mid }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 6);
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_song_detail",
            &json!({})
        )
        .is_none());
    }

    #[test]
    fn a_call_without_its_handle_is_refused_before_any_request() {
        let upstream = Upstream::new();
        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_similar_songs",
            &json!({}),
        )
        .expect("本模块认领这个方法")
        .expect_err("缺 songid 要报错");
        assert!(error.to_string().contains("songid"), "{error}");

        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_related_playlists",
            &json!({ "songid": 100, "last": ["不是数字"] }),
        )
        .expect("本模块认领这个方法")
        .expect_err("vecPlaylist 里有非整数要报错");
        assert!(error.to_string().contains("vecPlaylist"), "{error}");

        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "has_sheet_music",
            &json!({}),
        )
        .expect("本模块认领这个方法")
        .expect_err("缺 mid 要报错");
        assert!(error.to_string().contains("MID"), "{error}");
    }

    #[test]
    fn similar_songs_come_out_of_vec_song_new() {
        let payload = similar_songs_payload(&json!({
            "songTagInfoList": [{ "tagId": 1, "tagName": "怀旧" }],
            "vecSongNew": [{
                // 参考的两个标题字段没有别名，上游给的就是 snake_case。
                "title_template": "相似歌曲",
                "title_content": "根据你听的",
                "songs": [{
                    "track": {
                        "id": 2314161,
                        "mid": "003w2xz20QlUZt",
                        "name": "歌名",
                        "singer": [{ "id": 1, "mid": "0025NhlN2yWrP4", "name": "甲" }],
                        "album": { "id": 42, "mid": "0041WVfh2vtlJE", "name": "专辑" },
                        "interval": 240
                    }
                }]
            }]
        }));
        let response: GetSimilarSongResponse =
            serde_json::from_value(payload.clone()).expect("解析相似歌曲");
        // 先留一份宿主看到的形状，再从模型里读字段（读字段会把 `song` 移走）。
        let round: Value = serde_json::to_value(&response).unwrap();
        let groups = response.song.expect("有分组");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title_template.as_deref(), Some("相似歌曲"));
        assert_eq!(groups[0].title_content.as_deref(), Some("根据你听的"));
        let songs = groups[0].song.as_ref().expect("有曲目");
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].song_mid, "003w2xz20QlUZt");
        assert_eq!(songs[0].song_id, Some(2314161));
        assert_eq!(songs[0].artist, "甲");
        // `tag` 是原样透传的 dict 列表：宿主在 JSON 协议上看到的是数组。
        assert_eq!(round["tag"][0]["tagName"], "怀旧");

        // 上游没有推荐时是空分组列表，不是错误。
        let response: GetSimilarSongResponse =
            serde_json::from_value(similar_songs_payload(&json!({}))).expect("解析空回值");
        let round: Value = serde_json::to_value(&response).unwrap();
        assert!(response.song.expect("字段仍在").is_empty());
        assert_eq!(round["tag"], json!([]), "缺省是空列表");
    }

    #[test]
    fn labels_come_out_of_the_labels_list() {
        let payload = song_labels_payload(&json!({
            "labels": [{
                "id": 7,
                "tagTxt": "流行",
                "tagIcon": "https://y.gtimg.cn/icon.png",
                "tagUrl": "https://y.qq.com/tag",
                "tagType": 2,
                "species": 3
            }]
        }));
        let response: GetSongLabelsResponse = serde_json::from_value(payload).expect("解析标签");
        let labels = response.labels.expect("有标签");
        assert_eq!(labels[0].id, Some(7));
        assert_eq!(labels[0].tag_txt.as_deref(), Some("流行"));
        assert_eq!(
            labels[0].tag_icon.as_deref(),
            Some("https://y.gtimg.cn/icon.png")
        );
        assert_eq!(labels[0].tag_type, Some(2));
        assert_eq!(labels[0].species, Some(3));

        let response: GetSongLabelsResponse =
            serde_json::from_value(song_labels_payload(&json!({}))).expect("解析空回值");
        assert!(response.labels.expect("字段仍在").is_empty());
    }

    #[test]
    fn related_playlists_flatten_out_of_the_grouped_vec() {
        let payload = related_songlist_payload(&json!({
            "hasMore": 1,
            "vecPlaylistNew": [
                { "playlists": [
                    {
                        "id": 7011264340i64,
                        "dirid": 0,
                        "title": "歌单一",
                        "picurl": "https://qpic.y.qq.com/a.jpg",
                        "desc": "简介",
                        "songnum": 30,
                        "listennum": 1234,
                        "creator": "甲"
                    },
                    {
                        "id": 7011264341i64,
                        "title": "歌单二",
                        "creator": "乙"
                    }
                ]},
                { "playlists": [
                    {
                        "tid": 7011264342i64,
                        "dissname": "歌单三",
                        "cover": "https://qpic.y.qq.com/c.jpg",
                        "songNum": 12,
                        "playCnt": 99
                    }
                ]}
            ]
        }));
        let response: GetRelatedSonglistResponse =
            serde_json::from_value(payload.clone()).expect("解析相关歌单");
        // 先留一份宿主看到的形状，再从模型里读字段（读字段会把 `songlist` 移走）。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(response.has_more, Some(1));
        assert_eq!(round["hasMore"], 1);
        assert_eq!(round["songlist"][0]["creator"], "甲");
        let playlists = response.songlist.expect("有歌单");
        // 两个分组拍平成一条列表，顺序照 JSON 里的次序。
        assert_eq!(playlists.len(), 3);
        assert_eq!(playlists[0].id, Some(7011264340));
        assert_eq!(playlists[0].title.as_deref(), Some("歌单一"));
        assert_eq!(playlists[0].creator.as_deref(), Some("甲"));
        assert_eq!(playlists[0].songnum, Some(30));
        assert_eq!(playlists[0].listennum, Some(1234));
        // 参考的别名表：`tid`/`dissname`/`cover`/`songNum`/`playCnt` 都要认。
        assert_eq!(playlists[2].id, Some(7011264342));
        assert_eq!(playlists[2].title.as_deref(), Some("歌单三"));
        assert_eq!(
            playlists[2].picurl.as_deref(),
            Some("https://qpic.y.qq.com/c.jpg")
        );
        assert_eq!(playlists[2].songnum, Some(12));
        assert_eq!(playlists[2].listennum, Some(99));

        // 空列表是答案：这首歌没有相关歌单。
        let response: GetRelatedSonglistResponse =
            serde_json::from_value(related_songlist_payload(&json!({ "hasMore": 0 })))
                .expect("解析空回值");
        assert!(response.songlist.expect("字段仍在").is_empty());
    }

    #[test]
    fn related_mvs_map_every_reference_field() {
        let payload = related_mv_payload(&json!({
            "hasmore": 1,
            "list": [{
                "id": 123,
                "vid": "013xscuH0xlbie",
                "vt": 1,
                "mvname": "MV 名",
                "title_main": "MV 标题",
                "picurl": "https://y.gtimg.cn/mv.jpg",
                "playcnt": 999,
                "singers": [{
                    "id": 1,
                    "mid": "00255EBh1FcAEk",
                    "name": "甲",
                    "title": "甲",
                    "type": 0,
                    "uin": 0,
                    "pmid": "p1",
                    "picurl": "https://y.gtimg.cn/singer.jpg"
                }]
            }]
        }));
        let response: GetRelatedMvResponse =
            serde_json::from_value(payload.clone()).expect("解析相关 MV");
        assert_eq!(response.has_more, Some(1), "上游键是小写 hasmore");
        let mvs = response.mv.expect("有 MV");
        assert_eq!(mvs.len(), 1);
        assert_eq!(mvs[0].id, Some(123));
        assert_eq!(mvs[0].vid.as_deref(), Some("013xscuH0xlbie"));
        assert_eq!(mvs[0].kind, Some(1), "参考的 type 别名含 vt");
        assert_eq!(
            mvs[0].name.as_deref(),
            Some("MV 名"),
            "参考的 name 别名含 mvname"
        );
        assert_eq!(
            mvs[0].title.as_deref(),
            Some("MV 标题"),
            "参考的 title 别名是 title/title_main/name，不含 mvname"
        );
        assert_eq!(mvs[0].picurl.as_deref(), Some("https://y.gtimg.cn/mv.jpg"));
        assert_eq!(mvs[0].playcnt, Some(999));
        let singers = mvs[0].singers.as_ref().expect("有歌手");
        assert_eq!(singers[0].mid.as_deref(), Some("00255EBh1FcAEk"));
        assert_eq!(
            singers[0].picurl.as_deref(),
            Some("https://y.gtimg.cn/singer.jpg")
        );
        assert_eq!(singers[0].pmid.as_deref(), Some("p1"));

        let response: GetRelatedMvResponse =
            serde_json::from_value(related_mv_payload(&json!({ "hasmore": 0, "list": [] })))
                .expect("解析空回值");
        assert!(response.mv.expect("字段仍在").is_empty());
    }

    #[test]
    fn a_sheet_less_song_is_an_answer_not_an_error() {
        // 上游的「没有曲谱」：10007 + `result: null`。
        let payload = sheet_payload(&json!({ "result": null, "totalMap": {} }));
        let response: GetSheetResponse = serde_json::from_value(payload).expect("解析空曲谱");
        assert!(response.result.expect("字段仍在").is_empty());
        assert_eq!(response.total_map, Some(HashMap::new()));

        // call_signed 只对「10007 + 空 data」报错；那也要还原成空答案。
        assert!(no_sheet_code(&UpstreamError::Upstream(
            "上游返回错误（10007）：".into()
        )));
        assert!(!no_sheet_code(&UpstreamError::Upstream(
            "上游返回错误（2001）：".into()
        )));
        assert!(!no_sheet_code(&UpstreamError::Transport("超时".into())));
        let empty: GetSheetResponse =
            serde_json::from_value(empty_sheet_payload()).expect("解析空答案");
        assert!(empty.result.expect("字段仍在").is_empty());
    }

    #[test]
    fn every_sheet_field_maps_to_its_reference_name() {
        let payload = sheet_payload(&json!({
            "result": [{
                "scoreMID": "score-mid-1",
                "scoreName": "曲谱名",
                "picURLs": ["https://y.gtimg.cn/1.jpg"],
                "version": "原版",
                "tonality": 1,
                "scoreType": 0,
                "strScoreType": "用户上传",
                "uploader": "上传者",
                "viewFrequency": 123,
                "tonality2": 0,
                "author": "作者",
                "composer": "作曲",
                "lyricist": "作词",
                "singer": "演唱",
                "performer": "演奏",
                "songMID": "003w2xz20QlUZt",
                "subName": "",
                "url": "https://y.qq.com/score/1",
                "albumURL": "https://y.qq.com/album/1",
                "insType": 1,
                "strInsType": "钢琴",
                "coverURL": "https://y.gtimg.cn/cover.jpg",
                "difficulty": "简单",
                "sheetFile": "https://y.qq.com/sheet/1"
            }],
            "totalMap": { "0": 1, "1": "2" }
        }));
        let response: GetSheetResponse = serde_json::from_value(payload.clone()).expect("解析曲谱");
        // 先留一份宿主看到的形状，再从模型里读字段（读字段会把 `result` 移走）。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(response.total_map.as_ref().expect("有聚合")["1"], 2);
        // 宿主读到的键名与参考模型的字段名一致（camelCase）。
        assert_eq!(round["result"][0]["scoreMid"], "score-mid-1");
        assert_eq!(round["result"][0]["scoreTypeText"], "用户上传");
        assert_eq!(round["result"][0]["insTypeText"], "钢琴");
        assert_eq!(round["result"][0]["albumUrl"], "https://y.qq.com/album/1");
        assert_eq!(
            round["result"][0]["coverUrl"],
            "https://y.gtimg.cn/cover.jpg"
        );
        assert_eq!(round["result"][0]["picUrls"][0], "https://y.gtimg.cn/1.jpg");
        assert_eq!(round["totalMap"]["0"], 1);
        let sheets = response.result.expect("有曲谱");
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].score_mid.as_deref(), Some("score-mid-1"));
        assert_eq!(sheets[0].song_mid.as_deref(), Some("003w2xz20QlUZt"));
        assert_eq!(sheets[0].score_type_text.as_deref(), Some("用户上传"));
        assert_eq!(sheets[0].ins_type_text.as_deref(), Some("钢琴"));
        assert_eq!(
            sheets[0].album_url.as_deref(),
            Some("https://y.qq.com/album/1")
        );
        assert_eq!(sheets[0].view_frequency, Some(123));
        assert_eq!(
            sheets[0].pic_urls.as_deref(),
            Some(&["https://y.gtimg.cn/1.jpg".to_string()][..])
        );
        assert_eq!(
            sheets[0].sheet_file.as_deref(),
            Some("https://y.qq.com/sheet/1")
        );

        // `picURLs: null` 照参考的 NoneToEmptyList 收成空列表。
        let payload = sheet_payload(&json!({ "result": [{ "picURLs": null }], "totalMap": {} }));
        let response: GetSheetResponse = serde_json::from_value(payload).expect("解析");
        assert_eq!(
            response.result.expect("有曲谱")[0].pic_urls.as_deref(),
            Some(&[][..])
        );
    }

    #[test]
    fn has_sheet_flags_are_read_from_the_uppercase_spellings() {
        let payload = has_sheet_payload(&json!({
            "hasGuitar": true,
            "hasMore": false,
            "hasLDY": 1,
            "hasQRCX": "0",
            "hasChongChong": true
        }));
        let response: HasSheetMusicResponse =
            serde_json::from_value(payload.clone()).expect("解析");
        assert_eq!(response.has_guitar, Some(true));
        assert_eq!(response.has_more, Some(false));
        assert_eq!(response.has_ldy, Some(true), "上游键是 hasLDY");
        assert_eq!(response.has_qrcx, Some(false), "上游键是 hasQRCX");
        assert_eq!(response.has_chong_chong, Some(true));
        // 宿主按参考字段名的 camelCase 读。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["hasLdy"], true);
        assert_eq!(round["hasQrcx"], false);
        assert_eq!(round["hasChongChong"], true);
    }

    #[test]
    fn related_playlist_params_carry_the_last_batch() {
        assert_eq!(
            playlist_ids_of(&json!({ "vecPlaylist": [1, 2] })).unwrap(),
            vec![1, 2]
        );
        // 协议层换一批时也可能写成 last（宿主包装就是这么发的）。
        assert_eq!(
            playlist_ids_of(&json!({ "songid": 100, "last": [7011264340i64] })).unwrap(),
            vec![7011264340]
        );
        // 没给就是空列表——参考的 `last or []`。
        assert!(playlist_ids_of(&json!({ "songid": 100 }))
            .unwrap()
            .is_empty());
        // 参数形状照参考：`songid` 是数字，`vecPlaylist` 是数组。
        assert_eq!(
            related_songlist_params(100, vec![7, 8]),
            json!({ "songid": 100, "vecPlaylist": [7, 8] })
        );
        assert_eq!(
            related_songlist_params(100, Vec::new()),
            json!({ "songid": 100, "vecPlaylist": [] })
        );
    }

    #[test]
    fn related_mv_params_spell_songid_as_text_and_default_lastmvid_to_zero() {
        assert_eq!(last_mvid_of(&json!({ "songid": 100 })), json!(0));
        assert_eq!(
            last_mvid_of(&json!({ "lastMvid": "013xscuH0xlbie" })),
            json!("013xscuH0xlbie")
        );
        // 空字符串也算没给（参考的 `or 0`）。
        assert_eq!(last_mvid_of(&json!({ "lastmvid": "" })), json!(0));
        // 「换一批」的游标在参考里是数字的 MV id，数字原样放行。
        assert_eq!(last_mvid_of(&json!({ "lastmvid": 123 })), json!(123));

        // 参数形状：songid 是字符串、songtype 是 1、lastmvid 是 0 或 VID。
        assert_eq!(
            related_mv_params(100, last_mvid_of(&json!({}))),
            json!({ "songid": "100", "songtype": 1, "lastmvid": 0 })
        );
        assert_eq!(
            related_mv_params(100, last_mvid_of(&json!({ "lastmvid": "013xscuH0xlbie" }))),
            json!({ "songid": "100", "songtype": 1, "lastmvid": "013xscuH0xlbie" })
        );
    }

    #[test]
    fn similar_song_and_label_params_are_a_bare_songid() {
        assert_eq!(song_id_params(100), json!({ "songid": 100 }));
        // 协议层可能给数字字符串，也认。
        assert_eq!(song_id_of(&json!({ "songid": "100" })).unwrap(), 100);
    }

    #[test]
    fn sheet_requests_follow_the_reference_for_each_ttype() {
        // 默认档：GetMoreSheetMusic，scoreType -1，匿名 h5 comm 不带 platform。
        let (method, param, comm) = sheet_request("003w2xz20QlUZt", SHEET_TYPE_USER);
        assert_eq!(method, "GetMoreSheetMusic");
        assert_eq!(
            param,
            json!({
                "songMid": "003w2xz20QlUZt",
                "begin": 0,
                "end": 100,
                "scoreType": -1,
                "ttype": 0
            })
        );
        assert_eq!(comm["g_tk"], 5381);
        assert_eq!(comm["uin"], "");
        assert_eq!(comm["needNewCode"], 1);
        assert!(
            comm.get("platform").is_none(),
            "默认档的 comm 没有 platform"
        );

        // AI 档：同一个 method，scoreType -473。
        let (method, param, _) = sheet_request("m", SHEET_TYPE_AI);
        assert_eq!(method, "GetMoreSheetMusic");
        assert_eq!(param["scoreType"], -473);
        assert_eq!(param["ttype"], 1);

        // 虫虫钢琴档：另一个 method，ttype 固定 1，comm 带 platform h5。
        let (method, param, comm) = sheet_request("m", SHEET_TYPE_CHONGCHONG);
        assert_eq!(method, "GetChongChongSheetMusic");
        assert_eq!(
            param,
            json!({
                "songMid": "m",
                "begin": 0,
                "end": 100,
                "scoreType": -1,
                "ttype": 1
            })
        );
        assert_eq!(comm["platform"], "h5");
        assert_eq!(comm["g_tk"], 5381);
        // 两个 comm 都是参考那种 `override_comm=True` 的匿名 comm：uin 空、不需要登录。
        for (_, _, comm) in [
            sheet_request("m", 0),
            sheet_request("m", 1),
            sheet_request("m", 2),
        ] {
            assert_eq!(comm["uin"], "");
            assert_eq!(comm["format"], "json");
            assert_eq!(comm["inCharset"], "utf-8");
            assert_eq!(comm["outCharset"], "utf-8");
            assert_eq!(comm["notice"], 0);
        }
    }

    #[test]
    fn the_sheet_handle_is_a_mid_and_the_type_is_not_clamped() {
        assert_eq!(
            require_mid(&json!({ "mid": "003w2xz20QlUZt" })).unwrap(),
            "003w2xz20QlUZt"
        );
        assert!(require_mid(&json!({ "songMid": "003w2xz20QlUZt" })).is_ok());
        assert!(require_mid(&json!({ "mid": "" })).is_err());
        assert!(require_mid(&json!({})).is_err());
        // ttype 不做区间钳制：参考把调用方给的值原样传。
        assert_eq!(first_int(&json!({ "ttype": 9 }), &["ttype"]), Some(9));
    }
}

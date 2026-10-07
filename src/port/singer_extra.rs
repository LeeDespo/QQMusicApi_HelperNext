//! 歌手扩展：歌手列表（全量 / 索引分页）、相似歌手、主页 Tab、名称图片、歌手 MV。
//!
//! 当前公开契约见 `docs/endpoints.md`，解析与兼容规则见 `docs/parsing.md`。
//!
//! # 平台档案
//!
//! 参考里只有 `get_name_special_display` 标了 `platform=ANDROID`；其余五个没标，
//! 而参考 Client 的默认档案也是 ANDROID。本层按约定让没标的方法默认走调用方
//! 给的档案（组件默认 Web），但有两处按实测结论处理：
//!
//! * `fetch_artist_tab`：调用方没显式指定档案时用 android。实测 web 档案下
//!   `GetHomepageTabDetail` 回 `code 10000` 加一具空壳（`SongTab.List` 为 null），
//!   **不报错、只是没有内容**；android 档案才有内容。调用方显式给了 `platform`
//!   就尊重它（与既有 `fetch_artist_detail` 的处理同款）。
//! * `fetch_artist_display_name`：参考在 android 档案的 comm 上盖
//!   `{"cv": 20_080_000, "v": 20_080_000}`（实测 cv 14090008 时 DisplayType 恒为
//!   0）。组件的 `Upstream` 只有签名路能整块替换 comm，所以这条读走
//!   `musics.fcg`，见 [`display_comm`]。
//!
//! # 空值判据
//!
//! 参考里这六个接口都没有 `require_login`，读的是公开曲库：**空列表是答案**
//! （这个筛选条件下就是没有歌手、这个 Tab 就是空的），照常返回，不当故障。
//! 只有请求本身不成立的情况（缺 mid、枚举值不在参考的集合里、Tab 标识不认识）
//! 才报错——那几种上游多半会回一份"成功但空"的数据，静默空比报错更难查。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::upstream::{
    first_array, first_int, first_object, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "fetch_singer_list",
    "fetch_singer_index",
    "fetch_similar_artists",
    "fetch_artist_tab",
    "fetch_artist_display_name",
    "fetch_artist_mvs",
];

// MARK: - 参考枚举的取值

/// 参考 `AreaType`：地区筛选值。
const AREA_ALL: i64 = -100;
const AREA_CHINA: i64 = 200;
const AREA_TAIWAN: i64 = 2;
const AREA_AMERICA: i64 = 5;
const AREA_JAPAN: i64 = 4;
const AREA_KOREA: i64 = 3;
const AREAS: &[i64] = &[
    AREA_ALL,
    AREA_CHINA,
    AREA_TAIWAN,
    AREA_AMERICA,
    AREA_JAPAN,
    AREA_KOREA,
];

/// 参考 `GenreType`：风格筛选值。
const GENRE_ALL: i64 = -100;
const GENRE_POP: i64 = 7;
const GENRE_RAP: i64 = 3;
const GENRE_CHINESE_STYLE: i64 = 19;
const GENRE_ROCK: i64 = 4;
const GENRE_ELECTRONIC: i64 = 2;
const GENRE_FOLK: i64 = 8;
const GENRE_R_AND_B: i64 = 11;
const GENRE_ETHNIC: i64 = 37;
const GENRE_LIGHT_MUSIC: i64 = 93;
const GENRE_JAZZ: i64 = 14;
const GENRE_CLASSICAL: i64 = 33;
const GENRE_COUNTRY: i64 = 13;
const GENRE_BLUES: i64 = 10;
const GENRES: &[i64] = &[
    GENRE_ALL,
    GENRE_POP,
    GENRE_RAP,
    GENRE_CHINESE_STYLE,
    GENRE_ROCK,
    GENRE_ELECTRONIC,
    GENRE_FOLK,
    GENRE_R_AND_B,
    GENRE_ETHNIC,
    GENRE_LIGHT_MUSIC,
    GENRE_JAZZ,
    GENRE_CLASSICAL,
    GENRE_COUNTRY,
    GENRE_BLUES,
];

/// 参考 `SexType`：性别筛选值。
const SEX_ALL: i64 = -100;
const SEX_MALE: i64 = 0;
const SEX_FEMALE: i64 = 1;
const SEX_GROUP: i64 = 2;
const SEXES: &[i64] = &[SEX_ALL, SEX_MALE, SEX_FEMALE, SEX_GROUP];

/// 参考 `IndexType`：首字母索引值（1..=26 是 A..Z，27 是 `#`，-100 是全部）。
const INDEX_ALL: i64 = -100;
const INDEX_HASH: i64 = 27;
const INDEXES: &[i64] = &[
    INDEX_ALL, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
    24, 25, 26, INDEX_HASH,
];

/// 参考 `TabType` 的 `tab_id`（注释里是参考的 `tab_name`）。
const TAB_IDS: &[&str] = &[
    "wiki",           // IntroductionTab
    "album",          // AlbumTab
    "song_composing", // SongTab（作曲）
    "song_lyric",     // SongTab（作词）
    "producer",       // SongTab（制作人）
    "arranger",       // SongTab（编曲）
    "musician",       // SongTab（演奏）
    "song_sing",      // SongTab（演唱）
    "video",          // VideoTab
];

/// 参考 `get_singer_list_index` 的 `num` 缺省。
const INDEX_PAGE_SIZE: i64 = 80;
/// 参考 `get_similar` 的 `number` 缺省。
const SIMILAR_NUMBER: i64 = 10;
/// 参考 `get_tab_detail` 的 `num` 缺省。
const TAB_PAGE_SIZE: i64 = 10;
/// 参考 `get_mv_list` 的 `num` 缺省。
const MV_PAGE_SIZE: i64 = 10;

// MARK: - 模型

/// 参考 `TagOption`：歌手筛选标签项.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerTag {
    pub id: Option<i64>,
    pub name: Option<String>,
}

/// 参考 `SingerBrief`：歌手列表条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerBrief {
    /// 歌手数字 ID（参考别名 `singer_id` / `singerId` / `id`）.
    pub id: Option<i64>,
    /// 歌手 MID（别名 `singer_mid` / `singerMid` / `mid`）.
    pub mid: Option<String>,
    /// 歌手名称（别名 `singer_name` / `singerName` / `name`）.
    pub name: Option<String>,
    /// 图片标识（别名 `singer_pmid` / `singerPmid` / `pmid`）.
    pub pmid: Option<String>,
    pub area_id: Option<i64>,
    pub country_id: Option<i64>,
    pub country: Option<String>,
    pub other_name: Option<String>,
    /// 拼音.
    pub spell: Option<String>,
    /// 趋势标记.
    pub trend: Option<i64>,
    /// 关注数（上游键就是 `concernNum`）.
    pub concern_num: Option<i64>,
    /// 歌手图片地址.
    pub singer_pic: Option<String>,
}

/// 参考 `SingerTagData`：歌手筛选标签集合.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerTagData {
    pub area: Option<Vec<SingerTag>>,
    pub genre: Option<Vec<SingerTag>>,
    pub sex: Option<Vec<SingerTag>>,
    pub index: Option<Vec<SingerTag>>,
}

/// 参考 `SingerTypeListResponse`：歌手列表接口的响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerTypeList {
    /// 当前地区筛选值.
    pub area: Option<i64>,
    /// 当前性别筛选值.
    pub sex: Option<i64>,
    /// 当前流派筛选值.
    pub genre: Option<i64>,
    /// 当前返回的歌手列表.
    pub singerlist: Option<Vec<SingerBrief>>,
    pub code: Option<i64>,
    /// 热门歌手列表.
    pub hotlist: Option<Vec<SingerBrief>>,
    /// 可选筛选标签集合.
    pub tags: Option<SingerTagData>,
}

/// 参考 `SingerIndexPageResponse`：按索引分页的歌手列表响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerIndexPage {
    pub area: Option<i64>,
    pub sex: Option<i64>,
    pub genre: Option<i64>,
    pub singerlist: Option<Vec<SingerBrief>>,
    pub code: Option<i64>,
    pub hotlist: Option<Vec<SingerBrief>>,
    pub tags: Option<SingerTagData>,
    /// 当前索引筛选值.
    pub index: Option<i64>,
    /// 总数量（宿主据此翻页）.
    pub total: Option<i64>,
}

/// 参考 `SimilarSinger`：相似歌手条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SimilarSinger {
    pub id: Option<i64>,
    pub mid: Option<String>,
    pub name: Option<String>,
    /// 图片标识（参考别名 `pic_mid`）.
    pub pmid: Option<String>,
    pub singer_pic: Option<String>,
    /// 追踪信息.
    pub trace: Option<String>,
    /// 补充文案.
    pub abt: Option<String>,
    /// 附加标记.
    pub tf: Option<String>,
}

/// 参考 `SimilarSingerResponse`：相似歌手列表接口的响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SimilarSingerList {
    pub singerlist: Option<Vec<SimilarSinger>>,
    pub code: Option<i64>,
    /// 错误消息（上游键 `errMsg`）.
    pub err_msg: Option<String>,
}

/// 参考 `TabMeta`：主页标签元信息.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TabMeta {
    pub tab_id: Option<String>,
    pub tab_name: Option<String>,
    pub title: Option<String>,
}

/// 参考 `AlbumBrief`：主页专辑 Tab 里的专辑条目（别名与参考模型一致）.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerAlbumBrief {
    /// 专辑 ID（上游键是 `albumID`，大写 ID）.
    pub id: Option<i64>,
    pub mid: Option<String>,
    pub name: Option<String>,
    /// 专辑副标题（上游键 `albumTranName`）.
    pub subtitle: Option<String>,
    /// 发行日期.
    pub time_public: Option<String>,
    /// 曲目数.
    pub total_num: Option<i64>,
    /// 专辑类型文案.
    pub album_type: Option<String>,
    pub singer_name: Option<String>,
    pub tags: Option<Vec<String>>,
}

/// 参考 `VideoBrief`：歌手 MV / 视频条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerVideoBrief {
    /// MV ID（参考别名 `mvid`）.
    pub id: Option<i64>,
    pub vid: Option<String>,
    /// MV 类型（参考字段名就是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    pub title: Option<String>,
    pub picurl: Option<String>,
    pub picformat: Option<i64>,
    pub duration: Option<i64>,
    pub playcnt: Option<i64>,
    pub pubdate: Option<i64>,
    /// 图标类型（上游键 `icon_type`）.
    pub icon_type: Option<i64>,
}

/// 参考 `HomepageTabDetailResponse`：歌手主页标签页详情.
///
/// `introduction_tab` 在参考里是 `list[dict]`（原样透传的简介结构），这里存
/// JSON 文本：`#[data]` 认不出 `serde_json::Value`，而经 serde 进出时它仍是
/// 真正的对象数组（见 `raw_json_serialize`）。`song_tab` 的行就是曲目本体
/// （参考 jsonpath 是 `$.SongTab.List[*]`），复用组件既有的曲目模型，
/// 宿主在歌手页与「歌手歌曲」看到同一种行。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HomepageTabDetail {
    /// 当前标签页 ID（上游键 `TabID`）.
    pub tab_id: Option<String>,
    /// 是否还有更多结果.
    pub has_more: Option<i64>,
    /// 是否需要展示标签.
    pub need_show_tab: Option<i64>,
    pub order: Option<i64>,
    /// 标签页元信息列表（上游键 `TabList`）.
    pub tab_list: Option<Vec<TabMeta>>,
    /// 简介标签内容（原样透传，JSON 文本）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub introduction_tab: Option<String>,
    /// 歌曲标签内容.
    pub song_tab: Option<Vec<crate::models::Track>>,
    /// 专辑标签内容.
    pub album_tab: Option<Vec<SingerAlbumBrief>>,
    /// 视频标签内容.
    pub video_tab: Option<Vec<SingerVideoBrief>>,
}

/// 参考 `SingerNameSpecialDisplayResponse`：歌手名称透明 PNG 展示信息.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerNameSpecialDisplay {
    /// 展示类型：2 表示名称图片，0 表示无特殊展示.
    pub display_type: Option<i64>,
    /// 透明 PNG 地址；无名称图片时为空.
    pub pic_file: Option<String>,
    /// 签名与名称重叠比例.
    pub signature_name_overlap_ratio: Option<f64>,
    /// 歌手名称.
    pub name: Option<String>,
}

/// 参考 `SingerMvListResponse`：歌手 MV 列表接口的响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerMvList {
    /// MV 总数.
    pub total: Option<i64>,
    /// 当前页 MV 列表（参考字段名 `mv_list`，上游键是 `list`）.
    pub mv_list: Option<Vec<SingerVideoBrief>>,
}

// MARK: - 参考模型里 `dict` 形状的字段

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

// MARK: - 回值搬运（上游形状 → 参考模型形状）

/// 第一个出现的浮点键（`SignatureNameOverlapRatio` 有时给整数 0）。
fn first_float(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    })
}

/// 一组字符串（上游有时给数字，专辑 Tab 的 `tags` 也可能为 null）。
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

/// 封面地址统一转 https（docs/parsing.md §7：宿主会直接拒绝 http 图片）。
fn artwork_url(value: Option<String>) -> Option<String> {
    crate::methods::normalized_artwork_url(value.as_deref())
}

/// 单条歌手（参考 `SingerBrief`）。
fn singer_brief_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["singer_id", "singerId", "id"]),
        "mid": first_text(raw, &["singer_mid", "singerMid", "mid"]),
        "name": first_text(raw, &["singer_name", "singerName", "name"]),
        "pmid": first_text(raw, &["singer_pmid", "singerPmid", "pmid"]),
        "areaId": first_int(raw, &["area_id", "areaId"]),
        "countryId": first_int(raw, &["country_id", "countryId"]),
        "country": first_text(raw, &["country"]),
        "otherName": first_text(raw, &["other_name", "otherName"]),
        "spell": first_text(raw, &["spell"]),
        "trend": first_int(raw, &["trend"]),
        "concernNum": first_int(raw, &["concernNum", "concern_num"]),
        "singerPic": artwork_url(first_text(raw, &["singer_pic", "singerPic"])),
    })
}

/// 歌手条目列表（`singerlist` / `hotlist` 共用）。
fn singer_list_field(raw: &Value, keys: &[&str]) -> Option<Vec<Value>> {
    first_array(raw, keys).map(|items| items.iter().map(singer_brief_payload).collect())
}

/// 单个筛选标签（参考 `TagOption`）。
fn singer_tag_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id"]),
        "name": first_text(raw, &["name"]),
    })
}

/// 筛选标签集合（参考 `SingerTagData`）；某一类缺失时保持 null。
fn singer_tags_payload(data: &Value) -> Value {
    let tags = first_object(data, &["tags", "Tags"])
        .cloned()
        .unwrap_or(json!({}));
    let one = |key: &str| {
        first_array(&tags, &[key])
            .map(|items| items.iter().map(singer_tag_payload).collect::<Vec<_>>())
    };
    json!({
        "area": one("area"),
        "genre": one("genre"),
        "sex": one("sex"),
        "index": one("index"),
    })
}

/// 歌手列表回值（参考 `SingerTypeListResponse`）。
fn singer_type_list_payload(data: &Value) -> Value {
    json!({
        "area": first_int(data, &["area"]),
        "sex": first_int(data, &["sex"]),
        "genre": first_int(data, &["genre"]),
        "singerlist": singer_list_field(data, &["singerlist", "singerList"]),
        "code": first_int(data, &["code"]),
        "hotlist": singer_list_field(data, &["hotlist", "hotList"]),
        "tags": singer_tags_payload(data),
    })
}

/// 索引分页回值（参考 `SingerIndexPageResponse`）。
fn singer_index_payload(data: &Value) -> Value {
    let mut payload = singer_type_list_payload(data);
    payload["index"] = json!(first_int(data, &["index"]));
    payload["total"] = json!(first_int(data, &["total"]));
    payload
}

/// 单条相似歌手（参考 `SimilarSinger`）。
fn similar_singer_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["singerId", "singer_id", "id"]),
        "mid": first_text(raw, &["singerMid", "singer_mid", "mid"]),
        "name": first_text(raw, &["singerName", "singer_name", "name"]),
        "pmid": first_text(raw, &["pic_mid", "pmid"]),
        "singerPic": artwork_url(first_text(raw, &["singerPic", "singer_pic"])),
        "trace": first_text(raw, &["trace"]),
        "abt": first_text(raw, &["abt"]),
        "tf": first_text(raw, &["tf"]),
    })
}

/// 相似歌手回值（参考 `SimilarSingerResponse`）。
fn similar_singer_list_payload(data: &Value) -> Value {
    json!({
        "singerlist": first_array(data, &["singerlist", "singerList"])
            .map(|items| items.iter().map(similar_singer_payload).collect::<Vec<_>>()),
        "code": first_int(data, &["code"]),
        "errMsg": first_text(data, &["errMsg", "err_msg", "msg"]),
    })
}

/// 单个标签页元信息（参考 `TabMeta`）。
fn tab_meta_payload(raw: &Value) -> Value {
    json!({
        "tabId": first_text(raw, &["TabID", "tabId"]),
        "tabName": first_text(raw, &["TabName", "tabName"]),
        "title": first_text(raw, &["Title", "title"]),
    })
}

/// 单张专辑（参考 `AlbumBrief`）。
fn album_brief_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["albumID", "albumId", "id"]),
        "mid": first_text(raw, &["albumMid", "mid"]),
        "name": first_text(raw, &["albumName", "name"]),
        "subtitle": first_text(raw, &["albumTranName", "subtitle"]),
        "timePublic": first_text(raw, &["publishDate", "timePublic", "time_public"]),
        "totalNum": first_int(raw, &["totalNum", "total_num"]),
        "albumType": first_text(raw, &["albumType", "album_type"]),
        "singerName": first_text(raw, &["singerName", "singer_name"]),
        "tags": string_list(raw, &["tags"]),
    })
}

/// 单个 MV / 视频（参考 `VideoBrief`）。
fn video_brief_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["mvid", "id"]),
        "vid": first_text(raw, &["vid"]),
        "type": first_int(raw, &["type", "vt"]),
        "title": first_text(raw, &["title", "name"]),
        "picurl": artwork_url(first_text(raw, &["picurl", "picUrl"])),
        "picformat": first_int(raw, &["picformat"]),
        "duration": first_int(raw, &["duration"]),
        "playcnt": first_int(raw, &["playcnt"]),
        "pubdate": first_int(raw, &["pubdate"]),
        "iconType": first_int(raw, &["icon_type", "iconType"]),
    })
}

/// 主页 Tab 详情回值（参考 `HomepageTabDetailResponse`）。
///
/// 标签页容器里的内容键逐个找：`IntroductionTab.List`、`SongTab.List`、
/// `AlbumTab.AlbumList`、`VideoTab.VideoList`（参考模型的 jsonpath 就是这些）。
/// 四个内容字段与 `tab_list` 都保持 `Option`：**上游没给与给了空是两件事**
/// （与 `mv.rs` 的 `items` 同款判据）——wiki 这类 Tab 没有 `SongTab.List`，
/// 而一个空的歌曲页会给 `[]`。
fn homepage_tab_payload(data: &Value) -> Value {
    let introduction = first_object(data, &["IntroductionTab", "introductionTab"])
        .and_then(|tab| tab.get("List"))
        .filter(|value| !value.is_null());
    let songs = first_object(data, &["SongTab", "songTab"])
        .and_then(|tab| first_array(tab, &["List", "list"]))
        .map(|items| {
            // SongTab 的行是曲目本体，用组件既有的曲目解码（`SongTab.List` 不是
            // `decoded_tracks` 的候选键，这里换个键名喂给它）。
            crate::methods::decoded_tracks(&json!({ "songList": items }))
        });
    let albums = first_object(data, &["AlbumTab", "albumTab"])
        .and_then(|tab| first_array(tab, &["AlbumList", "albumList"]))
        .map(|items| items.iter().map(album_brief_payload).collect::<Vec<_>>());
    let videos = first_object(data, &["VideoTab", "videoTab"])
        .and_then(|tab| first_array(tab, &["VideoList", "videoList"]))
        .map(|items| items.iter().map(video_brief_payload).collect::<Vec<_>>());
    json!({
        "tabId": first_text(data, &["TabID", "tabId"]),
        "hasMore": first_int(data, &["HasMore", "hasMore"]),
        "needShowTab": first_int(data, &["NeedShowTab", "needShowTab"]),
        "order": first_int(data, &["Order", "order"]),
        "tabList": first_array(data, &["TabList", "tabList"])
            .map(|items| items.iter().map(tab_meta_payload).collect::<Vec<_>>()),
        "introductionTab": introduction,
        "songTab": songs,
        "albumTab": albums,
        "videoTab": videos,
    })
}

/// 名称图片回值（参考 `SingerNameSpecialDisplayResponse`）。
fn special_display_payload(data: &Value) -> Value {
    let singer = first_object(data, &["Info", "info"])
        .and_then(|info| first_object(info, &["Singer", "singer"]));
    let special = singer.and_then(|singer| {
        first_object(
            singer,
            &["SingerNameSpecialDisplay", "singerNameSpecialDisplay"],
        )
    });
    json!({
        "displayType": special.and_then(|special| first_int(special, &["DisplayType", "displayType"])),
        "picFile": special.and_then(|special| first_text(special, &["PicFile", "picFile"])),
        "signatureNameOverlapRatio": special.and_then(|special| {
            first_float(special, &["SignatureNameOverlapRatio", "signatureNameOverlapRatio"])
        }),
        // `name` 在 `Singer` 自己身上，不在 `SingerNameSpecialDisplay` 里。
        "name": singer.and_then(|singer| first_text(singer, &["Name", "name"])),
    })
}

/// MV 列表回值（参考 `SingerMvListResponse`）：上游把列表放在 `list` 上。
fn singer_mv_payload(data: &Value) -> Value {
    json!({
        "total": first_int(data, &["total", "Total"]),
        "mvList": first_array(data, &["list", "List"])
            .map(|items| items.iter().map(video_brief_payload).collect::<Vec<_>>()),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 列表参数（参考 `get_singer_list`）；`hastag` 的拼写照参考逐字抄。
fn singer_list_params(area: i64, sex: i64, genre: i64) -> Value {
    json!({ "hastag": 0, "area": area, "sex": sex, "genre": genre })
}

/// 索引分页参数（参考 `get_singer_list_index`）。
///
/// `sin` 是偏移、`cur_page` 是页码，两者都**不做区间钳制**——参考把调用方给的
/// 页码原样算（`page` 给 0 时 `sin` 就是负的）。
fn singer_index_params(area: i64, sex: i64, genre: i64, index: i64, page: i64, num: i64) -> Value {
    json!({
        "area": area,
        "sex": sex,
        "genre": genre,
        "index": index,
        "sin": (page - 1) * num,
        "cur_page": page,
    })
}

/// 相似歌手参数（参考 `get_similar`）。
fn similar_params(mid: &str, number: i64) -> Value {
    json!({ "singerMid": mid, "number": number })
}

/// 主页 Tab 参数（参考 `get_tab_detail`）：`PageNum` 就是 `page - 1`。
fn tab_params(mid: &str, tab_id: &str, page: i64, num: i64) -> Value {
    json!({
        "SingerMid": mid,
        "IsQueryTabDetail": 1,
        "TabID": tab_id,
        "PageNum": page - 1,
        "PageSize": num,
        "Order": 0,
    })
}

/// 歌手 MV 参数（参考 `get_mv_list`）：页码换算成偏移量 `start`。
fn singer_mv_params(mid: &str, num: i64, page: i64) -> Value {
    json!({
        "singermid": mid,
        "order": 1,
        "count": num,
        "start": (page - 1) * num,
    })
}

/// 参考的 IntEnum 语义：值不在集合里就报错。
///
/// 上游对不认识的筛选值不会报错，只会回一份"成功但空"的结果；参考的
/// `AreaType(area)` 则会直接抛异常，这里跟参考一样拒绝。
fn enum_value(
    params: &Value,
    keys: &[&str],
    default: i64,
    allowed: &[i64],
    what: &str,
) -> Result<i64, UpstreamError> {
    let value = first_int(params, keys).unwrap_or(default);
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(UpstreamError::Upstream(format!("不支持的{what}：{value}")))
    }
}

/// 页码：参考的缺省是 1。
fn page_of(params: &Value, default: i64) -> i64 {
    first_int(params, &["page", "pageNum"]).unwrap_or(default)
}

/// 每页数量：参考叫 `num`，驱动组件的调用方也常用 `limit`。
fn size_of(params: &Value, default: i64) -> i64 {
    first_int(params, &["num", "limit", "size"]).unwrap_or(default)
}

/// 必填的歌手 MID。
fn require_mid(params: &Value) -> Result<String, UpstreamError> {
    first_text(params, &["singerMid", "mid"])
        .ok_or_else(|| UpstreamError::Upstream("缺少歌手 MID（singerMid）".into()))
}

/// 主页 Tab 的标识：参考 `TabType.tab_id`，是一组固定的字符串，不是数字。
///
/// 上游对不认识的 TabID 同样不报错、只回空壳，所以这里先对着参考的集合校验。
/// 数字 1..=9 按参考 `TabType` 的声明次序收（1=wiki … 9=video）：这是给宿主
/// 的便利写法（`scripts/port-smoke.sh` 就是这么传的），协议上仍然发字符串。
fn require_tab_id(params: &Value) -> Result<String, UpstreamError> {
    let value = params
        .get("tabType")
        .or_else(|| params.get("tabId"))
        .or_else(|| params.get("tab_id"));
    match value {
        Some(Value::String(text)) if TAB_IDS.contains(&text.as_str()) => Ok(text.clone()),
        Some(Value::Number(number)) => match number.as_i64() {
            Some(position) if (1..=TAB_IDS.len() as i64).contains(&position) => {
                Ok(TAB_IDS[(position - 1) as usize].to_string())
            }
            _ => Err(UpstreamError::Upstream(format!(
                "不支持的 Tab 类型：{number}（可用序号 1..={}，或字符串：{}）",
                TAB_IDS.len(),
                TAB_IDS.join("、")
            ))),
        },
        _ => Err(UpstreamError::Upstream(format!(
            "缺少或不支持的 Tab 类型（可用字符串：{}；或按参考 TabType 的声明次序用 1..={}）",
            TAB_IDS.join("、"),
            TAB_IDS.len()
        ))),
    }
}

/// `fetch_artist_tab` 用的档案：调用方显式给了就用它，否则 android。
///
/// 参考没给这个端点标档案，但参考 Client 的默认档案是 ANDROID；实测 web 档案下
/// 端点回 `code 10000` + 空壳（不报错），android 档案才有内容。
fn tab_platform(params: &Value, platform: Platform) -> Platform {
    if params.get("platform").is_some() {
        platform
    } else {
        Platform::Android
    }
}

/// `fetch_artist_display_name` 的 comm：在 android 档案的 comm 上盖
/// `{"cv": 20_080_000, "v": 20_080_000}`（参考就是这么传的；实测 cv 14090008
/// 时 `DisplayType` 恒为 0）。
///
/// 组件没有"合并 comm"的入口，只有 `Upstream::call_signed` 能整块替换 comm，
/// 所以这里按 android 档案重建一份带覆盖的 comm。android 档案用 `qq`/`authst`
/// 认账号（不是 `uin`/`g_tk`），登录类型照凭据走、缺省按 QQ 的 1。
fn display_comm(credential: &Credential) -> Value {
    let mut comm = json!({
        "format": "json",
        "inCharset": "utf-8",
        "outCharset": "utf-8",
        "notice": 0,
        "needNewCode": 1,
        "platform": "yqq.json",
        "ct": 11,
        "cv": 20_080_000,
        "v": 20_080_000,
        "tmeAppID": "qqmusic",
        "tmeLoginType": first_int(&credential.raw, &["loginType", "login_type"]).unwrap_or(1),
        "chid": "10003505",
    });
    if credential.is_usable() {
        comm["qq"] = json!(credential.music_id);
        comm["authst"] = json!(credential.music_key);
    }
    comm
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
        "fetch_singer_list" => singer_list(upstream, credential, platform, params),
        "fetch_singer_index" => singer_index(upstream, credential, platform, params),
        "fetch_similar_artists" => similar_artists(upstream, credential, platform, params),
        "fetch_artist_tab" => artist_tab(upstream, credential, platform, params),
        "fetch_artist_display_name" => artist_display_name(upstream, credential, platform, params),
        "fetch_artist_mvs" => artist_mvs(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 歌手列表（`music.musichallSinger.SingerList / GetSingerList`）。
///
/// 空列表是答案：这个筛选条件下就是没有歌手，照常返回。
fn singer_list(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let area = enum_value(params, &["area"], AREA_ALL, AREAS, "地区（AreaType）")?;
    let sex = enum_value(params, &["sex"], SEX_ALL, SEXES, "性别（SexType）")?;
    let genre = enum_value(params, &["genre"], GENRE_ALL, GENRES, "风格（GenreType）")?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.musichallSinger.SingerList",
            method: "GetSingerList",
            param: singer_list_params(area, sex, genre),
        },
    )?;
    Ok(singer_type_list_payload(&data))
}

/// 索引分页歌手列表（`music.musichallSinger.SingerList / GetSingerListIndex`）。
///
/// 空列表是答案（这个字母下没有歌手）；翻页是 `sin` + `cur_page` 两个字段，
/// 宿主拿回值的 `total` 自己算下一页。
fn singer_index(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let area = enum_value(params, &["area"], AREA_ALL, AREAS, "地区（AreaType）")?;
    let sex = enum_value(params, &["sex"], SEX_ALL, SEXES, "性别（SexType）")?;
    let genre = enum_value(params, &["genre"], GENRE_ALL, GENRES, "风格（GenreType）")?;
    let index = enum_value(
        params,
        &["index"],
        INDEX_ALL,
        INDEXES,
        "首字母索引（IndexType）",
    )?;
    let page = page_of(params, 1);
    let num = size_of(params, INDEX_PAGE_SIZE);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.musichallSinger.SingerList",
            method: "GetSingerListIndex",
            param: singer_index_params(area, sex, genre, index, page, num),
        },
    )?;
    Ok(singer_index_payload(&data))
}

/// 相似歌手（`music.SimilarSingerSvr / GetSimilarSingerList`）。
///
/// 空列表是答案：上游对这位歌手就是没有相似歌手。
fn similar_artists(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let mid = require_mid(params)?;
    let number = first_int(params, &["number", "limit", "num"]).unwrap_or(SIMILAR_NUMBER);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.SimilarSingerSvr",
            method: "GetSimilarSingerList",
            param: similar_params(&mid, number),
        },
    )?;
    Ok(similar_singer_list_payload(&data))
}

/// 歌手主页 Tab（`music.UnifiedHomepage.UnifiedHomepageSrv / GetHomepageTabDetail`）。
///
/// 参考是 `pager=True` 的 PageStrategy：调用方给页码，`PageNum` 传 `page - 1`；
/// 回值 `hasMore` 决定还有没有下一页。
fn artist_tab(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let mid = require_mid(params)?;
    let tab_id = require_tab_id(params)?;
    let page = page_of(params, 1);
    let num = size_of(params, TAB_PAGE_SIZE);
    let data = upstream.call_with(
        credential,
        Class::Read,
        tab_platform(params, platform),
        Call {
            module: "music.UnifiedHomepage.UnifiedHomepageSrv",
            method: "GetHomepageTabDetail",
            param: tab_params(&mid, &tab_id, page, num),
        },
    )?;
    Ok(homepage_tab_payload(&data))
}

/// 歌手名称特殊展示（`music.UnifiedHomepage.UnifiedHomepageSrv / GetHomepageHeader`）。
///
/// 与 `fetch_artist_detail` 打同一个端点，但要的是 `SingerNameSpecialDisplay`
/// 那一段，而且 comm 要盖 `cv/v = 20080000`（见 [`display_comm`]）。无名称图片
/// 时上游回 `DisplayType 0` 与空 `PicFile`——那是答案，不是故障。
///
/// 但整块 `Singer` 是空壳（连 `Name` 都没有）不是答案：那是档案/参数不对时
/// 端点回的"成功但空"（实测 web 档案正是 `code 10000` + 空壳），报错比伪装成
/// 「这位歌手没有名称图片」更容易查——既有 `fetch_artist_detail` 对同一形状
/// 也是这么处理的。
fn artist_display_name(
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
            module: "music.UnifiedHomepage.UnifiedHomepageSrv",
            method: "GetHomepageHeader",
            param: json!({ "SingerMid": mid }),
        },
        &[],
        Some(display_comm(credential)),
    )?;
    if empty_singer_shell(&data) {
        return Err(UpstreamError::Upstream(
            "上游返回了空的歌手资料（名称展示读不到 SingerNameSpecialDisplay）".into(),
        ));
    }
    Ok(special_display_payload(&data))
}

/// 回值里的 `Info.Singer` 是不是空壳：连 `Name` 与 `SingerMid` 都没有。
///
/// 单独成函数是为了能不打网络地测出来——这正是「成功但空」与「这位歌手没有
/// 名称图片」的分界。
fn empty_singer_shell(data: &Value) -> bool {
    match first_object(data, &["Info", "info"])
        .and_then(|info| first_object(info, &["Singer", "singer"]))
    {
        Some(singer) => {
            first_text(singer, &["Name", "name"]).is_none()
                && first_text(singer, &["SingerMid", "singerMid", "mid"]).is_none()
        }
        None => true,
    }
}

/// 歌手 MV（`MvService.MvInfoProServer / GetSingerMvList`）。
///
/// 空列表是答案：这位歌手就是没有 MV；总数在 `total`，宿主据此翻页。
fn artist_mvs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let mid = require_mid(params)?;
    let num = size_of(params, MV_PAGE_SIZE);
    let page = page_of(params, 1);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "MvService.MvInfoProServer",
            method: "GetSingerMvList",
            param: singer_mv_params(&mid, num, page),
        },
    )?;
    Ok(singer_mv_payload(&data))
}

// MARK: - 宿主包装

/// 歌手列表；省略的筛选值按参考的缺省（都是 -100 = 全部）。
#[export]
pub fn fetch_singer_list(
    area: Option<i64>,
    sex: Option<i64>,
    genre: Option<i64>,
) -> Result<SingerTypeList, crate::HelperError> {
    let mut params = json!({});
    if let Some(area) = area {
        params["area"] = json!(area);
    }
    if let Some(sex) = sex {
        params["sex"] = json!(sex);
    }
    if let Some(genre) = genre {
        params["genre"] = json!(genre);
    }
    crate::port::call("fetch_singer_list", params)
}

/// 按首字母索引分页的歌手列表；`page` 缺省 1、`num` 缺省 80。
/// 翻页时把回值的 `total` 与当前 `sin`/`cur_page` 对上再取下一页。
#[export]
pub fn fetch_singer_index(
    area: Option<i64>,
    sex: Option<i64>,
    genre: Option<i64>,
    index: Option<i64>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<SingerIndexPage, crate::HelperError> {
    let mut params = json!({});
    if let Some(area) = area {
        params["area"] = json!(area);
    }
    if let Some(sex) = sex {
        params["sex"] = json!(sex);
    }
    if let Some(genre) = genre {
        params["genre"] = json!(genre);
    }
    if let Some(index) = index {
        params["index"] = json!(index);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_singer_index", params)
}

/// 相似歌手；`number` 缺省 10。
#[export]
pub fn fetch_similar_artists(
    singer_mid: String,
    number: Option<i64>,
) -> Result<SimilarSingerList, crate::HelperError> {
    let mut params = json!({ "singerMid": singer_mid });
    if let Some(number) = number {
        params["number"] = json!(number);
    }
    crate::port::call("fetch_similar_artists", params)
}

/// 歌手主页某个 Tab 的一页；`tab_type` 是参考 `TabType` 的字符串标识
/// （`wiki` / `album` / `song_sing` / `video` / `song_composing` / …）。
#[export]
pub fn fetch_artist_tab(
    singer_mid: String,
    tab_type: String,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<HomepageTabDetail, crate::HelperError> {
    let mut params = json!({ "singerMid": singer_mid, "tabType": tab_type });
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_artist_tab", params)
}

/// 歌手名称透明 PNG；无名称图片时 `displayType` 为 0、`picFile` 为空。
#[export]
pub fn fetch_artist_display_name(
    singer_mid: String,
) -> Result<SingerNameSpecialDisplay, crate::HelperError> {
    crate::port::call(
        "fetch_artist_display_name",
        json!({ "singerMid": singer_mid }),
    )
}

/// 歌手 MV 一页；`num` 缺省 10、`page` 缺省 1，总数在 `total`。
#[export]
pub fn fetch_artist_mvs(
    singer_mid: String,
    num: Option<i64>,
    page: Option<i64>,
) -> Result<SingerMvList, crate::HelperError> {
    let mut params = json!({ "singerMid": singer_mid });
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    crate::port::call("fetch_artist_mvs", params)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份歌手列表回值，键名照上游自己的拼写（实测响应裁出来的）。
    fn raw_singer_page() -> Value {
        json!({
            "area": 200,
            "sex": 0,
            "genre": 7,
            "code": 0,
            "singerlist": [{
                "area_id": 0,
                "singer_id": 4558,
                "country_id": 0,
                "singer_name": "周杰伦",
                "country": "",
                "other_name": "Jay Chou",
                "singer_mid": "0025NhlN2yWrP4",
                "spell": "zhoujielun",
                "trend": 0,
                "singer_pmid": "0025NhlN2yWrP4_11",
                "concernNum": 50826458,
                "singer_pic": "http://y.gtimg.cn/music/photo_new/T001R300x300M0000025NhlN2yWrP4.webp"
            }],
            "hotlist": [{
                "singer_id": 4286,
                "singer_mid": "001BLpXF2DyJe2",
                "singer_name": "林俊杰",
                "singer_pmid": "001BLpXF2DyJe2_13",
                "concernNum": 25888413,
                "singer_pic": ""
            }],
            "tags": {
                "area": [{"id": -100, "name": "全部"}, {"id": 200, "name": "内地"}],
                "genre": [{"id": 7, "name": "流行"}],
                "sex": [{"id": 0, "name": "男"}],
                "index": [{"id": 1, "name": "A"}]
            }
        })
    }

    #[test]
    fn the_list_request_is_the_reference_one() {
        assert_eq!(
            singer_list_params(AREA_CHINA, SEX_MALE, GENRE_POP),
            json!({"hastag": 0, "area": 200, "sex": 0, "genre": 7})
        );
        assert_eq!(
            singer_list_params(AREA_ALL, SEX_ALL, GENRE_ALL),
            json!({"hastag": 0, "area": -100, "sex": -100, "genre": -100}),
            "参考的缺省是全部"
        );
    }

    #[test]
    fn the_index_request_turns_page_into_sin_and_cur_page() {
        assert_eq!(
            singer_index_params(-100, -100, -100, -100, 1, 80),
            json!({
                "area": -100, "sex": -100, "genre": -100, "index": -100,
                "sin": 0, "cur_page": 1
            })
        );
        assert_eq!(
            singer_index_params(AREA_CHINA, SEX_FEMALE, GENRE_POP, 26, 2, 80),
            json!({
                "area": 200, "sex": 1, "genre": 7, "index": 26,
                "sin": 80, "cur_page": 2
            })
        );
    }

    #[test]
    fn every_request_carries_its_reference_parameters() {
        assert_eq!(
            similar_params("0025NhlN2yWrP4", 5),
            json!({"singerMid": "0025NhlN2yWrP4", "number": 5})
        );
        assert_eq!(
            tab_params("0025NhlN2yWrP4", "song_sing", 3, 10),
            json!({
                "SingerMid": "0025NhlN2yWrP4",
                "IsQueryTabDetail": 1,
                "TabID": "song_sing",
                "PageNum": 2,
                "PageSize": 10,
                "Order": 0
            }),
            "参考把页码换算成 PageNum = page - 1"
        );
        assert_eq!(
            singer_mv_params("0025NhlN2yWrP4", 5, 3),
            json!({
                "singermid": "0025NhlN2yWrP4", "order": 1, "count": 5, "start": 10
            }),
            "参考把页码换算成 start = num * (page - 1)"
        );
    }

    #[test]
    fn a_singer_page_maps_every_field_the_reference_model_names() {
        let payload = singer_type_list_payload(&raw_singer_page());
        let page: SingerTypeList = serde_json::from_value(payload.clone()).expect("解析歌手列表");
        assert_eq!(page.area, Some(200));
        assert_eq!(page.sex, Some(0));
        assert_eq!(page.genre, Some(7));
        assert_eq!(page.code, Some(0));
        let singers = page.singerlist.as_ref().expect("有歌手");
        assert_eq!(singers[0].id, Some(4558), "参考的 id 别名含 singer_id");
        assert_eq!(singers[0].mid.as_deref(), Some("0025NhlN2yWrP4"));
        assert_eq!(singers[0].name.as_deref(), Some("周杰伦"));
        assert_eq!(
            singers[0].pmid.as_deref(),
            Some("0025NhlN2yWrP4_11"),
            "参考的 pmid 别名含 singer_pmid"
        );
        assert_eq!(singers[0].area_id, Some(0));
        assert_eq!(singers[0].country_id, Some(0));
        assert_eq!(singers[0].other_name.as_deref(), Some("Jay Chou"));
        assert_eq!(singers[0].spell.as_deref(), Some("zhoujielun"));
        assert_eq!(
            singers[0].concern_num,
            Some(50826458),
            "上游键是 concernNum"
        );
        assert_eq!(
            singers[0].singer_pic.as_deref(),
            Some("https://y.gtimg.cn/music/photo_new/T001R300x300M0000025NhlN2yWrP4.webp"),
            "封面过 https 归一（docs/parsing.md §7）"
        );
        let hot = page.hotlist.as_ref().expect("有热门歌手");
        assert_eq!(hot[0].mid.as_deref(), Some("001BLpXF2DyJe2"));
        assert!(hot[0].singer_pic.is_none(), "空字符串归一成 None");
        let tags = page.tags.as_ref().expect("有标签");
        assert_eq!(tags.area.as_ref().expect("有地区")[1].id, Some(200));
        assert_eq!(tags.area.as_ref().unwrap()[1].name.as_deref(), Some("内地"));
        assert_eq!(
            tags.index.as_ref().expect("有索引")[0].name.as_deref(),
            Some("A")
        );

        // 宿主按 camelCase 读。
        let round: Value = serde_json::to_value(&page).unwrap();
        assert_eq!(round["singerlist"][0]["concernNum"], 50826458);
        assert_eq!(round["singerlist"][0]["otherName"], "Jay Chou");
        assert_eq!(round["singerlist"][0]["areaId"], 0);
        assert_eq!(round["hotlist"][0]["mid"], "001BLpXF2DyJe2");
        assert_eq!(round["tags"]["area"][1]["id"], 200);
    }

    #[test]
    fn an_index_page_reports_its_index_and_total() {
        let payload = singer_index_payload(&json!({
            "area": -100,
            "sex": -100,
            "genre": -100,
            "index": 19,
            "total": 499,
            "code": 0,
            "singerlist": [{
                "singer_id": 4558,
                "singer_mid": "0025NhlN2yWrP4",
                "singer_name": "周杰伦",
                "singer_pmid": ""
            }],
            "hotlist": [],
            "tags": {"index": [{"id": 19, "name": "S"}, {"id": 27, "name": "#"}]}
        }));
        let page: SingerIndexPage = serde_json::from_value(payload).expect("解析索引分页");
        assert_eq!(page.index, Some(19));
        assert_eq!(page.total, Some(499));
        assert_eq!(page.singerlist.as_ref().map(Vec::len), Some(1));
        let tags = page.tags.as_ref().expect("有标签");
        assert_eq!(tags.index.as_ref().map(Vec::len), Some(2));
        assert!(tags.area.is_none(), "这一类没给就是 null");
    }

    #[test]
    fn a_similar_page_maps_pic_mid_and_err_msg() {
        let payload = similar_singer_list_payload(&json!({
            "singerlist": [{
                "singerId": 245,
                "singerMid": "0010PLKl2Wgolz",
                "singerName": "F.I.R.飞儿乐团",
                "singerPic": "http://y.gtimg.cn/music/photo_new/T001R150x150M0000010PLKl2Wgolz.jpg",
                "pic_mid": "0010PLKl2Wgolz_4",
                "trace": "11_0_31_2_4558_2_245_2001_0_1",
                "abt": "",
                "tf": ""
            }],
            "code": 0,
            "errMsg": ""
        }));
        let list: SimilarSingerList = serde_json::from_value(payload).expect("解析相似歌手");
        assert_eq!(list.code, Some(0));
        let singers = list.singerlist.as_ref().expect("有歌手");
        assert_eq!(singers[0].id, Some(245));
        assert_eq!(singers[0].mid.as_deref(), Some("0010PLKl2Wgolz"));
        assert_eq!(singers[0].name.as_deref(), Some("F.I.R.飞儿乐团"));
        assert_eq!(singers[0].pmid.as_deref(), Some("0010PLKl2Wgolz_4"));
        assert_eq!(
            singers[0].singer_pic.as_deref(),
            Some("https://y.gtimg.cn/music/photo_new/T001R150x150M0000010PLKl2Wgolz.jpg")
        );
        assert_eq!(
            singers[0].trace.as_deref(),
            Some("11_0_31_2_4558_2_245_2001_0_1")
        );
    }

    #[test]
    fn a_tab_page_keeps_its_songs_albums_and_videos() {
        let payload = homepage_tab_payload(&json!({
            "TabID": "song_sing",
            "HasMore": 1,
            "NeedShowTab": 0,
            "Order": 0,
            "TabList": [{"TabID": "wiki", "TabName": "简介", "Title": "简介"}],
            "IntroductionTab": {"List": [{"ItemType": 2, "SingerInfoList": [{"Title": "简介"}]}]},
            "SongTab": {"List": [{
                "id": 649556373,
                "type": 1,
                "mid": "003FdJZH1wljMU",
                "name": "西西里",
                "title": "西西里",
                "subtitle": "",
                "singer": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦"}],
                "album": {
                    "id": 87495226,
                    "mid": "0041WVfh2vtlJE",
                    "name": "太阳之子",
                    "title": "太阳之子",
                    "subtitle": "",
                    "time_public": "2026-03-25",
                    "pmid": "0041WVfh2vtlJE_1"
                },
                "interval": 229,
                "time_public": "2026-03-25",
                "pay": {"pay_play": 1}
            }]},
            "AlbumTab": {"AlbumList": [{
                "albumMid": "0041WVfh2vtlJE",
                "albumName": "太阳之子",
                "albumTranName": "",
                "publishDate": "2026-03-25",
                "totalNum": 13,
                "albumType": "",
                "pmid": "0041WVfh2vtlJE_1",
                "albumID": 87495226,
                "singerName": "周杰伦",
                "tags": ["流行"]
            }]},
            "VideoTab": {"VideoList": [{
                "mvid": 2444488,
                "vid": "002TOGAF0XYfjY",
                "title": "西西里",
                "picurl": "http://y.gtimg.cn/music/photo_new/T015R640x360M101002TOGAF0XYfjY.jpg",
                "picformat": 0,
                "duration": 233,
                "playcnt": 118675,
                "pubdate": 1789056000,
                "type": 0,
                "icon_type": 0
            }]}
        }));
        let page: HomepageTabDetail =
            serde_json::from_value(payload.clone()).expect("解析 Tab 详情");
        assert_eq!(page.tab_id.as_deref(), Some("song_sing"));
        assert_eq!(page.has_more, Some(1), "hasMore 是宿主翻页的依据");
        assert_eq!(page.need_show_tab, Some(0));
        assert_eq!(page.order, Some(0));
        assert_eq!(
            page.tab_list.as_ref().unwrap()[0].tab_id.as_deref(),
            Some("wiki")
        );
        assert_eq!(
            page.tab_list.as_ref().unwrap()[0].tab_name.as_deref(),
            Some("简介")
        );

        // 歌曲 Tab 的行用组件既有的曲目模型；歌手是 ", " 连接。
        let songs = page.song_tab.as_ref().expect("有歌曲");
        assert_eq!(songs[0].song_mid, "003FdJZH1wljMU");
        assert_eq!(songs[0].song_id, Some(649556373));
        assert_eq!(songs[0].title, "西西里");
        assert_eq!(songs[0].artist, "周杰伦");
        assert_eq!(songs[0].album_id, Some(87495226));
        assert_eq!(songs[0].album_mid.as_deref(), Some("0041WVfh2vtlJE"));
        assert_eq!(songs[0].duration, Some(229));
        assert_eq!(songs[0].pay_play, Some(1));

        let albums = page.album_tab.as_ref().expect("有专辑");
        assert_eq!(albums[0].id, Some(87495226), "上游键是 albumID（大写 ID）");
        assert_eq!(albums[0].mid.as_deref(), Some("0041WVfh2vtlJE"));
        assert_eq!(albums[0].name.as_deref(), Some("太阳之子"));
        assert_eq!(albums[0].time_public.as_deref(), Some("2026-03-25"));
        assert_eq!(albums[0].total_num, Some(13));
        assert_eq!(albums[0].singer_name.as_deref(), Some("周杰伦"));
        assert_eq!(albums[0].tags.as_deref(), Some(&["流行".to_string()][..]));

        let videos = page.video_tab.as_ref().expect("有视频");
        assert_eq!(videos[0].id, Some(2444488), "参考的 id 别名含 mvid");
        assert_eq!(videos[0].vid.as_deref(), Some("002TOGAF0XYfjY"));
        assert_eq!(videos[0].kind, Some(0));
        assert_eq!(videos[0].playcnt, Some(118675));
        assert_eq!(
            videos[0].picurl.as_deref(),
            Some("https://y.gtimg.cn/music/photo_new/T015R640x360M101002TOGAF0XYfjY.jpg")
        );

        // 简介原样透传：Rust 侧是 JSON 文本，宿主看到的还是对象数组。
        let introduction: Value =
            serde_json::from_str(page.introduction_tab.as_deref().expect("有简介")).unwrap();
        assert_eq!(introduction[0]["ItemType"], 2);
        let round: Value = serde_json::to_value(&page).unwrap();
        assert_eq!(
            round["introductionTab"][0]["SingerInfoList"][0]["Title"],
            "简介"
        );
        assert_eq!(round["songTab"][0]["songMid"], "003FdJZH1wljMU");
        assert_eq!(round["albumTab"][0]["totalNum"], 13);
        assert_eq!(round["videoTab"][0]["iconType"], 0);
    }

    #[test]
    fn a_tab_without_content_lists_is_not_a_failure() {
        // wiki 这类 Tab 没有 SongTab.List / VideoTab.VideoList：上游直接给 null，
        // 那是「这个 Tab 没有这类内容」，照常解析。字段保持 Option：宿主可以区分
        // 「上游没给这个列表」与「给了个空列表」。
        let page: HomepageTabDetail = serde_json::from_value(homepage_tab_payload(&json!({
            "TabID": "wiki",
            "HasMore": 0,
            "NeedShowTab": 0,
            "Order": 0,
            "TabList": null,
            "IntroductionTab": {"List": [{"ItemType": 2}]},
            "SongTab": {"List": null},
            "AlbumTab": {"AlbumList": null},
            "VideoTab": {"VideoList": null}
        })))
        .expect("解析空 Tab");
        assert_eq!(page.tab_id.as_deref(), Some("wiki"));
        assert_eq!(page.has_more, Some(0));
        assert!(page.song_tab.is_none());
        assert!(page.album_tab.is_none());
        assert!(page.video_tab.is_none());
        assert!(page.tab_list.is_none(), "TabList 为 null 时保持 null");
        assert!(page.introduction_tab.is_some());

        // 给了空列表就是空列表：那是这个 Tab 的一页，只是这一页没有内容。
        let page: HomepageTabDetail = serde_json::from_value(homepage_tab_payload(&json!({
            "TabID": "song_sing",
            "HasMore": 0,
            "SongTab": {"List": []},
            "AlbumTab": {"AlbumList": []},
            "VideoTab": {"VideoList": []},
            "TabList": []
        })))
        .expect("解析空列表 Tab");
        assert!(page.song_tab.as_ref().expect("有歌曲字段").is_empty());
        assert!(page.album_tab.as_ref().expect("有专辑字段").is_empty());
        assert!(page.video_tab.as_ref().expect("有视频字段").is_empty());
        assert!(page.tab_list.as_ref().expect("有标签列表").is_empty());
    }

    #[test]
    fn the_name_display_comes_out_of_the_nested_singer_block() {
        let payload = special_display_payload(&json!({
            "Info": {
                "Singer": {
                    "SingerMid": "000qrPik2w6lDr",
                    "Name": "Taylor Swift",
                    "SingerNameSpecialDisplay": {
                        "DisplayType": 2,
                        "PicFile": "https://music-conf-cdn.y.qq.com/ocs/pp/156304/x/38f3.png",
                        "SignatureNameOverlapRatio": 0,
                        "Name": "Taylor Swift"
                    }
                }
            }
        }));
        let display: SingerNameSpecialDisplay =
            serde_json::from_value(payload.clone()).expect("解析名称图片");
        assert_eq!(display.display_type, Some(2));
        assert_eq!(
            display.pic_file.as_deref(),
            Some("https://music-conf-cdn.y.qq.com/ocs/pp/156304/x/38f3.png")
        );
        assert_eq!(display.signature_name_overlap_ratio, Some(0.0));
        assert_eq!(display.name.as_deref(), Some("Taylor Swift"));

        // 没有名称图片的歌手：DisplayType 0、PicFile 空——是答案，不是故障。
        let none: SingerNameSpecialDisplay =
            serde_json::from_value(special_display_payload(&json!({
                "Info": {"Singer": {"Name": "林俊杰",
                    "SingerNameSpecialDisplay": {
                        "DisplayType": 0, "PicFile": "",
                        "SignatureNameOverlapRatio": 0, "Name": "林俊杰"
                    }}}
            })))
            .expect("解析无图片的展示");
        assert_eq!(none.display_type, Some(0));
        assert!(none.pic_file.is_none(), "空字符串落成 None");
        assert_eq!(none.name.as_deref(), Some("林俊杰"));
    }

    #[test]
    fn an_empty_singer_shell_is_a_failure_not_a_display_type_zero() {
        // 实测 web 档案下的回值：`Info.Singer` 每个字段都空。它是"请联系档案/
        // 参数"，不能伪装成「这位歌手没有名称图片」。
        assert!(empty_singer_shell(&json!({
            "Info": {"Singer": {"Name": "", "SingerMid": "", "SingerNameSpecialDisplay": {}}}
        })));
        assert!(empty_singer_shell(&json!({"Info": {}})));
        assert!(!empty_singer_shell(&json!({
            "Info": {"Singer": {"Name": "林俊杰",
                "SingerNameSpecialDisplay": {"DisplayType": 0, "PicFile": ""}}}
        })));
    }

    #[test]
    fn the_display_comm_overrides_cv_and_v_like_the_reference() {
        let anonymous = display_comm(&Credential::default());
        assert_eq!(anonymous["ct"], 11);
        assert_eq!(anonymous["cv"], 20_080_000);
        assert_eq!(anonymous["v"], 20_080_000);
        assert_eq!(anonymous["platform"], "yqq.json");
        assert_eq!(anonymous["tmeAppID"], "qqmusic");
        assert_eq!(anonymous["tmeLoginType"], 1, "没有凭据时按 QQ 的登录类型");
        assert!(anonymous.get("qq").is_none(), "没登录就不带账号身份");

        let account = display_comm(&Credential {
            music_id: "225".into(),
            music_key: "W_X_key".into(),
            raw: json!({"loginType": 2}),
            ..Default::default()
        });
        assert_eq!(account["qq"], "225");
        assert_eq!(account["authst"], "W_X_key");
        assert_eq!(account["tmeLoginType"], 2, "登录类型照凭据走");
        assert_eq!(account["cv"], 20_080_000);
    }

    #[test]
    fn a_mv_page_reports_its_total_and_videos() {
        let payload = singer_mv_payload(&json!({
            "total": 10424,
            "list": [{
                "duration": 317,
                "icon_type": 0,
                "mvid": 293791,
                "picformat": 0,
                "picurl": "http://y.gtimg.cn/music/photo_new/T015R640x360M10300061J2t0b0PPW.jpg",
                "playcnt": 120407054,
                "pubdate": 1059408000,
                "title": "晴天",
                "type": 0,
                "vid": "w0026q7f01a"
            }]
        }));
        let page: SingerMvList = serde_json::from_value(payload).expect("解析 MV 列表");
        assert_eq!(page.total, Some(10424));
        assert_eq!(page.mv_list.as_ref().expect("有 MV")[0].id, Some(293791));
        assert_eq!(
            page.mv_list.as_ref().unwrap()[0].title.as_deref(),
            Some("晴天")
        );
    }

    #[test]
    fn an_empty_page_is_an_answer_not_a_failure() {
        // 六个接口都不是 require_login 的整表读取：空是合法答案，照常解析。
        let page: SingerTypeList =
            serde_json::from_value(singer_type_list_payload(&json!({}))).expect("空列表照常解析");
        assert!(page.singerlist.is_none());
        assert!(page.tags.is_some(), "tags 的四个槽位保持 null");

        let list: SimilarSingerList =
            serde_json::from_value(similar_singer_list_payload(&json!({"singerlist": []})))
                .expect("空相似歌手照常解析");
        assert!(list.singerlist.expect("有 singerlist 字段").is_empty());

        let page: SingerMvList =
            serde_json::from_value(singer_mv_payload(&json!({"list": [], "total": 0})))
                .expect("空 MV 列表照常解析");
        assert_eq!(page.total, Some(0));
        assert!(page.mv_list.expect("有 mvList 字段").is_empty());
    }

    #[test]
    fn unknown_enum_values_are_refused_like_the_reference() {
        // 参考的 AreaType(7) 之类会直接抛异常；上游则会回"成功但空"。
        assert!(enum_value(&json!({"area": 7}), &["area"], AREA_ALL, AREAS, "地区").is_err());
        assert!(enum_value(&json!({"sex": 9}), &["sex"], SEX_ALL, SEXES, "性别").is_err());
        assert!(enum_value(&json!({"genre": 1}), &["genre"], GENRE_ALL, GENRES, "风格").is_err());
        assert!(enum_value(
            &json!({"index": 30}),
            &["index"],
            INDEX_ALL,
            INDEXES,
            "首字母索引"
        )
        .is_err());
        assert_eq!(
            enum_value(&json!({}), &["area"], AREA_ALL, AREAS, "地区").unwrap(),
            AREA_ALL
        );
        assert_eq!(
            enum_value(
                &json!({"index": 27}),
                &["index"],
                INDEX_ALL,
                INDEXES,
                "首字母索引"
            )
            .unwrap(),
            INDEX_HASH
        );
    }

    #[test]
    fn the_tab_id_accepts_the_reference_strings_and_order_numbers() {
        assert_eq!(
            require_tab_id(&json!({"tabType": "song_sing"})).unwrap(),
            "song_sing"
        );
        // 数字按参考 TabType 的声明次序收，协议上仍然发字符串。
        assert_eq!(require_tab_id(&json!({"tabType": 1})).unwrap(), "wiki");
        assert_eq!(require_tab_id(&json!({"tabType": 8})).unwrap(), "song_sing");
        assert_eq!(require_tab_id(&json!({"tabType": 9})).unwrap(), "video");
        let error = require_tab_id(&json!({"tabType": 10})).expect_err("序号越界要报错");
        assert!(error.to_string().contains("wiki"), "报错要列出可用值");
        assert!(require_tab_id(&json!({"tabType": "song_singing"})).is_err());
        assert!(require_tab_id(&json!({})).is_err());
    }

    #[test]
    fn page_and_size_accept_the_protocol_spellings() {
        assert_eq!(size_of(&json!({"limit": 20}), INDEX_PAGE_SIZE), 20);
        assert_eq!(size_of(&json!({"num": 5}), INDEX_PAGE_SIZE), 5);
        assert_eq!(page_of(&json!({"pageNum": 4}), 1), 4);
        assert_eq!(page_of(&json!({}), 1), 1);
        // 参考的缺省：index 每页 80、相似歌手 10 位、Tab 每页 10、MV 每页 10。
        assert_eq!(size_of(&json!({}), INDEX_PAGE_SIZE), 80);
        assert_eq!(size_of(&json!({}), TAB_PAGE_SIZE), 10);
        assert_eq!(size_of(&json!({}), MV_PAGE_SIZE), 10);
        assert_eq!(SIMILAR_NUMBER, 10);
    }

    #[test]
    fn the_tab_defaults_to_android_unless_the_caller_chose() {
        // 参考没给这个端点标档案，但参考的默认档案是 ANDROID；实测 web 下
        // 端点只回空壳，所以调用方没显式选择时用 android。
        assert_eq!(tab_platform(&json!({}), Platform::Web), Platform::Android);
        assert_eq!(
            tab_platform(&json!({"platform": "web"}), Platform::Web),
            Platform::Web
        );
    }

    /// 请求本身不成立时，在发出任何请求之前就应当被拒。
    #[test]
    fn malformed_requests_are_refused_before_any_request() {
        let upstream = Upstream::new();
        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_similar_artists",
            &json!({}),
        )
        .expect("fetch_similar_artists 是本层的方法")
        .expect_err("没有 mid 要报错");
        assert!(error.to_string().contains("MID"));

        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_artist_tab",
            &json!({"singerMid": "0025NhlN2yWrP4", "tabType": "song_singing"}),
        )
        .expect("fetch_artist_tab 是本层的方法")
        .expect_err("不认识的 Tab 类型要报错");
        assert!(error.to_string().contains("Tab"));

        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_singer_list",
            &json!({"area": 7}),
        )
        .expect("fetch_singer_list 是本层的方法")
        .expect_err("不在 AreaType 里的地区要报错");
        assert!(error.to_string().contains("AreaType"));

        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_artist_display_name",
            &json!({}),
        )
        .expect("fetch_artist_display_name 是本层的方法")
        .expect_err("没有 mid 要报错");
        assert!(error.to_string().contains("MID"));
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 6);
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_liked_songs",
            &json!({})
        )
        .is_none());
    }
}

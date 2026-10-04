//! 账号资产：收藏的歌单 / 专辑 / MV、音乐基因、不喜欢列表（含签名路）。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/user.py` 的
//! `get_fav_songlist` / `get_fav_album` / `get_fav_mv` / `get_music_gene` /
//! `get_dislike_list` / `add_dislike` / `cancel_dislike` /
//! `cancel_all_dislike_song`；回值字段名照 `models/user.py` 的同名模型
//! （camelCase），模型层级也一样。（同一模块里的账号关系、VIP、主页等接口
//! 由其他领域文件承担，这里不重复移植。）
//!
//! # 平台档案
//!
//! 四个读取端点参考都没标 `platform`，按本层约定默认 Web；`dispatch` 收到调用方
//! 传入的 platform 时原样尊重。不喜欢四个端点只看模块与参数，档案同样照传。
//!
//! # 签名路
//!
//! 不喜欢家族（[`fetch_dislike_list`] / [`add_dislike`] / [`cancel_dislike`] /
//! [`clear_dislike_songs`]）走 `musics.fcg` 签名路（[`Upstream::call_signed`]，
//! comm 用账号默认那套、不覆盖）。参考里只有 `get_dislike_list` 标了
//! `sign=True`，三个写接口没标——工作单要求整个家族都签名。签名对它们只是同一份
//! 信封多带一个 `zzc`，module / method / 参数一个字不改；写接口若在真机上拒签名，
//! 回退到 `musicu.fcg` 即可（见 openQuestions）。
//!
//! # 账号标识（euin）
//!
//! 参考按 euin（加密 uin）寻址。调用方给了就用；给了空串按没给，退回
//! [`Upstream::encrypted_uin`]——凭据里有 `encrypt_uin` 时它直接给出来，
//! 没有时会去 `GetLoginUserInfo` 找一次。找不到就是**凭据问题**，报错而不是拿
//! 空串去发请求：空串会被上游当成"查无此人"，回一份空列表，看起来像"这个账号
//! 没有收藏"。
//!
//! # 空值判据
//!
//! 「收藏歌单 / 收藏专辑 / 收藏 MV」是账号的整表读取，判据是**列表键在不在**：
//! 参考模型把这三个列表声明成必填（`playlists` 的 jsonpath 是 `$.v_list`、
//! `albums` 是 `$.v_list[*]`、`mv_list` 是上游的 `mvlist`），键不在时那套模型
//! 直接校验失败，所以这里也报错——形状不对的空壳照空列表返回，会让宿主拿它去
//! 覆盖缓存里的真数据（docs/parsing.md §13）。键在而列表为空是**合法答案**
//! （这个账号就是没有收藏），照常返回。
//!
//! 不喜欢列表相反：参考给 `singers`/`songs`/`styles` 都带 `default_factory=list`，
//! 「没有不喜欢」就是正常状态，空列表照常返回。音乐基因以 `UserInfoCard` 为形状
//! 锚点（参考里它是必填）：卡片不在说明账号标识或档案不对，报错；卡片在而
//! `ListeningReport` 缺了就是这个账号还没有听歌报告，不给额外错误。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::models::Singer;
use crate::upstream::{
    first_array, first_int, first_object, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "fetch_fav_playlists",
    "fetch_fav_albums",
    "fetch_fav_mvs",
    "fetch_music_gene",
    "fetch_dislike_list",
    "add_dislike",
    "cancel_dislike",
    "clear_dislike_songs",
];

/// 不喜欢模块的 module 名（四个端点共用）。
const FEEDBACK_MODULE: &str = "music.feedback.FeedbackBlack";
/// 参考 `get_dislike_list` 的缺省 `cmd`：3=歌曲。
const DISLIKE_CMD_SONG: i64 = 3;
/// `cancel_all_dislike_song` 第二步的 `DelType`：3=歌曲（与 `cmd` 同义）。
const DISLIKE_DEL_TYPE_SONG: i64 = 3;
/// 参考的缺省页码。
const DEFAULT_PAGE: i64 = 1;
/// 参考的缺省每页数量。
const DEFAULT_PAGE_SIZE: i64 = 10;

/// 收藏歌单 / 收藏专辑的两个列表键（两个端点都用 `v_list`）。
const V_LIST_KEYS: &[&str] = &["v_list", "vList"];
/// 收藏 MV 的列表键。
const MV_LIST_KEYS: &[&str] = &["mvlist", "mvList", "MvList"];

// MARK: - 模型

/// 参考 `UserFavSonglistItem`（继承 `SongList`）：收藏歌单列表里的单个条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFavSonglistItem {
    /// 歌单 ID（参考 `SongList.id`，别名 `tid`/`dissid`）.
    pub id: Option<i64>,
    /// 目录 ID.
    pub dirid: Option<i64>,
    /// 歌单标题.
    pub title: Option<String>,
    /// 歌单封面地址.
    pub picurl: Option<String>,
    /// 歌单简介.
    pub desc: Option<String>,
    /// 歌曲数量.
    pub songnum: Option<i64>,
    /// 播放量.
    pub listennum: Option<i64>,
    /// 歌单所属用户 UIN.
    pub uin: Option<String>,
    /// 歌单拥有者昵称.
    pub nickname: Option<String>,
    /// 创建时间戳（上游键 `createtime`）.
    pub create_time: Option<i64>,
    /// 更新时间戳.
    pub update_time: Option<i64>,
    /// 收藏排序时间戳.
    pub order_time: Option<i64>,
    /// 目录展示标记.
    pub dir_show: Option<i64>,
    /// 目录类型.
    pub dir_type: Option<i64>,
    /// 边角标识.
    pub edge_mark: Option<String>,
    /// 分层装饰地址.
    pub layer_url: Option<String>,
    /// 专辑拼接封面地址.
    pub album_pic_url: Option<String>,
    /// 操作类型标记.
    pub op_type: Option<i64>,
    /// 排序权重.
    pub sort_weight: Option<i64>,
    /// 最近读取时间戳.
    pub readtime: Option<i64>,
}

/// 参考 `UserFavSonglistResponse`：用户收藏的外部歌单列表页响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFavSonglistResponse {
    /// 当前页数量或请求数量.
    pub number: Option<i64>,
    /// 收藏歌单总数.
    pub total: Option<i64>,
    /// 是否还有更多结果.
    pub hasmore: Option<i64>,
    /// 列表是否隐藏.
    pub hide: Option<bool>,
    /// 当前页收藏歌单列表（上游键 `v_list`）.
    pub playlists: Option<Vec<UserFavSonglistItem>>,
    /// 上游返回的删除歌单 ID 列表（上游键 `v_delTids`）.
    pub deleted_ids: Option<Vec<i64>>,
    /// 拉取失败的歌单 ID 列表（上游键 `v_failTids`）.
    pub failed_ids: Option<Vec<i64>>,
}

/// 参考 `UserFavAlbumItem`（继承 `Album`）：收藏专辑列表里的单个条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFavAlbumItem {
    /// 专辑数字 ID（上游键 `albumID`，大写 ID 是上游的拼写）.
    pub id: Option<i64>,
    /// 专辑 MID.
    pub mid: Option<String>,
    /// 专辑名称.
    pub name: Option<String>,
    /// 专辑展示标题.
    pub title: Option<String>,
    /// 专辑副标题.
    pub subtitle: Option<String>,
    /// 发行日期（参考 `Album.time_public`）.
    pub time_public: Option<String>,
    /// 图片 Media ID，用于拼接封面 URL.
    pub pmid: Option<String>,
    /// 专辑曲目数.
    pub songnum: Option<i64>,
    /// 发布时间戳.
    pub pubtime: Option<i64>,
    /// 收藏排序时间戳.
    pub ordertime: Option<i64>,
    /// 状态标记.
    pub status: Option<i64>,
    /// 位置或来源标记.
    pub loc: Option<i64>,
    /// 专辑歌手列表（上游键 `v_singer`，复用组件既有的歌手模型）.
    pub singers: Option<Vec<Singer>>,
}

/// 参考 `UserFavAlbumResponse`：用户收藏的专辑列表页响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFavAlbumResponse {
    /// 当前页数量或请求数量.
    pub number: Option<i64>,
    /// 收藏专辑总数.
    pub total: Option<i64>,
    /// 是否还有更多结果.
    pub hasmore: Option<i64>,
    /// 列表是否隐藏.
    pub hide: Option<bool>,
    /// 当前页收藏专辑列表（上游键 `v_list`）.
    pub albums: Option<Vec<UserFavAlbumItem>>,
    /// 拉取失败的专辑 ID 列表（上游键 `v_failAlbumId`）.
    pub failed_album_ids: Option<Vec<i64>>,
}

/// 参考 `UserFavMvItem`（继承 `MV`）：收藏 MV 列表里的单个条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFavMvItem {
    /// MV 数字 ID（参考 `MV.id`，别名 `sid`/`mvid`/`singerId`）.
    pub id: Option<i64>,
    /// MV VID.
    pub vid: Option<String>,
    /// MV 类型（参考 `MV.type`，上游键 `vt`；Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// MV 名称.
    pub name: Option<String>,
    /// MV 展示标题.
    pub title: Option<String>,
    /// MV 封面地址（上游键 `picUrl`）.
    pub picurl: Option<String>,
    /// 播放量.
    pub playcount: Option<i64>,
    /// 发布时间（参考字段名 `publish_date`）.
    pub publish_date: Option<i64>,
    /// 歌手 ID（上游键 `singerId`）.
    pub singer_id: Option<i64>,
    /// 歌手 MID.
    pub singer_mid: Option<String>,
    /// 歌手名称.
    pub singer_name: Option<String>,
    /// 状态标记.
    pub status: Option<i64>,
}

/// 参考 `UserFavMvResponse`：用户收藏 MV 列表视图响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFavMvResponse {
    /// 返回码.
    pub code: Option<i64>,
    /// 子返回码（参考同时接受 `subCode` 与 `subcode` 两种拼写）.
    pub sub_code: Option<i64>,
    /// 附加消息.
    pub msg: Option<String>,
    /// 当前页收藏 MV 列表（上游键 `mvlist`）.
    pub mv_list: Option<Vec<UserFavMvItem>>,
}

/// 参考 `UserInfoCard`：用户音乐基因页头部卡片信息.
///
/// `preferences` 在参考里是 `dict[str, Any]`（原样透传的上游形状），这里存
/// JSON 文本：`#[data]` 认不出 `serde_json::Value`，而经 serde 进出时它仍是
/// 真正的对象（见 `raw_json_serialize`）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserInfoCard {
    /// 头像地址（上游键 `HeadUrl`）.
    pub head_url: Option<String>,
    /// 昵称（上游键 `NickName`）.
    pub nick_name: Option<String>,
    /// 个性签名（上游键 `Signature`）.
    pub signature: Option<String>,
    /// 加密账号标识（上游键 `EncryptionAccount`）.
    pub encryption_account: Option<String>,
    /// 偏好信息块（上游键 `Preferences`，dict 原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub preferences: Option<String>,
}

/// 参考 `ListeningReport`：用户听歌报告摘要.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListeningReport {
    /// 听歌报告分块列表（上游键 `Report`，dict 列表原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub report: Option<String>,
}

/// 参考 `UserMusicGeneResponse`：用户音乐基因视图响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserMusicGeneResponse {
    /// 用户卡片信息（上游键 `UserInfoCard`）.
    pub user_info_card: Option<UserInfoCard>,
    /// 听歌报告摘要（上游键 `ListeningReport`）.
    pub listening_report: Option<ListeningReport>,
    /// 排序提示数组（上游键 `SortArray`）.
    pub sort_array: Option<Vec<i64>>,
    /// 是否访问本人账号（上游键 `IsVisitAccount`）.
    pub is_visit_account: Option<bool>,
}

/// 参考 `DislikeItem`：不喜欢列表中的单个条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DislikeItem {
    /// 不喜欢实体的 ID（歌手 ID、歌曲 ID 等；参考声明为字符串）.
    pub id: Option<String>,
    /// 不喜欢实体的名称.
    pub name: Option<String>,
    /// 不喜欢实体的图片 / 封面 URL.
    pub img: Option<String>,
    /// 实体类型标识.
    pub id_type: Option<i64>,
    /// 添加到不喜欢列表的时间戳.
    pub time: Option<i64>,
}

/// 参考 `DislikeListData`：`GetDislikeList` 响应数据.
///
/// 三个列表在参考里都带 `default_factory=list`——空列表是答案（没有不喜欢），
/// 所以 payload 会把它们补齐成数组。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DislikeListData {
    /// 业务返回码（参考的 `Retcode`）.
    pub retcode: Option<i64>,
    /// 业务返回信息.
    pub msg: Option<String>,
    /// 不喜欢的歌手列表.
    pub singers: Option<Vec<DislikeItem>>,
    /// 不喜欢的歌曲列表.
    pub songs: Option<Vec<DislikeItem>>,
    /// 不喜欢的风格 / 流派列表.
    pub styles: Option<Vec<DislikeItem>>,
    /// 当前页码.
    pub page: Option<i64>,
    /// 翻页 Token.
    pub token: Option<String>,
}

// MARK: - 参考模型里 `dict` 形状的字段

/// 把 Rust 侧存的 JSON 文本按真 JSON 写出（宿主在 JSON 协议上看到的是对象/数组）。
///
/// 与 `mv.rs` / `comment.rs` 的同名助手同源：每个领域文件自包含，这份是刻意复制的。
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

/// 取第一个出现的键，原样保留（不透传形状的字段用）。
fn raw_field(raw: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .filter(|found| !found.is_null())
        .cloned()
}

// MARK: - 回值搬运（上游形状 → 参考模型形状）

/// 封面类 URL 一律转 https（docs/parsing.md §7）——宿主拒绝 http 图片，
/// 表现为封面静默空白。字段名与取值都不变，只把协议换掉。
fn artwork(value: Option<String>) -> Option<String> {
    crate::methods::normalized_artwork_url(value.as_deref())
}

/// 布尔字段：上游给 true/false，也给 1/0 与 "true"/"1"（同 comment.rs）。
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

/// 数字或数字字符串 → 整数。
fn coerce_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 一组整数（上游给数字也给数字字符串）。
fn int_list(raw: &Value, keys: &[&str]) -> Option<Vec<i64>> {
    first_array(raw, keys).map(|items| items.iter().filter_map(coerce_int).collect())
}

/// 列表里的单个歌手（参考基础模型 `Singer`，复用组件的 `crate::models::Singer`）。
fn singer_payload(raw: &Value) -> Value {
    json!({
        "mid": first_text(raw, &["mid", "singerMid", "singerMID", "SingerMid", "singer_mid"]),
        "name": first_text(raw, &["name", "singerName", "singer_name"]),
    })
}

/// 账号列表的形状检查：**列表键在不在**（判据见文件头「空值判据」）。
fn require_list(data: &Value, keys: &[&str], what: &str) -> Result<(), UpstreamError> {
    if first_array(data, keys).is_some() {
        Ok(())
    } else {
        Err(UpstreamError::Upstream(format!(
            "上游没有返回{what}（响应形状不对：可能账号标识不对或登录已过期）"
        )))
    }
}

/// 收藏歌单列表里的单个条目（参考 `UserFavSonglistItem`）。
fn songlist_item_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "tid", "dissid", "dissId"]),
        "dirid": first_int(raw, &["dirid", "dirId"]),
        "title": first_text(raw, &["title", "dissname", "name", "dirName"]),
        "picurl": artwork(first_text(raw, &["picurl", "cover", "logo", "picUrl"])),
        "desc": first_text(raw, &["desc", "description"]),
        "songnum": first_int(raw, &["songnum", "songNum", "song_cnt"]),
        "listennum": first_int(raw, &["listennum", "playCnt", "play_cnt"]),
        "uin": first_text(raw, &["uin"]),
        "nickname": first_text(raw, &["nickname", "nickName", "nick"]),
        // 参考的 `create_time` 别名是上游的 `createtime`（一个词）。
        "createTime": first_int(raw, &["createtime", "createTime"]),
        "updateTime": first_int(raw, &["updateTime", "updatetime"]),
        "orderTime": first_int(raw, &["orderTime", "ordertime"]),
        "dirShow": first_int(raw, &["dirShow", "dirshow"]),
        "dirType": first_int(raw, &["dirType", "dirtype"]),
        "edgeMark": first_text(raw, &["edgeMark", "edgemark"]),
        "layerUrl": artwork(first_text(raw, &["layerUrl", "layerurl"])),
        "albumPicUrl": artwork(first_text(raw, &["albumPicUrl", "albumpicUrl"])),
        "opType": first_int(raw, &["opType", "optype"]),
        "sortWeight": first_int(raw, &["sortWeight", "sortweight"]),
        "readtime": first_int(raw, &["readtime", "readTime"]),
    })
}

/// 收藏歌单回值（参考 `UserFavSonglistResponse`）。
fn fav_songlist_payload(data: &Value) -> Result<Value, UpstreamError> {
    require_list(data, V_LIST_KEYS, "收藏歌单列表（v_list）")?;
    Ok(json!({
        "number": first_int(data, &["number", "Number"]),
        "total": first_int(data, &["total", "Total"]),
        "hasmore": first_int(data, &["hasmore", "hasMore", "HasMore"]),
        "hide": bool_field(data, &["hide", "Hide"]),
        "playlists": first_array(data, V_LIST_KEYS)
            .map(|items| items.iter().map(songlist_item_payload).collect::<Vec<_>>()),
        "deletedIds": int_list(data, &["v_delTids", "v_delTid", "deletedIds"]),
        "failedIds": int_list(data, &["v_failTids", "v_failTid", "failedIds"]),
    }))
}

/// 收藏专辑列表里的单个条目（参考 `UserFavAlbumItem`）。
fn fav_album_item_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "albumID", "albumId"]),
        "mid": first_text(raw, &["mid", "albumMid", "albumMID", "albummid"]),
        "name": first_text(raw, &["name", "albumName"]),
        "title": first_text(raw, &["title", "albumName", "name"]),
        "subtitle": first_text(raw, &["subtitle", "albumTranName"]),
        "timePublic": first_text(raw, &["time_public", "publish_date", "publishDate"]),
        "pmid": first_text(raw, &["pmid", "logo"]),
        "songnum": first_int(raw, &["songnum", "songNum"]),
        "pubtime": first_int(raw, &["pubtime"]),
        "ordertime": first_int(raw, &["ordertime", "orderTime"]),
        "status": first_int(raw, &["status"]),
        "loc": first_int(raw, &["loc"]),
        "singers": first_array(raw, &["v_singer", "singers", "singer"])
            .map(|items| items.iter().map(singer_payload).collect::<Vec<_>>()),
    })
}

/// 收藏专辑回值（参考 `UserFavAlbumResponse`）。
fn fav_album_payload(data: &Value) -> Result<Value, UpstreamError> {
    require_list(data, V_LIST_KEYS, "收藏专辑列表（v_list）")?;
    Ok(json!({
        "number": first_int(data, &["number", "Number"]),
        "total": first_int(data, &["total", "Total"]),
        "hasmore": first_int(data, &["hasmore", "hasMore", "HasMore"]),
        "hide": bool_field(data, &["hide", "Hide"]),
        "albums": first_array(data, V_LIST_KEYS)
            .map(|items| items.iter().map(fav_album_item_payload).collect::<Vec<_>>()),
        "failedAlbumIds": int_list(data, &["v_failAlbumId", "v_failAlbumID", "failedAlbumIds"]),
    }))
}

/// 收藏 MV 列表里的单个条目（参考 `UserFavMvItem`）。
///
/// 注意 `id` 的别名里有 `singerId`（参考 `MV` 就是这么写的）：上游同时给了
/// `mvid` 与 `singerId` 时 `id` 取 `mvid`，只给 `singerId` 时两个字段都拿它。
fn fav_mv_item_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "sid", "mvid", "singerId"]),
        "vid": first_text(raw, &["vid"]),
        "type": first_int(raw, &["vt", "type"]),
        "name": first_text(raw, &["name", "mvname", "title"]),
        "title": first_text(raw, &["title", "title_main", "name"]),
        "picurl": artwork(first_text(raw, &["picUrl", "picurl"])),
        "playcount": first_int(raw, &["playcount", "playCount"]),
        "publishDate": first_int(raw, &["publishDate", "publish_date", "pubdate"]),
        "singerId": first_int(raw, &["singerId", "singer_id"]),
        "singerMid": first_text(raw, &["singerMid", "singerMID", "singer_mid"]),
        "singerName": first_text(raw, &["singerName", "singer_name"]),
        "status": first_int(raw, &["status"]),
    })
}

/// 收藏 MV 回值（参考 `UserFavMvResponse`）。
fn fav_mv_payload(data: &Value) -> Result<Value, UpstreamError> {
    require_list(data, MV_LIST_KEYS, "收藏 MV 列表（mvlist）")?;
    Ok(json!({
        "code": first_int(data, &["code", "Code"]),
        // 参考里 `sub_code` 同时接受 `subCode` 与 `subcode` 两种拼写。
        "subCode": first_int(data, &["subCode", "subcode", "SubCode", "sub_code"]),
        "msg": first_text(data, &["msg", "Msg"]),
        "mvList": first_array(data, MV_LIST_KEYS)
            .map(|items| items.iter().map(fav_mv_item_payload).collect::<Vec<_>>()),
    }))
}

/// 音乐基因页头部卡片（参考 `UserInfoCard`）。
fn user_info_card_payload(raw: &Value) -> Value {
    json!({
        "headUrl": artwork(first_text(raw, &["HeadUrl", "headUrl", "head_url"])),
        "nickName": first_text(raw, &["NickName", "nickName", "nick_name"]),
        "signature": first_text(raw, &["Signature", "signature"]),
        "encryptionAccount": first_text(
            raw,
            &["EncryptionAccount", "encryptionAccount", "encryption_account"],
        ),
        "preferences": raw_field(raw, &["Preferences", "preferences"]),
    })
}

/// 音乐基因回值（参考 `UserMusicGeneResponse`）。
///
/// 形状锚点是 `UserInfoCard`：参考里它是必填，卡片不在说明账号标识或档案不对，
/// 报错比伪装成「这个账号没有音乐基因」好查。听歌报告（`ListeningReport`）缺了
/// 就是这个账号还没有报告——那是答案，不给额外错误。
fn music_gene_payload(data: &Value) -> Result<Value, UpstreamError> {
    let card = first_object(data, &["UserInfoCard", "userInfoCard"]).ok_or_else(|| {
        UpstreamError::Upstream(
            "上游没有返回音乐基因卡片（UserInfoCard）：账号标识或档案可能不对".into(),
        )
    })?;
    let report = first_object(data, &["ListeningReport", "listeningReport"])
        .map(|report| json!({ "report": raw_field(report, &["Report", "report"]) }));
    Ok(json!({
        "userInfoCard": user_info_card_payload(card),
        "listeningReport": report.unwrap_or_else(|| json!({ "report": Value::Null })),
        "sortArray": first_array(data, &["SortArray", "sortArray"])
            .map(|items| items.iter().filter_map(coerce_int).collect::<Vec<_>>()),
        "isVisitAccount": bool_field(data, &["IsVisitAccount", "isVisitAccount"]),
    }))
}

/// 不喜欢列表里的单个条目（参考 `DislikeItem`，上游键首字母大写）。
fn dislike_item_payload(raw: &Value) -> Value {
    json!({
        "id": first_text(raw, &["ID", "id"]),
        "name": first_text(raw, &["Name", "name"]),
        "img": artwork(first_text(raw, &["Img", "img"])),
        "idType": first_int(raw, &["IdType", "idType"]),
        "time": first_int(raw, &["Time", "time"]),
    })
}

/// 不喜欢回值（参考 `DislikeListData`）。
///
/// 三个列表按参考的 `default_factory=list` 补齐成数组：「没有不喜欢」是答案。
fn dislike_list_payload(data: &Value) -> Value {
    let items = |keys: &[&str]| {
        first_array(data, keys)
            .map(|items| items.iter().map(dislike_item_payload).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    json!({
        "retcode": first_int(data, &["Retcode", "retcode"]),
        "msg": first_text(data, &["Msg", "msg"]).unwrap_or_default(),
        "singers": items(&["Singers", "singers"]),
        "songs": items(&["Songs", "songs"]),
        "styles": items(&["Styles", "styles"]),
        "page": first_int(data, &["Page", "page"]),
        "token": first_text(data, &["Token", "token"]).unwrap_or_default(),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 参考 `require_login=True` 的门槛：凭据可用（`musicid` + `musickey`）。
fn require_login(credential: &Credential) -> Result<(), UpstreamError> {
    if credential.is_usable() {
        Ok(())
    } else {
        Err(UpstreamError::Upstream("需要登录后才能读取".into()))
    }
}

/// 账号标识：调用方给了就用（空串按没给），没给退回 [`Upstream::encrypted_uin`]。
///
/// 参考的这四个读取接口按 euin（加密 uin）寻址；本地凭据里有 `encrypt_uin` 时
/// [`Upstream::encrypted_uin`] 直接给出来，没有时会去 `GetLoginUserInfo` 找一次。
/// 找不到就是凭据问题，报错（空串会被上游当"查无此人"）。
fn resolved_euin(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<String, UpstreamError> {
    if let Some(euin) =
        first_text(params, &["euin", "encUin", "enc_uin"]).filter(|value| !value.trim().is_empty())
    {
        return Ok(euin);
    }
    upstream
        .encrypted_uin(credential)
        .map_err(|error| match error {
            UpstreamError::Upstream(detail) => UpstreamError::Upstream(format!(
                "这个凭据没有可用的账号标识（encrypt_uin）：{detail}"
            )),
            other => other,
        })
}

/// 页码：参考的缺省是 1，给什么发什么（参考不做区间钳制）。
fn page_of(params: &Value) -> i64 {
    first_int(params, &["page"]).unwrap_or(DEFAULT_PAGE)
}

/// 每页数量：参考叫 `num`，驱动组件的调用方也常用 `limit`/`size`。
fn size_of(params: &Value) -> i64 {
    first_int(params, &["num", "limit", "size"]).unwrap_or(DEFAULT_PAGE_SIZE)
}

/// 收藏歌单参数（参考 `get_fav_songlist`）：`uin` + 偏移量，不是页码。
fn fav_songlist_params(euin: &str, page: i64, num: i64) -> Value {
    json!({
        "uin": euin,
        "offset": (page - 1) * num,
        "size": num,
    })
}

/// 收藏专辑参数（参考 `get_fav_album`）：键名是 `euin`（与歌单那个 `uin` 不同）。
fn fav_album_params(euin: &str, page: i64, num: i64) -> Value {
    json!({
        "euin": euin,
        "offset": (page - 1) * num,
        "size": num,
    })
}

/// 收藏 MV 参数（参考 `get_fav_mv`）：`num` 那一项是**页码减一**，照抄不要"修正"。
fn fav_mv_params(euin: &str, page: i64, num: i64) -> Value {
    json!({
        "encuin": euin,
        "pagesize": num,
        "num": page - 1,
    })
}

/// 音乐基因参数（参考 `get_music_gene`）。
fn music_gene_params(euin: &str) -> Value {
    json!({ "VisitAccount": euin })
}

/// 不喜欢列表的游标字段（参考 `lastid_fields`）：2=歌手 / 3=歌曲 / 4=风格。
fn lastid_field(cmd: i64) -> Result<&'static str, UpstreamError> {
    match cmd {
        2 => Ok("SingersLastid"),
        3 => Ok("SongLastid"),
        4 => Ok("StyleLastid"),
        other => Err(UpstreamError::Upstream(format!(
            "不支持的 cmd：{other}（2=歌手 3=歌曲 4=风格）"
        ))),
    }
}

/// 不喜欢列表参数（参考 `get_dislike_list`）：`Cmd` + `Page`，游标非 0 才带。
///
/// 参考只在**给了游标**时才查 `lastid_fields[cmd]`（查不到就是 KeyError），
/// 这里同样只在给游标时校验 `cmd`，其余值原样传给上游。
fn dislike_list_params(params: &Value) -> Result<Value, UpstreamError> {
    let cmd = first_int(params, &["cmd"]).unwrap_or(DISLIKE_CMD_SONG);
    let page = first_int(params, &["page"]).unwrap_or(DEFAULT_PAGE);
    let mut param = json!({ "Cmd": cmd, "Page": page });
    let lastid = first_int(params, &["lastid", "lastId"]).unwrap_or(0);
    if lastid != 0 {
        param[lastid_field(cmd)?] = json!(lastid);
    }
    Ok(param)
}

/// 加/取消不喜欢共用的参数形状（参考的 `keys = {1: "Songs", 2: "Singers", 3: "Styles"}`）：
/// ID 转成**字符串**放进 `ID`，`IdType` 保留数字。
fn dislike_write_params(params: &Value) -> Result<Value, UpstreamError> {
    let id_type = first_int(params, &["idType", "id_type"])
        .ok_or_else(|| UpstreamError::Upstream("缺少 id_type（1=歌曲 2=歌手 3=风格）".into()))?;
    let key = match id_type {
        1 => "Songs",
        2 => "Singers",
        3 => "Styles",
        other => {
            return Err(UpstreamError::Upstream(format!(
                "不支持的 id_type：{other}（1=歌曲 2=歌手 3=风格）"
            )))
        }
    };
    let values = first_array(params, &["values", "ids"])
        .cloned()
        .unwrap_or_default();
    let mut entries: Vec<Value> = Vec::with_capacity(values.len());
    for value in &values {
        let id = coerce_int(value)
            .ok_or_else(|| UpstreamError::Upstream(format!("values 里有非整数：{value}")))?;
        entries.push(json!({ "ID": id.to_string(), "IdType": id_type }));
    }
    let mut param = serde_json::Map::new();
    param.insert(key.to_string(), Value::Array(entries));
    Ok(Value::Object(param))
}

/// 清空不喜欢的第一步参数：参考用 `preserve_bool=True`，`true` 保持布尔值，
/// 不转成 1。
fn cancel_all_token_params() -> Value {
    json!({ "ISOnlyGetToken": true })
}

/// 清空不喜欢的第二步参数。
fn cancel_all_params(token: &str) -> Value {
    json!({ "DelType": DISLIKE_DEL_TYPE_SONG, "Token": token })
}

/// 第一步给的 Token（参考的 `result.get("Token", "")`）。
fn token_of(data: &Value) -> String {
    first_text(data, &["Token", "token"]).unwrap_or_default()
}

fn clear_dislike_token(data: &Value) -> Result<String, UpstreamError> {
    let token = token_of(data);
    if first_int(data, &["Retcode", "retcode"]) != Some(0) || token.trim().is_empty() {
        return Err(UpstreamError::Upstream(
            "清空不喜欢列表的预检未成功或缺少 Token".into(),
        ));
    }
    Ok(token)
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
        "fetch_fav_playlists" => fav_playlists(upstream, credential, platform, params),
        "fetch_fav_albums" => fav_albums(upstream, credential, platform, params),
        "fetch_fav_mvs" => fav_mvs(upstream, credential, platform, params),
        "fetch_music_gene" => music_gene(upstream, credential, platform, params),
        "fetch_dislike_list" => dislike_list(upstream, credential, platform, params),
        "add_dislike" => dislike_add(upstream, credential, platform, params),
        "cancel_dislike" => dislike_cancel(upstream, credential, platform, params),
        "clear_dislike_songs" => dislike_clear(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 收藏的外部歌单（`music.musicasset.PlaylistFavRead / CgiGetPlaylistFavInfo`）。
///
/// 参考没有 `require_login`，读的是 euin 指向的账号；空 `v_list` 是答案，
/// 列表键不在才是故障（见 [`require_list`]）。
fn fav_playlists(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let euin = resolved_euin(upstream, credential, params)?;
    let param = fav_songlist_params(&euin, page_of(params), size_of(params));
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: "music.musicasset.PlaylistFavRead",
            method: "CgiGetPlaylistFavInfo",
            param,
        },
    )?;
    fav_songlist_payload(&data)
}

/// 收藏的专辑（`music.musicasset.AlbumFavRead / CgiGetAlbumFavInfo`）。
fn fav_albums(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let euin = resolved_euin(upstream, credential, params)?;
    let param = fav_album_params(&euin, page_of(params), size_of(params));
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: "music.musicasset.AlbumFavRead",
            method: "CgiGetAlbumFavInfo",
            param,
        },
    )?;
    fav_album_payload(&data)
}

/// 收藏的 MV（`music.musicasset.MVFavRead / getMyFavMV_v2`），参考标了 `require_login`。
fn fav_mvs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let euin = resolved_euin(upstream, credential, params)?;
    let param = fav_mv_params(&euin, page_of(params), size_of(params));
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: "music.musicasset.MVFavRead",
            method: "getMyFavMV_v2",
            param,
        },
    )?;
    fav_mv_payload(&data)
}

/// 音乐基因（`music.recommend.UserProfileSettingSvr / GetProfileReport`）。
fn music_gene(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let euin = resolved_euin(upstream, credential, params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.recommend.UserProfileSettingSvr",
            method: "GetProfileReport",
            param: music_gene_params(&euin),
        },
    )?;
    music_gene_payload(&data)
}

/// 不喜欢列表（`music.feedback.FeedbackBlack / GetDislikeList`，签名路），需要登录。
///
/// 参考的 `pager=True` 是客户端自动翻页：下一页把这一页最后一项的 `id` 回填成
/// `lastid`、`page` 加一。组件一次只给一页，宿主拿回值的最后一项自己回填即可。
fn dislike_list(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = dislike_list_params(params)?;
    require_login(credential)?;
    let data = upstream.call_signed(
        credential,
        Class::Account,
        platform,
        Call {
            module: FEEDBACK_MODULE,
            method: "GetDislikeList",
            param,
        },
        &[],
        None,
    )?;
    Ok(dislike_list_payload(&data))
}

/// 添加不喜欢（`music.feedback.FeedbackBlack / AddDislike`，签名路），需要登录。
///
/// 参考的判据是 `Retcode == 0`；没有 `Retcode` 也算不成功（参考与 `None` 比较）。
fn dislike_add(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = dislike_write_params(params)?;
    require_login(credential)?;
    let data = upstream.call_signed(
        credential,
        Class::Write,
        platform,
        Call {
            module: FEEDBACK_MODULE,
            method: "AddDislike",
            param,
        },
        &[],
        None,
    )?;
    Ok(json!({
        "success": first_int(&data, &["Retcode", "retcode"]) == Some(0),
    }))
}

/// 取消不喜欢（`music.feedback.FeedbackBlack / CancelDislike`，签名路），需要登录。
fn dislike_cancel(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = dislike_write_params(params)?;
    require_login(credential)?;
    let data = upstream.call_signed(
        credential,
        Class::Write,
        platform,
        Call {
            module: FEEDBACK_MODULE,
            method: "CancelDislike",
            param,
        },
        &[],
        None,
    )?;
    Ok(json!({
        "success": first_int(&data, &["Retcode", "retcode"]) == Some(0),
    }))
}

/// 清空所有不喜欢歌曲（`music.feedback.FeedbackBlack / CancelAllDislike`，签名路）。
///
/// 参考是两步：先 `ISOnlyGetToken` 拿 Token（`preserve_bool=True`，布尔值原样发），
/// 再带 `DelType=3` 与 Token 真正删除。参考的返回值只看第二步的 `Retcode == 0`
/// （第一步只要有 Token 就继续）；`params` 里没有业务参数，签名路照发空对象。
fn dislike_clear(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    _params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let first = upstream.call_signed(
        credential,
        Class::Write,
        platform,
        Call {
            module: FEEDBACK_MODULE,
            method: "CancelAllDislike",
            param: cancel_all_token_params(),
        },
        &[],
        None,
    )?;
    let token = clear_dislike_token(&first)?;
    let second = upstream.call_signed(
        credential,
        Class::Write,
        platform,
        Call {
            module: FEEDBACK_MODULE,
            method: "CancelAllDislike",
            param: cancel_all_params(&token),
        },
        &[],
        None,
    )?;
    Ok(json!({
        "success": first_int(&second, &["Retcode", "retcode"]) == Some(0),
    }))
}

// MARK: - 宿主包装
//
// euin 不给（或给空串）时由协议层退回 `Upstream::encrypted_uin`；
// 省略的 page/num 按参考的缺省（第 1 页、每页 10 条）。

/// 收藏的外部歌单一页；空列表表示这个账号没有收藏歌单。
#[export]
pub fn fetch_fav_playlists(
    euin: Option<String>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserFavSonglistResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(euin) = euin.filter(|value| !value.trim().is_empty()) {
        params["euin"] = json!(euin);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_fav_playlists", params)
}

/// 收藏的专辑一页；`total` / `hasmore` 用来翻页。
#[export]
pub fn fetch_fav_albums(
    euin: Option<String>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserFavAlbumResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(euin) = euin.filter(|value| !value.trim().is_empty()) {
        params["euin"] = json!(euin);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_fav_albums", params)
}

/// 收藏的 MV 一页；需要登录，`num` 在协议上是页码减一。
#[export]
pub fn fetch_fav_mvs(
    euin: Option<String>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserFavMvResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(euin) = euin.filter(|value| !value.trim().is_empty()) {
        params["euin"] = json!(euin);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_fav_mvs", params)
}

/// 音乐基因；`euin` 指要看的账号，不给就是本账号。
#[export]
pub fn fetch_music_gene(euin: Option<String>) -> Result<UserMusicGeneResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(euin) = euin.filter(|value| !value.trim().is_empty()) {
        params["euin"] = json!(euin);
    }
    crate::port::call("fetch_music_gene", params)
}

/// 不喜欢列表一页；`cmd`: 2=歌手 3=歌曲 4=风格，`lastid` 是上一页最后一项的 ID。
#[export]
pub fn fetch_dislike_list(
    cmd: Option<i64>,
    page: Option<i64>,
    lastid: Option<i64>,
) -> Result<DislikeListData, crate::HelperError> {
    let mut params = json!({});
    if let Some(cmd) = cmd {
        params["cmd"] = json!(cmd);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(lastid) = lastid {
        params["lastid"] = json!(lastid);
    }
    crate::port::call("fetch_dislike_list", params)
}

/// 添加不喜欢；`id_type`: 1=歌曲 2=歌手 3=风格。返回是否操作成功。
#[export]
pub fn add_dislike(id_type: i64, values: Vec<i64>) -> Result<bool, crate::HelperError> {
    let value: Value = crate::port::call(
        "add_dislike",
        json!({ "idType": id_type, "values": values }),
    )?;
    Ok(value
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// 取消不喜欢；`id_type`: 1=歌曲 2=歌手 3=风格。返回是否操作成功。
#[export]
pub fn cancel_dislike(id_type: i64, values: Vec<i64>) -> Result<bool, crate::HelperError> {
    let value: Value = crate::port::call(
        "cancel_dislike",
        json!({ "idType": id_type, "values": values }),
    )?;
    Ok(value
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// 清空所有不喜欢歌曲（两步：先取 Token 再删除）。返回是否操作成功。
#[export]
pub fn clear_dislike_songs() -> Result<bool, crate::HelperError> {
    let value: Value = crate::port::call("clear_dislike_songs", json!({}))?;
    Ok(value
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一页收藏歌单回值，键名照上游自己的拼写。
    fn raw_fav_songlist() -> Value {
        json!({
            "number": 1,
            "total": 12,
            "hasmore": 0,
            "hide": false,
            "v_list": [{
                "tid": 9578424174i64,
                "dirid": 0,
                "dissname": "一个人的夜",
                "picurl": "http://y.gtimg.cn/music/photo_new/T002R300x300M0000041WVfh2vtlJE.jpg",
                "desc": "简介",
                "songnum": 30,
                "listennum": 1234,
                "uin": "2651932936",
                "nickname": "甲",
                "createtime": 1_600_000_000,
                "updateTime": 1_600_000_100,
                "orderTime": 1_600_000_200,
                "dirShow": 1,
                "dirType": 2,
                "edgeMark": "",
                "layerUrl": "http://y.gtimg.cn/layer.png",
                "albumPicUrl": "http://y.gtimg.cn/album.jpg",
                "opType": 1,
                "sortWeight": 9,
                "readtime": 1_600_000_300
            }],
            "v_delTids": [1, "2"],
            "v_failTids": []
        })
    }

    /// 一页收藏专辑回值。
    fn raw_fav_album() -> Value {
        json!({
            "number": 1,
            "total": 3,
            "hasmore": 1,
            "hide": false,
            "v_list": [{
                "albumID": 87495226,
                "albumMid": "0041WVfh2vtlJE",
                "albumName": "太阳之子",
                "subtitle": "",
                "publishDate": "2026-03-25",
                "logo": "http://y.gtimg.cn/album.jpg",
                "songnum": 13,
                "pubtime": 1_700_000_000,
                "ordertime": 1_700_000_100,
                "status": 1,
                "loc": 0,
                "v_singer": [{ "mid": "0025NhlN2yWrP4", "name": "周杰伦" }]
            }],
            "v_failAlbumId": [42]
        })
    }

    /// 一页收藏 MV 回值（子返回码用参考测试里的 `subcode` 拼写）。
    fn raw_fav_mv() -> Value {
        json!({
            "code": 0,
            "subcode": 0,
            "msg": "",
            "mvlist": [{
                "mvid": 2444488,
                "vid": "002TOGAF0XYfjY",
                "type": 0,
                "mvname": "雨滴",
                "picUrl": "http://y.gtimg.cn/mv.jpg",
                "playcount": 118675,
                "publishDate": 1_700_000_000,
                "singerId": 2749932,
                "singerMid": "00255EBh1FcAEk",
                "singerName": "刘宇",
                "status": 1
            }]
        })
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 8);
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_fav_song",
            &json!({})
        )
        .is_none());
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fav_songlist",
            &json!({})
        )
        .is_none());
    }

    #[test]
    fn the_favourite_pages_turn_page_into_the_reference_offsets() {
        assert_eq!(
            fav_songlist_params("EUIN", 1, 10),
            json!({ "uin": "EUIN", "offset": 0, "size": 10 })
        );
        assert_eq!(
            fav_songlist_params("EUIN", 3, 5),
            json!({ "uin": "EUIN", "offset": 10, "size": 5 })
        );
        assert_eq!(
            fav_album_params("EUIN", 1, 10),
            json!({ "euin": "EUIN", "offset": 0, "size": 10 }),
            "收藏专辑的键是 euin，不是 uin"
        );
        assert_eq!(
            fav_mv_params("EUIN", 3, 10),
            json!({ "encuin": "EUIN", "pagesize": 10, "num": 2 }),
            "参考的 num 是页码减一，照抄"
        );
        assert_eq!(music_gene_params("EUIN"), json!({ "VisitAccount": "EUIN" }));
    }

    #[test]
    fn page_and_size_come_from_the_reference_defaults() {
        assert_eq!(page_of(&json!({})), 1);
        assert_eq!(size_of(&json!({})), 10);
        assert_eq!(page_of(&json!({ "page": 4 })), 4);
        assert_eq!(size_of(&json!({ "limit": 5 })), 5);
        assert_eq!(size_of(&json!({ "size": 7 })), 7);
        // 参考不钳制：给 0 就发 0，由上游决定怎么处理。
        assert_eq!(page_of(&json!({ "page": 0 })), 0);
    }

    #[test]
    fn the_dislike_page_carries_the_cursor_field_of_its_command() {
        assert_eq!(
            dislike_list_params(&json!({})).unwrap(),
            json!({ "Cmd": 3, "Page": 1 }),
            "参考的缺省是 cmd=3、page=1"
        );
        assert_eq!(
            dislike_list_params(&json!({ "cmd": 3, "page": 2, "lastid": 398282803 })).unwrap(),
            json!({ "Cmd": 3, "Page": 2, "SongLastid": 398282803 })
        );
        assert_eq!(
            dislike_list_params(&json!({ "cmd": 2, "lastid": 7 })).unwrap(),
            json!({ "Cmd": 2, "Page": 1, "SingersLastid": 7 })
        );
        assert_eq!(
            dislike_list_params(&json!({ "cmd": 4, "lastid": 9 })).unwrap(),
            json!({ "Cmd": 4, "Page": 1, "StyleLastid": 9 })
        );
        // lastid 为 0 按没给（参考的 `if lastid:`）。
        assert_eq!(
            dislike_list_params(&json!({ "cmd": 3, "lastid": 0 })).unwrap(),
            json!({ "Cmd": 3, "Page": 1 })
        );
        // 参考只在给了游标时才查字段表；没有游标时别的 cmd 原样发。
        assert_eq!(
            dislike_list_params(&json!({ "cmd": 9 })).unwrap(),
            json!({ "Cmd": 9, "Page": 1 })
        );
        let error =
            dislike_list_params(&json!({ "cmd": 9, "lastid": 1 })).expect_err("给游标要认得出 cmd");
        assert!(error.to_string().contains("不支持的 cmd"), "{error}");
    }

    #[test]
    fn a_dislike_write_wraps_every_id_with_its_type() {
        assert_eq!(
            dislike_write_params(&json!({ "idType": 1, "values": [398282803] })).unwrap(),
            json!({ "Songs": [{ "ID": "398282803", "IdType": 1 }] }),
            "ID 是字符串，IdType 是数字"
        );
        assert_eq!(
            dislike_write_params(&json!({ "idType": 2, "values": [1, "2"] })).unwrap(),
            json!({ "Singers": [
                { "ID": "1", "IdType": 2 },
                { "ID": "2", "IdType": 2 }
            ] })
        );
        assert_eq!(
            dislike_write_params(&json!({ "idType": 3, "values": [] })).unwrap(),
            json!({ "Styles": [] }),
            "空列表照参考发空数组"
        );
        let error = dislike_write_params(&json!({ "idType": 9, "values": [1] }))
            .expect_err("认不出的 id_type 要报错");
        assert!(error.to_string().contains("不支持的 id_type"), "{error}");
        assert!(
            dislike_write_params(&json!({ "values": [1] })).is_err(),
            "缺 id_type"
        );
        assert!(
            dislike_write_params(&json!({ "idType": 1, "values": ["abc"] })).is_err(),
            "非整数要报错"
        );
    }

    /// 仅验证清空前的 Token 预检, 不发送删除命令, 不打印 Token.
    #[test]
    #[ignore = "requires credentials and network; never clears existing entries"]
    fn clear_dislike_preflight_without_deletion() {
        let dir = std::env::var("QQMUSIC_HELPER_NEXT_DIR").expect("set credential directory");
        let credential = crate::CredentialStore::for_directory(std::path::Path::new(&dir))
            .load()
            .expect("credential required");
        let data = Upstream::new()
            .call_signed(
                &credential,
                Class::Write,
                Platform::Web,
                Call {
                    module: FEEDBACK_MODULE,
                    method: "CancelAllDislike",
                    param: cancel_all_token_params(),
                },
                &[],
                None,
            )
            .expect("preflight must succeed");
        assert!(clear_dislike_token(&data).is_ok());
    }

    #[test]
    fn the_clear_all_request_asks_for_a_token_first() {
        // 参考的 preserve_bool=True：这里必须是布尔 true，不是 1。
        assert_eq!(cancel_all_token_params(), json!({ "ISOnlyGetToken": true }));
        assert!(cancel_all_token_params()["ISOnlyGetToken"].is_boolean());
        assert_eq!(
            cancel_all_params("TOKEN-1"),
            json!({ "DelType": 3, "Token": "TOKEN-1" })
        );
        assert_eq!(token_of(&json!({ "Token": "T" })), "T");
        assert_eq!(token_of(&json!({})), "", "参考的 `get(\"Token\", \"\")`");
        assert_eq!(token_of(&json!({ "Token": 123 })), "123");
        assert_eq!(
            clear_dislike_token(&json!({"Retcode":0,"Token":"T"})).unwrap(),
            "T"
        );
        for invalid in [
            json!({}),
            json!({"Retcode":1,"Token":"T"}),
            json!({"Retcode":0,"Token":" "}),
        ] {
            assert!(clear_dislike_token(&invalid).is_err());
        }
    }

    #[test]
    fn a_favourite_playlist_page_decodes_its_items() {
        let payload = fav_songlist_payload(&raw_fav_songlist()).expect("解析收藏歌单");
        let response: UserFavSonglistResponse =
            serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(response.number, Some(1));
        assert_eq!(response.total, Some(12));
        assert_eq!(response.hasmore, Some(0));
        assert_eq!(response.hide, Some(false));
        assert_eq!(response.deleted_ids.as_deref(), Some(&[1, 2][..]));
        assert_eq!(response.failed_ids.as_deref(), Some(&[][..]));
        let playlists = response.playlists.expect("有歌单");
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].id, Some(9578424174), "参考的 id 别名含 tid");
        assert_eq!(playlists[0].title.as_deref(), Some("一个人的夜"));
        assert_eq!(playlists[0].desc.as_deref(), Some("简介"));
        assert_eq!(playlists[0].songnum, Some(30));
        assert_eq!(playlists[0].listennum, Some(1234));
        assert_eq!(playlists[0].uin.as_deref(), Some("2651932936"));
        assert_eq!(playlists[0].nickname.as_deref(), Some("甲"));
        assert_eq!(
            playlists[0].create_time,
            Some(1_600_000_000),
            "上游键是 createtime"
        );
        assert_eq!(playlists[0].order_time, Some(1_600_000_200));
        assert_eq!(playlists[0].dir_type, Some(2));
        assert_eq!(playlists[0].sort_weight, Some(9));
        assert_eq!(playlists[0].readtime, Some(1_600_000_300));
        // 宿主看到的字段名与封面协议。
        assert_eq!(
            payload["playlists"][0]["createTime"], 1_600_000_000u64,
            "宿主按 camelCase 读"
        );
        assert_eq!(
            payload["playlists"][0]["picurl"],
            "https://y.gtimg.cn/music/photo_new/T002R300x300M0000041WVfh2vtlJE.jpg",
            "封面一律 https（docs/parsing.md §7）"
        );
        assert_eq!(payload["playlists"][0]["dirid"], 0);
    }

    #[test]
    fn a_favourite_album_page_decodes_its_items() {
        let payload = fav_album_payload(&raw_fav_album()).expect("解析收藏专辑");
        let response: UserFavAlbumResponse =
            serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(response.total, Some(3));
        assert_eq!(response.hasmore, Some(1));
        assert_eq!(response.failed_album_ids.as_deref(), Some(&[42][..]));
        let albums = response.albums.expect("有专辑");
        assert_eq!(albums[0].id, Some(87495226), "上游键是 albumID（大写 ID）");
        assert_eq!(albums[0].mid.as_deref(), Some("0041WVfh2vtlJE"));
        assert_eq!(albums[0].name.as_deref(), Some("太阳之子"));
        assert_eq!(
            albums[0].title.as_deref(),
            Some("太阳之子"),
            "参考里 title 回退到 albumName"
        );
        assert_eq!(albums[0].time_public.as_deref(), Some("2026-03-25"));
        assert_eq!(albums[0].songnum, Some(13));
        assert_eq!(albums[0].pubtime, Some(1_700_000_000));
        assert_eq!(albums[0].ordertime, Some(1_700_000_100));
        assert_eq!(albums[0].status, Some(1));
        let singers = albums[0].singers.as_ref().expect("有歌手");
        assert_eq!(singers[0].mid.as_deref(), Some("0025NhlN2yWrP4"));
        assert_eq!(singers[0].name.as_deref(), Some("周杰伦"));
        assert_eq!(
            payload["albums"][0]["pmid"], "http://y.gtimg.cn/album.jpg",
            "pmid 是 Media ID，不是封面地址，不转协议"
        );
    }

    #[test]
    fn a_favourite_mv_page_accepts_both_subcode_spellings() {
        let payload = fav_mv_payload(&raw_fav_mv()).expect("解析收藏 MV");
        let response: UserFavMvResponse =
            serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(response.code, Some(0));
        assert_eq!(response.sub_code, Some(0));
        let items = response.mv_list.expect("有 MV");
        assert_eq!(
            items[0].id,
            Some(2444488),
            "mvid 与 singerId 同给时 id 取 mvid"
        );
        assert_eq!(items[0].vid.as_deref(), Some("002TOGAF0XYfjY"));
        assert_eq!(items[0].kind, Some(0));
        assert_eq!(items[0].name.as_deref(), Some("雨滴"));
        assert_eq!(items[0].playcount, Some(118675));
        assert_eq!(items[0].publish_date, Some(1_700_000_000));
        assert_eq!(items[0].singer_id, Some(2749932));
        assert_eq!(items[0].singer_mid.as_deref(), Some("00255EBh1FcAEk"));
        assert_eq!(items[0].singer_name.as_deref(), Some("刘宇"));
        assert_eq!(payload["mvList"][0]["singerId"], 2749932);
        assert_eq!(payload["mvList"][0]["type"], 0);

        // 参考里 `id` 的别名含 `singerId`：只给 singerId 时两个字段都拿它。
        let payload = fav_mv_payload(&json!({ "mvlist": [{ "singerId": 7 }] })).unwrap();
        assert_eq!(payload["mvList"][0]["id"], 7);
        assert_eq!(payload["mvList"][0]["singerId"], 7);

        // 两种拼写都收（参考的 `test_fav_mv_response_accepts_subcode_spellings`）。
        for raw in [
            json!({ "mvlist": [], "subCode": 0 }),
            json!({ "mvlist": [], "subcode": 0 }),
        ] {
            let payload = fav_mv_payload(&raw).unwrap();
            let response: UserFavMvResponse =
                serde_json::from_value(payload).expect("解析子返回码");
            assert_eq!(response.sub_code, Some(0), "回值：{raw}");
        }
    }

    #[test]
    fn the_music_gene_card_keeps_its_nested_json() {
        let payload = music_gene_payload(&json!({
            "UserInfoCard": {
                "HeadUrl": "http://y.gtimg.cn/head.jpg",
                "NickName": "甲",
                "Signature": "签名",
                "EncryptionAccount": "7eEFNeSlNKns",
                "Preferences": { "Genre": ["流行"] }
            },
            "ListeningReport": { "Report": [{ "Title": "年度报告", "Score": 90 }] },
            "SortArray": [2, 1, 3],
            "IsVisitAccount": true
        }))
        .expect("解析音乐基因");
        let response: UserMusicGeneResponse =
            serde_json::from_value(payload.clone()).expect("模型解析");
        let card = response.user_info_card.expect("有卡片");
        assert_eq!(card.nick_name.as_deref(), Some("甲"));
        assert_eq!(card.signature.as_deref(), Some("签名"));
        assert_eq!(card.encryption_account.as_deref(), Some("7eEFNeSlNKns"));
        assert_eq!(
            card.head_url.as_deref(),
            Some("https://y.gtimg.cn/head.jpg"),
            "头像一律 https"
        );
        assert_eq!(response.sort_array.as_deref(), Some(&[2, 1, 3][..]));
        assert_eq!(response.is_visit_account, Some(true));
        // 嵌套 JSON 进 Rust 是文本，出 Rust 仍是对象/数组（宿主按真 JSON 读）。
        let preferences: Value =
            serde_json::from_str(card.preferences.as_deref().expect("有偏好")).unwrap();
        assert_eq!(preferences["Genre"][0], "流行");
        let report: Value = serde_json::from_str(
            response
                .listening_report
                .expect("有报告")
                .report
                .as_deref()
                .expect("有报告块"),
        )
        .unwrap();
        assert_eq!(report[0]["Title"], "年度报告");
        assert_eq!(payload["userInfoCard"]["preferences"]["Genre"][0], "流行");
        assert_eq!(payload["listeningReport"]["report"][0]["Score"], 90);
    }

    #[test]
    fn a_dislike_page_reads_the_uppercase_keys() {
        let payload = dislike_list_payload(&json!({
            "Retcode": 0,
            "Msg": "",
            "Singers": [{
                "ID": "123",
                "Name": "某歌手",
                "Img": "http://y.gtimg.cn/a.jpg",
                "IdType": 2,
                "Time": 1_700_000_000
            }],
            "Songs": [{
                "ID": 398282803,
                "Name": "某歌",
                "IdType": 1,
                "Time": 1_700_000_001
            }],
            "Styles": [],
            "Page": 2,
            "Token": "TOKEN"
        }));
        let response: DislikeListData = serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(response.retcode, Some(0));
        assert_eq!(response.page, Some(2));
        assert_eq!(response.token.as_deref(), Some("TOKEN"));
        let singers = response.singers.expect("有歌手");
        assert_eq!(singers[0].id.as_deref(), Some("123"));
        assert_eq!(singers[0].name.as_deref(), Some("某歌手"));
        assert_eq!(singers[0].id_type, Some(2));
        assert_eq!(
            payload["singers"][0]["img"], "https://y.gtimg.cn/a.jpg",
            "封面一律 https"
        );
        let songs = response.songs.expect("有歌曲");
        assert_eq!(
            songs[0].id.as_deref(),
            Some("398282803"),
            "数字 ID 也收成字符串"
        );
        assert_eq!(songs[0].id_type, Some(1));
        assert!(response.styles.expect("有风格字段").is_empty());

        // 一个上限都没有：三个列表缺了按参考的 default_factory 补空数组。
        let response: DislikeListData = serde_json::from_value(dislike_list_payload(&json!({
            "Retcode": 0,
            "Page": 1
        })))
        .expect("解析空回值");
        assert!(response.songs.expect("字段仍在").is_empty());
        assert!(response.singers.expect("字段仍在").is_empty());
        assert!(response.styles.expect("字段仍在").is_empty());
        assert_eq!(response.msg.as_deref(), Some(""));
        assert_eq!(response.token.as_deref(), Some(""));
    }

    #[test]
    fn a_missing_list_key_is_a_shape_fault_not_an_empty_answer() {
        // 键在而列表为空：合法答案（这个账号没有收藏）。
        let payload = fav_songlist_payload(&json!({ "v_list": [], "total": 0 })).unwrap();
        let response: UserFavSonglistResponse = serde_json::from_value(payload).unwrap();
        assert!(response.playlists.expect("字段仍在").is_empty());
        let payload = fav_album_payload(&json!({ "v_list": [] })).unwrap();
        let response: UserFavAlbumResponse = serde_json::from_value(payload).unwrap();
        assert!(response.albums.expect("字段仍在").is_empty());
        let payload = fav_mv_payload(&json!({ "mvlist": [], "code": 0 })).unwrap();
        let response: UserFavMvResponse = serde_json::from_value(payload).unwrap();
        assert!(response.mv_list.expect("字段仍在").is_empty());

        // 键不在：形状不对（会话过期 / 账号标识不对），报错而不是回空列表。
        let error = fav_songlist_payload(&json!({})).expect_err("缺 v_list 要报错");
        assert!(error.to_string().contains("v_list"), "{error}");
        let error = fav_album_payload(&json!({ "v_list": null })).expect_err("null 也算缺");
        assert!(error.to_string().contains("v_list"), "{error}");
        let error = fav_mv_payload(&json!({ "code": 0 })).expect_err("缺 mvlist 要报错");
        assert!(error.to_string().contains("mvlist"), "{error}");

        // 音乐基因的形状锚点是卡片。
        let error = music_gene_payload(&json!({ "SortArray": [] })).expect_err("缺卡片要报错");
        assert!(error.to_string().contains("UserInfoCard"), "{error}");
        // 卡片在、报告缺：那是答案，不是故障。
        let payload = music_gene_payload(&json!({ "UserInfoCard": { "NickName": "甲" } })).unwrap();
        assert!(payload["listeningReport"]["report"].is_null());
    }

    #[test]
    fn an_account_handle_is_the_given_one_or_the_credentials() {
        let upstream = Upstream::new();
        let credential = Credential {
            encrypted_uin: "EUIN".into(),
            ..Credential::default()
        };
        // 给了非空 euin 就用它，不看凭据；空白 euin 按没给，退回凭据里的
        // `encrypt_uin`（参考的 euin 是必填字符串，空串会被上游当查无此人）。
        assert_eq!(
            resolved_euin(&upstream, &credential, &json!({ "euin": "GIVEN" })).unwrap(),
            "GIVEN"
        );
        assert_eq!(
            resolved_euin(&upstream, &credential, &json!({ "euin": "  " })).unwrap(),
            "EUIN"
        );
        // 「没给且凭据里也没有」会去问上游（`GetLoginUserInfo`），那是网络路，
        // 留给真机冒烟；这里只固定本地的取值次序。
        assert_eq!(
            resolved_euin(&upstream, &credential, &json!({})).unwrap(),
            "EUIN",
            "凭据里有 encrypt_uin 时不需要再问上游"
        );
    }

    #[test]
    fn the_signed_reads_refuse_to_go_out_without_a_login() {
        let upstream = Upstream::new();
        let credential = Credential::default();
        // 缺页 / 参数不成立之前先过登录门槛（这两条都在发请求之前）。
        let error = dispatch(
            &upstream,
            &credential,
            Platform::Web,
            "fetch_dislike_list",
            &json!({}),
        )
        .expect("是本层方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("需要登录"), "{error}");
        let error = dispatch(
            &upstream,
            &credential,
            Platform::Web,
            "fetch_fav_mvs",
            &json!({ "euin": "EUIN" }),
        )
        .expect("是本层方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("需要登录"), "{error}");
        let error = dispatch(
            &upstream,
            &credential,
            Platform::Web,
            "add_dislike",
            &json!({ "idType": 1, "values": [1] }),
        )
        .expect("是本层方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("需要登录"), "{error}");
        let error = dispatch(
            &upstream,
            &credential,
            Platform::Web,
            "cancel_dislike",
            &json!({ "idType": 1, "values": [1] }),
        )
        .expect("是本层方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("需要登录"), "{error}");
        let error = dispatch(
            &upstream,
            &credential,
            Platform::Web,
            "clear_dislike_songs",
            &json!({}),
        )
        .expect("是本层方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("需要登录"), "{error}");
    }
}

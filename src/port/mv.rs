//! MV：详情、播放地址、分类列表。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/mv.py`，
//! 回值字段名照 `models/mv.py` 的同名模型（camelCase），模型层级也一样：
//! 详情与播放地址的顶层都是「以 vid 为键的映射」（参考里挂在一个 `data` 字段上），
//! 分类列表是 `total` + `items`。
//!
//! 三个端点：
//! * 详情 `video.VideoDataServer / get_video_info_batch`，参数 `vidlist` 与长
//!   `required` 数组**照参考逐字抄**（含 `uploader_hasfollow` 的重复项）；
//! * 播放地址 `music.stream.MvUrlProxy / GetMvUrls`，`guid` 用本文件里的本地
//!   生成函数（与 `catalog.rs` 的取流实现同源，见 [`guid`]）；
//! * 分类列表 `MvService.MvInfoProServer / GetAllocMvInfo`，页码换算成
//!   `start = num * (page - 1)` 的偏移量（参考的 OffsetStrategy）。
//!
//! 参考的 `@cgi_endpoint` 三个都没标 `platform`，按本层约定默认 Web；
//! `dispatch` 收到调用方传入的 platform 时原样尊重。
//!
//! 空值判据：三个接口在参考里都没有 `require_login`，读的是公开曲库，
//! **空映射 / 空列表是答案**（这批 vid 上游没有资料、这个筛选条件下就是没有 MV），
//! 照常返回，不报错——「整表读取空即故障」那条规则针对的是账号自己的列表，
//! 不适用于这里。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::models::Singer;
use crate::upstream::{
    first_array, first_int, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &["fetch_mv_detail", "resolve_mv_urls", "fetch_mv_list"];

/// 详情接口的长 `required` 数组：照参考逐字抄，**重复的 `uploader_hasfollow`
/// 也保留**（参考就是这么发的）。
const DETAIL_REQUIRED: &[&str] = &[
    "vid",
    "type",
    "sid",
    "cover_pic",
    "duration",
    "singers",
    "video_switch",
    "msg",
    "name",
    "desc",
    "playcnt",
    "pubdate",
    "isfav",
    "gmid",
    "uploader_headurl",
    "uploader_nick",
    "uploader_encuin",
    "uploader_uin",
    "uploader_hasfollow",
    "uploader_follower_num",
    "uploader_hasfollow",
    "related_songs",
];

/// 分类列表的默认筛选：15=全部地区、7=全部类型、0=最新、每页 10 条、第 1 页
/// （参考 `get_mv_list` 的默认参数）。
const LIST_AREA_ALL: i64 = 15;
const LIST_VERSION_ALL: i64 = 7;
const LIST_ORDER_NEWEST: i64 = 0;
const LIST_PAGE_SIZE: i64 = 10;

// MARK: - 模型

/// 参考 `MvDetail`（继承 `MV`）：详情接口的单个视频条目.
///
/// `singers` 在参考里是 `list[dict[str, Any]]`（原样透传的上游形状），这里存
/// JSON 文本：`#[data]` 认不出 `serde_json::Value`，而经 serde 进出时它仍是
/// 真正的对象数组（见 `raw_json_serialize`）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvDetail {
    /// MV 数字 ID（参考 `MV.id`，别名 `sid`/`mvid`/`singerId`）.
    pub id: Option<i64>,
    /// MV VID，请求详情/播放地址的把手.
    pub vid: Option<String>,
    /// MV 类型（参考 `MV.type`，别名 `vt`；Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// MV 名称（参考 `MV.name`，别名 `mvname`/`title`）.
    pub name: Option<String>,
    /// MV 展示标题（参考 `MV.title`，别名 `title_main`/`name`）.
    pub title: Option<String>,
    /// 封面地址.
    pub cover_pic: Option<String>,
    /// MV 时长.
    pub duration: Option<i64>,
    /// MV 歌手列表（参考声明为 dict 列表，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub singers: Option<String>,
    /// MV 开关位.
    pub video_switch: Option<i64>,
    /// 附加消息.
    pub msg: Option<String>,
    /// MV 描述.
    pub desc: Option<String>,
    /// MV 播放量.
    pub playcnt: Option<i64>,
    /// 发布时间戳.
    pub pubdate: Option<i64>,
    /// 是否已收藏.
    pub isfav: Option<i64>,
    /// 全局媒体标识.
    pub gmid: Option<String>,
    /// 上传者头像.
    pub uploader_headurl: Option<String>,
    /// 上传者昵称.
    pub uploader_nick: Option<String>,
    /// 上传者加密 UIN.
    pub uploader_encuin: Option<String>,
    /// 上传者 UIN.
    pub uploader_uin: Option<String>,
    /// 是否已关注上传者.
    pub uploader_hasfollow: Option<i64>,
    /// 上传者粉丝数.
    pub uploader_follower_num: Option<i64>,
    /// 关联歌曲 ID 列表.
    pub related_songs: Option<Vec<i64>>,
}

/// 参考 `GetMvDetailResponse`：以 VID 为键的 MV 详情映射.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvDetailResponse {
    pub data: Option<HashMap<String, MvDetail>>,
}

/// 参考 `MvUrlItem`：单一路径规格下的 MV 播放地址信息.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvUrlItem {
    /// 直连地址列表.
    pub url: Option<Vec<String>>,
    /// 免流地址列表.
    pub freeflow_url: Option<Vec<String>>,
    /// 通用地址列表.
    pub comm_url: Option<Vec<String>>,
    /// 文件名.
    pub cn: Option<String>,
    /// 播放令牌.
    pub vkey: Option<String>,
    /// 过期时间.
    pub expire: Option<i64>,
    /// 结果码.
    pub code: Option<i64>,
    /// 文件类型.
    pub filetype: Option<i64>,
    /// m3u8 地址.
    pub m3u8: Option<String>,
    /// 新文件类型标识（参考字段名 `new_file_type`，上游键就是 `newFileType`）.
    pub new_file_type: Option<i64>,
    /// 编码格式.
    pub format: Option<i64>,
    /// 文件大小（参考字段名 `file_size`，上游键就是 `fileSize`）.
    pub file_size: Option<i64>,
}

/// 参考 `MvUrlSet`：同一 MV 在不同协议下的播放地址集合.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvUrlSet {
    /// MP4 地址列表.
    pub mp4: Option<Vec<MvUrlItem>>,
    /// HLS 地址列表.
    pub hls: Option<Vec<MvUrlItem>>,
    /// 是否支持超清能力标记.
    pub svp_flag: Option<i64>,
    /// MV 时长.
    pub duration: Option<i64>,
}

/// 参考 `GetMvUrlsResponse`：以 MV 标识分组的播放地址集合.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvUrlResponse {
    pub data: Option<HashMap<String, MvUrlSet>>,
}

/// 参考 `MvListItem`（继承 `MV`）：MV 分类列表中的单个 MV 摘要.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvListItem {
    pub id: Option<i64>,
    pub vid: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    pub name: Option<String>,
    pub title: Option<String>,
    /// 歌手列表（参考用基础模型 `Singer`，这里复用组件既有的同名模型）.
    pub singers: Option<Vec<Singer>>,
    /// MV 副标题.
    pub subtitle: Option<String>,
    /// 播放量.
    pub playcnt: Option<i64>,
    /// 发布时间戳.
    pub pubdate: Option<i64>,
    /// 时长（秒）.
    pub duration: Option<i64>,
    /// 封面图片地址.
    pub picurl: Option<String>,
}

/// 参考 `GetMvListResponse`：MV 分类列表接口的响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvListResponse {
    /// 该分类条件下的 MV 总数.
    pub total: Option<i64>,
    /// 当前页 MV 列表（参考模型字段名 `items`，上游键是 `list`）.
    pub items: Option<Vec<MvListItem>>,
}

// MARK: - 参考模型里 `dict` 形状的字段

/// 把 Rust 侧存的 JSON 文本按真 JSON 写出（宿主在 JSON 协议上看到的是对象/数组）。
///
/// 与 `comment.rs` 的同名助手同源：每个领域文件自包含，这份是刻意复制的。
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

/// 取第一个出现的键，原样保留（不透传形状的字段用）。
fn raw_field(raw: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .filter(|found| !found.is_null())
        .cloned()
}

/// 一组字符串（上游有时给数字）。
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

/// 单条 MV 详情（参考 `MvDetail`）。
///
/// 上游详情是 snake_case，这里的候选键把参考的别名表（`MV` 的 id/type/name/title
/// 别名）也覆盖进去，输出统一成 camelCase。
fn detail_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "sid", "mvid", "singerId"]),
        "vid": first_text(raw, &["vid"]),
        "type": first_int(raw, &["type", "vt"]),
        "name": first_text(raw, &["name", "mvname", "title"]),
        "title": first_text(raw, &["title", "title_main", "name"]),
        "coverPic": first_text(raw, &["cover_pic", "coverPic"]),
        "duration": first_int(raw, &["duration"]),
        "singers": raw_field(raw, &["singers"]),
        "videoSwitch": first_int(raw, &["video_switch", "videoSwitch"]),
        "msg": first_text(raw, &["msg"]),
        "desc": first_text(raw, &["desc"]),
        "playcnt": first_int(raw, &["playcnt"]),
        "pubdate": first_int(raw, &["pubdate"]),
        "isfav": first_int(raw, &["isfav"]),
        "gmid": first_text(raw, &["gmid"]),
        "uploaderHeadurl": first_text(raw, &["uploader_headurl", "uploaderHeadurl"]),
        "uploaderNick": first_text(raw, &["uploader_nick", "uploaderNick"]),
        "uploaderEncuin": first_text(raw, &["uploader_encuin", "uploaderEncuin"]),
        "uploaderUin": first_text(raw, &["uploader_uin", "uploaderUin"]),
        "uploaderHasfollow": first_int(raw, &["uploader_hasfollow", "uploaderHasfollow"]),
        "uploaderFollowerNum": first_int(raw, &["uploader_follower_num", "uploaderFollowerNum"]),
        "relatedSongs": raw_field(raw, &["related_songs", "relatedSongs"]),
    })
}

/// 详情回值：上游的 `data` 本身就是「vid → 详情」的映射（参考模型的 jsonpath `$`）。
fn detail_map_payload(data: &Value) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(entries) = data.as_object() {
        for (vid, raw) in entries {
            map.insert(vid.clone(), detail_payload(raw));
        }
    }
    json!({ "data": Value::Object(map) })
}

/// 单条播放地址（参考 `MvUrlItem`）。
fn url_item_payload(raw: &Value) -> Value {
    json!({
        "url": string_list(raw, &["url"]),
        "freeflowUrl": string_list(raw, &["freeflow_url", "freeflowUrl"]),
        "commUrl": string_list(raw, &["comm_url", "commUrl"]),
        "cn": first_text(raw, &["cn"]),
        "vkey": first_text(raw, &["vkey"]),
        "expire": first_int(raw, &["expire"]),
        "code": first_int(raw, &["code"]),
        "filetype": first_int(raw, &["filetype"]),
        "m3u8": first_text(raw, &["m3u8"]),
        "newFileType": first_int(raw, &["newFileType", "new_file_type"]),
        "format": first_int(raw, &["format"]),
        "fileSize": first_int(raw, &["fileSize", "file_size"]),
    })
}

/// 某一协议（`mp4`/`hls`）下的地址列表。
fn url_items(raw: &Value, key: &str) -> Option<Vec<Value>> {
    first_array(raw, &[key]).map(|items| items.iter().map(url_item_payload).collect())
}

/// 一个 MV 的地址集合（参考 `MvUrlSet`）。
fn url_set_payload(raw: &Value) -> Value {
    json!({
        "mp4": url_items(raw, "mp4"),
        "hls": url_items(raw, "hls"),
        "svpFlag": first_int(raw, &["svp_flag", "svpFlag"]),
        "duration": first_int(raw, &["duration"]),
    })
}

/// 播放地址回值：上游的 `data` 是「vid → 地址集合」的映射（参考模型 jsonpath `$`）。
fn url_map_payload(data: &Value) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(entries) = data.as_object() {
        for (vid, raw) in entries {
            map.insert(vid.clone(), url_set_payload(raw));
        }
    }
    json!({ "data": Value::Object(map) })
}

/// 列表里的单个歌手（参考基础模型 `Singer`，复用组件的 `crate::models::Singer`）。
fn singer_payload(raw: &Value) -> Value {
    json!({
        "mid": first_text(raw, &["mid", "singerMid", "singerMID", "SingerMid", "singer_mid"]),
        "name": first_text(raw, &["name", "singerName", "singer_name"]),
    })
}

/// 列表里的单个 MV 摘要（参考 `MvListItem`）。
fn list_item_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "sid", "mvid", "singerId"]),
        "vid": first_text(raw, &["vid"]),
        "type": first_int(raw, &["type", "vt"]),
        "name": first_text(raw, &["name", "mvname", "title"]),
        "title": first_text(raw, &["title", "title_main", "name"]),
        "singers": first_array(raw, &["singers"])
            .map(|items| items.iter().map(singer_payload).collect::<Vec<_>>()),
        "subtitle": first_text(raw, &["subtitle"]),
        "playcnt": first_int(raw, &["playcnt"]),
        "pubdate": first_int(raw, &["pubdate"]),
        "duration": first_int(raw, &["duration"]),
        "picurl": first_text(raw, &["picurl", "picUrl"]),
    })
}

/// 分类列表回值（参考 `GetMvListResponse`）：`total` 与 `list` 都在 `data` 上，
/// 模型里的名字是 `items`。
fn list_payload(data: &Value) -> Value {
    json!({
        "total": first_int(data, &["total", "Total"]),
        "items": first_array(data, &["list", "List"])
            .map(|items| items.iter().map(list_item_payload).collect::<Vec<_>>()),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 详情参数：`vidlist` 是原样的 vid 数组，`required` 照参考逐字抄。
fn detail_params(vids: &[String]) -> Value {
    json!({
        "vidlist": vids,
        "required": DETAIL_REQUIRED,
    })
}

/// 播放地址参数（参考 `get_mv_urls`）：`format 265` 等开关照抄；
/// `guid` 每次请求现生成。
fn urls_params(vids: &[String]) -> Value {
    json!({
        "vids": vids,
        "request_type": 10003,
        "guid": guid(),
        "videoformat": 1,
        "format": 265,
        "dolby": 1,
        "use_new_domain": 1,
        "use_ipv6": 1,
    })
}

/// 分类列表参数：页码换算成偏移量 `start = num * (page - 1)`（参考的
/// OffsetStrategy），**不做区间钳制**——参考把调用方给的值原样传给上游。
fn list_params(params: &Value) -> Value {
    let area = first_int(params, &["area"]).unwrap_or(LIST_AREA_ALL);
    let version = first_int(params, &["version"]).unwrap_or(LIST_VERSION_ALL);
    let order = first_int(params, &["order"]).unwrap_or(LIST_ORDER_NEWEST);
    let num = first_int(params, &["num", "size"]).unwrap_or(LIST_PAGE_SIZE);
    let page = first_int(params, &["page"]).unwrap_or(1);
    json!({
        "area": area,
        "version": version,
        "order": order,
        "start": num * (page - 1),
        "size": num,
    })
}

/// MV 播放地址接口要的 `guid`：上游的每请求设备号，10 位数字即可，CDN 认它。
///
/// 与 `catalog.rs` 的取流实现同源；那个函数是文件私有的，这里等价重写一份
/// （`catalog.rs` 一个字都不动）。
fn guid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64)
        .unwrap_or(0);
    format!("{:010}", (nanos % 9_000_000_000) + 1_000_000_000)
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
        "fetch_mv_detail" => mv_detail(upstream, credential, platform, params),
        "resolve_mv_urls" => mv_urls(upstream, credential, platform, params),
        "fetch_mv_list" => mv_list(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// MV 详情（`video.VideoDataServer / get_video_info_batch`）。
///
/// 空映射是答案：这批 vid 上游没有资料，照常返回空 map。
fn mv_detail(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let vids = string_list(params, &["vidlist", "vids"]).unwrap_or_default();
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "video.VideoDataServer",
            method: "get_video_info_batch",
            param: detail_params(&vids),
        },
    )?;
    Ok(detail_map_payload(&data))
}

/// MV 播放地址（`music.stream.MvUrlProxy / GetMvUrls`）。
///
/// 走 Playback 桶（与取流同类：用户点一次 MV 播一次）。
/// 空映射或全为 0 码之外的条目都是答案——上游没给这条 MV 授权，不是故障。
fn mv_urls(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let vids = string_list(params, &["vids", "vidlist"]).unwrap_or_default();
    let data = upstream.call_with(
        credential,
        Class::Playback,
        platform,
        Call {
            module: "music.stream.MvUrlProxy",
            method: "GetMvUrls",
            param: urls_params(&vids),
        },
    )?;
    Ok(url_map_payload(&data))
}

/// MV 分类列表（`MvService.MvInfoProServer / GetAllocMvInfo`）。
///
/// 空列表是答案：这个筛选条件下就是没有 MV。
fn mv_list(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "MvService.MvInfoProServer",
            method: "GetAllocMvInfo",
            param: list_params(params),
        },
    )?;
    Ok(list_payload(&data))
}

// MARK: - 宿主包装

/// 批量获取 MV 详情；回值 `data` 以 vid 为键。
#[export]
pub fn fetch_mv_detail(vids: Vec<String>) -> Result<MvDetailResponse, crate::HelperError> {
    crate::port::call("fetch_mv_detail", json!({ "vidlist": vids }))
}

/// 批量获取 MV 播放地址；回值 `data` 以 vid 为键，每条含 `mp4`/`hls` 两组地址。
#[export]
pub fn resolve_mv_urls(vids: Vec<String>) -> Result<MvUrlResponse, crate::HelperError> {
    crate::port::call("resolve_mv_urls", json!({ "vids": vids }))
}

/// MV 分类列表一页；省略的参数按参考的默认值走
/// （15=全部地区、7=全部类型、0=最新、10 条、第 1 页）。
#[export]
pub fn fetch_mv_list(
    area: Option<i64>,
    version: Option<i64>,
    order: Option<i64>,
    num: Option<i64>,
    page: Option<i64>,
) -> Result<MvListResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(area) = area {
        params["area"] = json!(area);
    }
    if let Some(version) = version {
        params["version"] = json!(version);
    }
    if let Some(order) = order {
        params["order"] = json!(order);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    crate::port::call("fetch_mv_list", params)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 详情回值的一份样例，键名照上游自己的拼写。
    fn raw_detail_map() -> Value {
        json!({
            "013xscuH0xlbie": {
                "vid": "013xscuH0xlbie",
                "type": 1,
                "sid": 13641889,
                "cover_pic": "https://y.gtimg.cn/music/photo_new/T053R750x750M000001UUMpF0ZfQZy.jpg",
                "duration": 15,
                "singers": [{"id": 1, "mid": "00255EBh1FcAEk", "name": "某歌手"}],
                "video_switch": 2047,
                "msg": "",
                "name": "藤井风为HONDA「VEZEL」新CM演唱新曲",
                "desc": "描述",
                "playcnt": 2321,
                "pubdate": 1619059631,
                "isfav": 0,
                "gmid": "013xscuH0xlbie",
                "uploader_headurl": "https://y.gtimg.cn/head.jpg",
                "uploader_nick": "上传者",
                "uploader_encuin": "owCkoKEiowEi7c**",
                "uploader_uin": "2651932936",
                "uploader_hasfollow": 0,
                "uploader_follower_num": 4728,
                "related_songs": [212622854]
            }
        })
    }

    #[test]
    fn detail_params_carry_the_reference_required_array_verbatim() {
        let param = detail_params(&["013xscuH0xlbie".to_string()]);
        assert_eq!(param["vidlist"], json!(["013xscuH0xlbie"]));
        assert_eq!(
            param["required"],
            json!([
                "vid",
                "type",
                "sid",
                "cover_pic",
                "duration",
                "singers",
                "video_switch",
                "msg",
                "name",
                "desc",
                "playcnt",
                "pubdate",
                "isfav",
                "gmid",
                "uploader_headurl",
                "uploader_nick",
                "uploader_encuin",
                "uploader_uin",
                "uploader_hasfollow",
                "uploader_follower_num",
                "uploader_hasfollow",
                "related_songs"
            ])
        );
        // 参考里 `uploader_hasfollow` 出现两次，这里也保留两次。
        let required = param["required"].as_array().unwrap();
        assert_eq!(required.len(), 22);
        assert_eq!(
            required
                .iter()
                .filter(|key| *key == "uploader_hasfollow")
                .count(),
            2
        );
    }

    #[test]
    fn a_detail_map_maps_every_field_the_reference_model_names() {
        let payload = detail_map_payload(&raw_detail_map());
        let response: MvDetailResponse = serde_json::from_value(payload.clone()).expect("解析详情");
        let map = response.data.as_ref().expect("有映射");
        let detail = map.get("013xscuH0xlbie").expect("按 vid 取详情");
        assert_eq!(detail.id, Some(13641889), "参考的 id 别名含 sid");
        assert_eq!(detail.vid.as_deref(), Some("013xscuH0xlbie"));
        assert_eq!(detail.kind, Some(1));
        assert_eq!(
            detail.name.as_deref(),
            Some("藤井风为HONDA「VEZEL」新CM演唱新曲")
        );
        assert_eq!(
            detail.title.as_deref(),
            Some("藤井风为HONDA「VEZEL」新CM演唱新曲"),
            "参考里 title 回退到 name"
        );
        assert_eq!(detail.duration, Some(15));
        assert_eq!(detail.video_switch, Some(2047));
        assert_eq!(detail.playcnt, Some(2321));
        assert_eq!(detail.pubdate, Some(1619059631));
        assert_eq!(detail.uploader_uin.as_deref(), Some("2651932936"));
        assert_eq!(detail.uploader_follower_num, Some(4728));
        assert_eq!(detail.related_songs.as_deref(), Some(&[212622854][..]));
        // 参考声明 singers 是 dict 列表，透传后展开仍是对象数组。
        let singers: Value =
            serde_json::from_str(detail.singers.as_deref().expect("有歌手")).unwrap();
        assert_eq!(singers[0]["mid"], "00255EBh1FcAEk");
        // 宿主按 camelCase 读，且嵌套对象/数组的形状不变。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["data"]["013xscuH0xlbie"]["type"], 1);
        assert_eq!(
            round["data"]["013xscuH0xlbie"]["coverPic"],
            "https://y.gtimg.cn/music/photo_new/T053R750x750M000001UUMpF0ZfQZy.jpg"
        );
        assert_eq!(round["data"]["013xscuH0xlbie"]["uploaderFollowerNum"], 4728);
        assert_eq!(
            round["data"]["013xscuH0xlbie"]["relatedSongs"][0],
            212622854
        );
        assert_eq!(
            round["data"]["013xscuH0xlbie"]["singers"][0]["name"],
            "某歌手"
        );
    }

    #[test]
    fn mv_url_params_carry_the_reference_switches_and_a_ten_digit_guid() {
        let param = urls_params(&["013xscuH0xlbie".to_string()]);
        assert_eq!(param["vids"], json!(["013xscuH0xlbie"]));
        assert_eq!(param["request_type"], 10003);
        assert_eq!(param["videoformat"], 1);
        assert_eq!(param["format"], 265);
        assert_eq!(param["dolby"], 1);
        assert_eq!(param["use_new_domain"], 1);
        assert_eq!(param["use_ipv6"], 1);
        let guid = param["guid"].as_str().expect("guid 是字符串");
        assert_eq!(guid.len(), 10);
        assert!(guid.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn a_url_map_keeps_the_protocol_groups_and_their_sizes() {
        let payload = url_map_payload(&json!({
            "013xscuH0xlbie": {
                "mp4": [{
                    "url": [],
                    "freeflow_url": ["http://v0.stream.tencentmusic.com/a.f120000.mp4"],
                    "comm_url": [],
                    "cn": "a.f120000.mp4",
                    "vkey": "",
                    "expire": 21570,
                    "code": 0,
                    "filetype": 20,
                    "m3u8": "",
                    "newFileType": 20,
                    "format": 264,
                    "fileSize": 1719554
                }],
                "hls": [{
                    "url": [],
                    "freeflow_url": ["http://v0.stream.tencentmusic.com/a.f220000.m3u8"],
                    "comm_url": [],
                    "cn": "a.f220000.m3u8",
                    "vkey": "",
                    "expire": 21570,
                    "code": 0,
                    "filetype": 20,
                    "m3u8": "",
                    "newFileType": 20,
                    "format": 264,
                    "fileSize": 827576
                }],
                "svp_flag": 1,
                "duration": 15
            }
        }));
        let response: MvUrlResponse = serde_json::from_value(payload).expect("解析播放地址");
        // 先留一份宿主看到的形状，再从模型里读字段（读字段会把 `data` 移走）。
        let round: Value = serde_json::to_value(&response).unwrap();
        let mut map = response.data.expect("有映射");
        let set = map.remove("013xscuH0xlbie").expect("按 vid 取地址集");
        assert_eq!(set.svp_flag, Some(1));
        assert_eq!(set.duration, Some(15));
        let mp4 = set.mp4.expect("有 mp4");
        assert_eq!(mp4[0].file_size, Some(1719554));
        assert_eq!(mp4[0].new_file_type, Some(20));
        assert_eq!(
            mp4[0].freeflow_url.as_deref(),
            Some(&["http://v0.stream.tencentmusic.com/a.f120000.mp4".to_string()][..])
        );
        assert_eq!(set.hls.expect("有 hls")[0].filetype, Some(20));

        assert_eq!(
            round["data"]["013xscuH0xlbie"]["mp4"][0]["fileSize"],
            1719554
        );
        assert_eq!(round["data"]["013xscuH0xlbie"]["mp4"][0]["newFileType"], 20);
        assert_eq!(
            round["data"]["013xscuH0xlbie"]["mp4"][0]["freeflowUrl"][0],
            "http://v0.stream.tencentmusic.com/a.f120000.mp4"
        );
        assert_eq!(round["data"]["013xscuH0xlbie"]["svpFlag"], 1);
    }

    #[test]
    fn mv_list_params_turn_page_into_the_reference_offset() {
        assert_eq!(
            list_params(&json!({})),
            json!({"area": 15, "version": 7, "order": 0, "start": 0, "size": 10}),
            "参考默认：全部地区、全部类型、最新、10 条、第 1 页"
        );
        assert_eq!(
            list_params(&json!({"area": 8, "version": 13, "order": 1, "num": 5, "page": 3})),
            json!({"area": 8, "version": 13, "order": 1, "start": 10, "size": 5})
        );
    }

    #[test]
    fn a_list_page_reports_its_total_and_typed_items() {
        let payload = list_payload(&json!({
            "list": [{
                "mvid": 2207646,
                "vid": "001UZToL4FDJ9b",
                "title": "雨滴",
                "subtitle": "",
                "picurl": "http://y.gtimg.cn/music/photo_new/T015R640x360M101001UZToL4FDJ9b.jpg",
                "playcnt": 17978258,
                "pubdate": 1737820800,
                "duration": 215,
                "singers": [{
                    "id": 2749932,
                    "mid": "00255EBh1FcAEk",
                    "name": "刘宇",
                    "picurl": "http://y.gtimg.cn/music/photo_new/T001R150x150M00000255EBh1FcAEk_17.jpg"
                }]
            }],
            "total": 1000
        }));
        let response: MvListResponse = serde_json::from_value(payload).expect("解析列表");
        assert_eq!(response.total, Some(1000));
        let items = response.items.expect("有列表");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, Some(2207646), "参考的 id 别名含 mvid");
        assert_eq!(items[0].vid.as_deref(), Some("001UZToL4FDJ9b"));
        assert_eq!(items[0].title.as_deref(), Some("雨滴"));
        assert_eq!(
            items[0].name.as_deref(),
            Some("雨滴"),
            "参考里 name 回退到 title"
        );
        assert_eq!(items[0].playcnt, Some(17978258));
        assert_eq!(items[0].pubdate, Some(1737820800));
        assert_eq!(items[0].duration, Some(215));
        let singers = items[0].singers.as_ref().expect("有歌手");
        assert_eq!(singers[0].mid.as_deref(), Some("00255EBh1FcAEk"));
        assert_eq!(singers[0].name.as_deref(), Some("刘宇"));
    }

    #[test]
    fn an_empty_map_or_list_is_an_answer_not_a_failure() {
        // 三个接口都不是 require_login 的整表读取：空是合法答案，照常解析。
        let response: MvDetailResponse =
            serde_json::from_value(detail_map_payload(&json!({}))).expect("空详情映射照常解析");
        assert!(response.data.expect("有 data").is_empty());

        let response: MvUrlResponse =
            serde_json::from_value(url_map_payload(&json!({}))).expect("空地址映射照常解析");
        assert!(response.data.expect("有 data").is_empty());

        let response: MvListResponse = serde_json::from_value(list_payload(&json!({
            "list": [],
            "total": 0
        })))
        .expect("空列表照常解析");
        assert_eq!(response.total, Some(0));
        assert!(response.items.expect("有 items").is_empty());
    }

    #[test]
    fn a_missing_list_key_leaves_items_absent_not_empty() {
        // 上游没给 `list` 与「给了空 list」是两件事：前者是形状不对，items 保持
        // None，宿主可以据此判断这次不是「没有 MV」而是响应不成形。
        let response: MvListResponse = serde_json::from_value(list_payload(&json!({"total": 12})))
            .expect("只有 total 也能解析");
        assert_eq!(response.total, Some(12));
        assert!(response.items.is_none());
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(
            METHODS,
            &["fetch_mv_detail", "resolve_mv_urls", "fetch_mv_list"]
        );
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

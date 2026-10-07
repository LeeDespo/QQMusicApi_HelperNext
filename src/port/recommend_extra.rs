//! 推荐扩展：首页信息流、雷达推荐、推荐歌单。
//!
//! 当前公开契约见 `docs/endpoints.md`，解析与兼容规则见 `docs/parsing.md`。
//!
//! 三个端点：
//! * 首页信息流 `music.recommend.RecommendFeed / get_recommend_feed`；
//! * 雷达推荐 `music.recommend.TrackRelationServer / GetRadarSong`；
//! * 推荐歌单 `music.playlist.PlaylistSquare / GetRecommendFeed`。
//!
//! # 平台档案
//!
//! 三个端点参考都没标 `platform`，按本层约定默认 Web；`dispatch` 收到调用方传入的
//! platform 时原样尊重。（docs/parsing.md §1 把「推荐流」归在 android 档案下，
//! 那是既有 `fetch_recommend_feed` 的默认；宿主若发现 Web 档案回空，
//! 在参数里传 `platform: "android"` 即可。）
//!
//! # 翻页（参考的 pager 策略；组件只做单次请求，状态由宿主持有）
//!
//! * 首页信息流：参考的 `MultiFieldContinuationStrategy` 按回值续参——下一次
//!   `direction=1`、`page` 加一、`s_num` 加上本页楼层数、`v_cache` 并入本页各楼层的
//!   id（参考就是这么防重复推荐的）；
//! * 雷达推荐：页码翻页（`Page`），回值 `hasMore` 为真时 `Page` 加一；
//! * 推荐歌单：游标翻页，下一次的 `From` 用回值里的 `fromLimit`，`Size` 不变。
//!
//! # 空值判据
//!
//! 三个接口在参考里都没有 `require_login`（参考的 web 路由也是公开缓存），读的是公开
//! 推荐内容：**空列表是答案**（这一刻上游就是没有推荐），照常返回，不报错。
//! 但「列表键不在」与「给了空列表」是两件事：前者形状不对，模型字段保持 `None`，
//! 宿主可以据此判断这次不是「没有推荐」而是响应不成形（与 `mv.rs` 的 `items`
//! 同款判据）。参考模型里 `more` / `cards` / `videoCards` 声明为
//! `dict` / `list[dict]`（原样透传的上游形状），这里也原样透传。
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

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "fetch_home_feed",
    "fetch_radar_recommend",
    "fetch_recommend_playlists",
];

/// 参考的缺省页码（三个接口共用）。
const DEFAULT_PAGE: i64 = 1;
/// 首页信息流的缺省刷新方向：首次是 0，参考的续参把它改成 1。
const HOME_FEED_DIRECTION_FIRST: i64 = 0;
/// 雷达推荐写死的 `ReqType`（参考逐字写 0）。
const RADAR_REQ_TYPE: i64 = 0;
/// 推荐歌单的缺省每页数量（参考 `get_recommend_songlist(num=25)`）。
const SONGLIST_DEFAULT_NUM: i64 = 25;

// MARK: - 模型

/// 参考 `RecommendNiche`：首页推荐楼层中的细分卡片分组.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendNiche {
    /// 细分分组 ID.
    pub id: Option<i64>,
    /// 标题模板.
    pub title_template: Option<String>,
    /// 标题实际展示内容.
    pub title_content: Option<String>,
    /// 原始卡片列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub cards: Option<String>,
}

/// 参考 `RecommendShelf`：首页推荐页中的单个楼层.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendShelf {
    /// 楼层 ID.
    pub id: Option<i64>,
    /// 楼层标题模板.
    pub title_template: Option<String>,
    /// 楼层标题实际展示内容.
    pub title_content: Option<String>,
    /// 更多入口信息（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more: Option<String>,
    /// 楼层下属的细分分组列表.
    pub niches: Option<Vec<RecommendNiche>>,
}

/// 参考 `RecommendFeedCardResponse`：首页推荐首屏响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendFeedCardResponse {
    /// 接口返回码.
    pub retcode: Option<i64>,
    /// 附加消息.
    pub msg: Option<String>,
    /// 提示信息.
    pub prompt: Option<String>,
    /// 分页或批次计数信息（参考字段是 snake_case，输出 `dNum`）.
    pub d_num: Option<i64>,
    /// 继续加载标记（参考字段是 snake_case，输出 `loadMark`）.
    pub load_mark: Option<i64>,
    /// 首页推荐楼层列表.
    pub shelves: Option<Vec<RecommendShelf>>,
}

/// 参考 `RadarRecommendResponse`：雷达推荐响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadarRecommendResponse {
    /// 推荐歌曲列表（参考的 jsonpath 是 `$.VecSongs[*].Track`）.
    pub songs: Option<Vec<Track>>,
    /// 推荐歌曲 ID 列表.
    pub recommend_song_ids: Option<Vec<i64>>,
    /// 作为推荐依据的基础歌曲 ID 列表.
    pub base_song_ids: Option<Vec<i64>>,
    /// 是否还能继续获取更多推荐（翻页依据）.
    pub has_more: Option<bool>,
    /// 提示信息块或提示文案.
    pub toast: Option<String>,
    /// 服务端时间戳.
    pub timestamp: Option<i64>,
    /// 关联视频卡片数据（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub video_cards: Option<String>,
}

/// 参考 `RecommendSonglistItem`（继承基础模型 `SongList`）：推荐歌单里的单个条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendSonglistItem {
    /// 歌单 ID（参考 `SongList.id`，别名 `tid`/`dissid`）.
    pub id: Option<i64>,
    /// 目录 ID.
    pub dirid: Option<i64>,
    /// 歌单标题.
    pub title: Option<String>,
    /// 歌单封面地址（参考的 jsonpath 是 `$.cover.default_url`）.
    pub picurl: Option<String>,
    /// 歌单简介.
    pub desc: Option<String>,
    /// 歌单歌曲数量（参考别名 `song_cnt`/`songnum`/`songNum`）.
    pub songnum: Option<i64>,
    /// 歌单播放量（参考别名 `play_cnt`/`listennum`/`playCnt`）.
    pub listennum: Option<i64>,
    /// 创建者昵称（参考的 jsonpath 是 `$.creator.nick`）.
    pub creator_nick: Option<String>,
}

/// 参考 `RecommendSonglistResponse`：推荐歌单分页响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecommendSonglistResponse {
    /// 当前批次推荐歌单列表（参考的 jsonpath 是 `$.List[*].Playlist.basic`）.
    pub songlists: Option<Vec<RecommendSonglistItem>>,
    /// 是否还能继续拉取更多歌单.
    pub has_more: Option<bool>,
    /// 当前批次对应的偏移（下一次请求的 `From` 就用它）.
    pub from_limit: Option<i64>,
    /// 附加消息.
    pub msg: Option<String>,
}

// MARK: - 参考模型里 `dict` 形状的字段

/// 把 Rust 侧存的 JSON 文本按真 JSON 写出（宿主在 JSON 协议上看到的是对象/数组）。
///
/// 与 `mv.rs` 的同名助手同源：每个领域文件自包含，这份是刻意复制的。
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

// MARK: - 通用小工具（每个领域文件自包含）

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

/// 一组整数（上游给数字也给数字字符串）。
fn int_list(raw: &Value, keys: &[&str]) -> Option<Vec<i64>> {
    first_array(raw, keys).map(|items| {
        items
            .iter()
            .filter_map(|item| match item {
                Value::Number(number) => number.as_i64(),
                Value::String(text) => text.trim().parse::<i64>().ok(),
                _ => None,
            })
            .collect()
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

/// 封面 / 头像类 URL 一律转 https（docs/parsing.md §7：宿主会直接拒绝 http 图片）。
fn artwork(value: Option<String>) -> Option<String> {
    crate::methods::normalized_artwork_url(value.as_deref())
}

// MARK: - 回值搬运（上游形状 → 参考模型形状）

/// 单个细分卡片分组（参考 `RecommendNiche`）。
fn niche_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id"]),
        "titleTemplate": first_text(raw, &["title_template", "titleTemplate"]),
        "titleContent": first_text(raw, &["title_content", "titleContent"]),
        // 参考别名是 `v_card`，原样透传。
        "cards": raw_field(raw, &["v_card", "vCard", "cards"]),
    })
}

/// 单个推荐楼层（参考 `RecommendShelf`）。
fn shelf_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id"]),
        "titleTemplate": first_text(raw, &["title_template", "titleTemplate"]),
        "titleContent": first_text(raw, &["title_content", "titleContent"]),
        "more": raw_field(raw, &["more"]),
        // 参考别名是 `v_niche`。
        "niches": first_array(raw, &["v_niche", "vNiche", "niches"])
            .map(|items| items.iter().map(niche_payload).collect::<Vec<_>>()),
    })
}

/// 首页信息流回值（参考 `RecommendFeedCardResponse`）：楼层在 `v_shelf` 上，
/// 每层的分组在 `v_niche` 上，分组的卡片在 `v_card` 上。
fn home_feed_payload(data: &Value) -> Value {
    json!({
        "retcode": first_int(data, &["retcode", "Retcode"]),
        "msg": first_text(data, &["msg", "Msg"]),
        "prompt": first_text(data, &["prompt", "Prompt"]),
        "dNum": first_int(data, &["d_num", "dNum"]),
        "loadMark": first_int(data, &["load_mark", "loadMark"]),
        "shelves": first_array(data, &["v_shelf", "vShelf", "shelves"])
            .map(|items| items.iter().map(shelf_payload).collect::<Vec<_>>()),
    })
}

/// 雷达推荐回值（参考 `RadarRecommendResponse`）。
///
/// 歌曲按参考的 jsonpath `$.VecSongs[*].Track` 抽出来，曲目本体复用组件既有的
/// 曲目解码（`methods::decoded_tracks`）。
fn radar_payload(data: &Value) -> Value {
    // 列表键在不在与「给了空列表」是两件事：键不在时 songs 保持 None。
    let songs = first_array(data, &["VecSongs", "vecSongs"]).map(|items| {
        let tracks = items
            .iter()
            .map(|entry| {
                first_object(entry, &["Track", "track"])
                    .unwrap_or(entry)
                    .clone()
            })
            .collect::<Vec<_>>();
        crate::methods::decoded_tracks(&json!({ "songs": tracks }))
    });
    json!({
        "songs": songs,
        "recommendSongIds": int_list(data, &["RecommendSongIds", "recommendSongIds"]),
        "baseSongIds": int_list(data, &["BaseSongIds", "baseSongIds"]),
        "hasMore": bool_field(data, &["HasMore", "hasMore"]),
        "toast": first_text(data, &["toast", "Toast"]),
        "timestamp": first_int(data, &["TimeStamp", "timestamp"]),
        // 参考别名是 `VideoCards`，原样透传。
        "videoCards": raw_field(data, &["VideoCards", "videoCards"]),
    })
}

/// 单个推荐歌单（参考 `RecommendSonglistItem`，继承 `SongList` 的字段与别名）。
fn recommend_songlist_item_payload(raw: &Value) -> Value {
    // 封面在参考里走 jsonpath `$.cover.default_url`；命中就优先用它，
    // 否则退回 `SongList.picurl` 的别名表。
    let cover = first_object(raw, &["cover", "Cover"])
        .and_then(|cover| first_text(cover, &["default_url", "defaultUrl", "url"]));
    json!({
        "id": first_int(raw, &["id", "tid", "dissid", "dissId"]),
        "dirid": first_int(raw, &["dirid", "dirId"]),
        "title": first_text(raw, &["title", "dissname", "name", "dirName"]),
        "picurl": artwork(cover.or_else(|| first_text(raw, &["picurl", "picUrl", "logo"]))),
        "desc": first_text(raw, &["desc", "description"]),
        "songnum": first_int(raw, &["song_cnt", "songnum", "songNum"]),
        "listennum": first_int(raw, &["play_cnt", "listennum", "playCnt"]),
        // 创建者昵称在参考里走 jsonpath `$.creator.nick`。
        "creatorNick": first_object(raw, &["creator", "Creator"])
            .and_then(|creator| first_text(creator, &["nick", "name"])),
    })
}

/// 推荐歌单回值（参考 `RecommendSonglistResponse`）：列表在 `List` 上，
/// 每项是 `Playlist.basic`（参考的 jsonpath `$.List[*].Playlist.basic`）。
fn recommend_songlist_payload(data: &Value) -> Value {
    let items = first_array(data, &["List", "list"]).map(|items| {
        items
            .iter()
            .filter_map(|entry| first_object(entry, &["Playlist", "playlist"]))
            .filter_map(|playlist| first_object(playlist, &["basic", "Basic"]))
            .map(recommend_songlist_item_payload)
            .collect::<Vec<_>>()
    });
    json!({
        "songlists": items,
        "hasMore": bool_field(data, &["HasMore", "hasMore"]),
        "fromLimit": first_int(data, &["FromLimit", "fromLimit"]),
        "msg": first_text(data, &["Msg", "msg"]),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 首页信息流参数（参考 `get_home_feed`）：`page=1`、`direction=0`、`s_num=0`、
/// `v_cache=[]` 是缺省；续参规则见文件头的「翻页」。
fn home_feed_params(params: &Value) -> Value {
    json!({
        "direction": first_int(params, &["direction"]).unwrap_or(HOME_FEED_DIRECTION_FIRST),
        "page": first_int(params, &["page"]).unwrap_or(DEFAULT_PAGE),
        "s_num": first_int(params, &["s_num", "sNum"]).unwrap_or(0),
        "v_cache": string_list(params, &["v_cache", "vCache"]).unwrap_or_default(),
    })
}

/// 雷达推荐参数（参考 `get_radar_recommend`）：两个空列表照参考逐字发。
fn radar_params(params: &Value) -> Value {
    json!({
        "Page": first_int(params, &["Page", "page"]).unwrap_or(DEFAULT_PAGE),
        "ReqType": RADAR_REQ_TYPE,
        "FavSongs": [],
        "EntranceSongs": [],
    })
}

/// 推荐歌单参数（参考 `get_recommend_songlist`）：页码换算成偏移
/// `From = num * (page - 1)`（参考的 CursorStrategy），**不做区间钳制**——
/// 参考把调用方给的值原样传给上游。
fn recommend_songlist_params(params: &Value) -> Value {
    let page = first_int(params, &["page", "Page"]).unwrap_or(DEFAULT_PAGE);
    let num = first_int(params, &["num", "Num", "size"]).unwrap_or(SONGLIST_DEFAULT_NUM);
    json!({
        "From": num * (page - 1),
        "Size": num,
    })
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
        "fetch_home_feed" => home_feed(upstream, credential, platform, params),
        "fetch_radar_recommend" => radar_recommend(upstream, credential, platform, params),
        "fetch_recommend_playlists" => recommend_playlists(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 首页信息流（`music.recommend.RecommendFeed / get_recommend_feed`）。
///
/// 空楼层列表是答案：这一刻上游没有可推荐的。
fn home_feed(
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
            module: "music.recommend.RecommendFeed",
            method: "get_recommend_feed",
            param: home_feed_params(params),
        },
    )?;
    Ok(home_feed_payload(&data))
}

/// 雷达推荐（`music.recommend.TrackRelationServer / GetRadarSong`）。
///
/// 与 `song_related.rs` 的相似歌曲同一个模块，但这里读的是账号的雷达流；
/// 空歌曲列表是答案。
fn radar_recommend(
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
            module: "music.recommend.TrackRelationServer",
            method: "GetRadarSong",
            param: radar_params(params),
        },
    )?;
    Ok(radar_payload(&data))
}

/// 推荐歌单（`music.playlist.PlaylistSquare / GetRecommendFeed`）。
///
/// 空歌单列表是答案：这个批次上游没有推荐歌单。
fn recommend_playlists(
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
            module: "music.playlist.PlaylistSquare",
            method: "GetRecommendFeed",
            param: recommend_songlist_params(params),
        },
    )?;
    Ok(recommend_songlist_payload(&data))
}

// MARK: - 宿主包装
//
// 省略的参数按参考的缺省走（第 1 页、方向 0、已加载 0 张、25 条）。翻页状态由
// 宿主自己持有，规则见文件头「翻页」。

/// 首页信息流一页；`v_cache` 传已曝光的楼层 ID，防止重复推荐。
#[export]
pub fn fetch_home_feed(
    page: Option<i64>,
    direction: Option<i64>,
    s_num: Option<i64>,
    v_cache: Option<Vec<String>>,
) -> Result<RecommendFeedCardResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(direction) = direction {
        params["direction"] = json!(direction);
    }
    if let Some(s_num) = s_num {
        params["s_num"] = json!(s_num);
    }
    if let Some(v_cache) = v_cache {
        params["v_cache"] = json!(v_cache);
    }
    crate::port::call("fetch_home_feed", params)
}

/// 雷达推荐一页；`page` 省略时是第 1 页，回值 `hasMore` 为真时翻下一页。
#[export]
pub fn fetch_radar_recommend(
    page: Option<i64>,
) -> Result<RadarRecommendResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    crate::port::call("fetch_radar_recommend", params)
}

/// 推荐歌单一页；`num` 省略时是 25 条，回值 `fromLimit` 就是下一页的 `From`。
#[export]
pub fn fetch_recommend_playlists(
    page: Option<i64>,
    num: Option<i64>,
) -> Result<RecommendSonglistResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_recommend_playlists", params)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 首页信息流回值的一份样例，键名照上游自己的拼写（snake_case + `v_*` 列表名）。
    fn raw_home_feed() -> Value {
        json!({
            "retcode": 0,
            "msg": "",
            "prompt": "为你推荐",
            "d_num": 3,
            "load_mark": 1,
            "v_shelf": [{
                "id": 100,
                "title_template": "晚间推荐",
                "title_content": "晚间推荐 · 华语",
                "more": {"type": 1, "id": 200, "title": "查看全部"},
                "v_niche": [{
                    "id": 1,
                    "title_template": "华语",
                    "title_content": "华语精选",
                    "v_card": [{"id": 11, "type": 1, "title": "卡片"}]
                }]
            }]
        })
    }

    #[test]
    fn home_feed_params_carry_the_reference_defaults_and_continuation() {
        assert_eq!(
            home_feed_params(&json!({})),
            json!({"direction": 0, "page": 1, "s_num": 0, "v_cache": []}),
            "参考缺省：第 1 页、方向 0、已加载 0 张、没有曝光缓存"
        );
        // 参考续参：direction=1、page+1、s_num 加本页楼层数、v_cache 并入楼层 id。
        assert_eq!(
            home_feed_params(&json!({
                "direction": 1,
                "page": 2,
                "s_num": 4,
                "v_cache": ["100", "101"]
            })),
            json!({
                "direction": 1,
                "page": 2,
                "s_num": 4,
                "v_cache": ["100", "101"]
            })
        );
        // 宿主从协议层传数字数组也收成字符串（参考的 v_cache 是 list[str]）。
        assert_eq!(
            home_feed_params(&json!({"v_cache": [100, 101]}))["v_cache"],
            json!(["100", "101"])
        );
    }

    #[test]
    fn a_home_feed_page_maps_every_field_the_reference_model_names() {
        let payload = home_feed_payload(&raw_home_feed());
        let response: RecommendFeedCardResponse =
            serde_json::from_value(payload.clone()).expect("解析首页信息流");
        assert_eq!(response.retcode, Some(0));
        assert_eq!(response.prompt.as_deref(), Some("为你推荐"));
        assert_eq!(response.d_num, Some(3), "参考字段 d_num 输出成 dNum");
        assert_eq!(
            response.load_mark,
            Some(1),
            "参考字段 load_mark 输出成 loadMark"
        );
        let shelves = response.shelves.as_ref().expect("有楼层");
        assert_eq!(shelves.len(), 1);
        assert_eq!(shelves[0].id, Some(100));
        assert_eq!(shelves[0].title_template.as_deref(), Some("晚间推荐"));
        assert_eq!(shelves[0].title_content.as_deref(), Some("晚间推荐 · 华语"));
        // `more` 参考声明为 dict，透传后展开仍是对象。
        let more: Value =
            serde_json::from_str(shelves[0].more.as_deref().expect("有 more")).unwrap();
        assert_eq!(more["type"], 1);
        assert_eq!(more["title"], "查看全部");
        let niches = shelves[0].niches.as_ref().expect("有细分分组");
        assert_eq!(niches.len(), 1);
        assert_eq!(niches[0].id, Some(1));
        assert_eq!(niches[0].title_content.as_deref(), Some("华语精选"));
        // `cards` 参考声明为 list[dict]，透传后展开仍是数组。
        let cards: Value =
            serde_json::from_str(niches[0].cards.as_deref().expect("有卡片")).unwrap();
        assert_eq!(cards[0]["id"], 11);

        // 宿主按 camelCase 读，嵌套对象/数组的形状不变。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["dNum"], 3);
        assert_eq!(round["loadMark"], 1);
        assert_eq!(round["shelves"][0]["titleTemplate"], "晚间推荐");
        assert_eq!(round["shelves"][0]["more"]["id"], 200);
        assert_eq!(round["shelves"][0]["niches"][0]["cards"][0]["type"], 1);
    }

    #[test]
    fn radar_params_writes_the_two_empty_lists_the_reference_sends() {
        assert_eq!(
            radar_params(&json!({})),
            json!({"Page": 1, "ReqType": 0, "FavSongs": [], "EntranceSongs": []})
        );
        assert_eq!(
            radar_params(&json!({"page": 3})),
            json!({"Page": 3, "ReqType": 0, "FavSongs": [], "EntranceSongs": []})
        );
    }

    #[test]
    fn a_radar_page_unwraps_each_track_and_keeps_the_paging_flags() {
        let payload = radar_payload(&json!({
            "VecSongs": [{
                "Track": {
                    "id": 123,
                    "mid": "0039MnYb0qxYhV",
                    "name": "歌名",
                    "interval": 260,
                    "singer": [{"mid": "0025NhlN2yWrP4", "name": "某某"}],
                    "album": {"id": 456, "mid": "002fRO0N4FftzY", "name": "专辑"},
                    "pay": {"pay_play": 1}
                }
            }],
            "RecommendSongIds": [123, 456],
            "BaseSongIds": ["789"],
            "HasMore": true,
            "Toast": "",
            "TimeStamp": 1760000000,
            "VideoCards": {"cards": []}
        }));
        let response: RadarRecommendResponse =
            serde_json::from_value(payload).expect("解析雷达推荐");
        let songs = response.songs.as_ref().expect("有歌曲");
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].song_id, Some(123));
        assert_eq!(songs[0].song_mid, "0039MnYb0qxYhV");
        assert_eq!(songs[0].title, "歌名");
        assert_eq!(songs[0].artist, "某某");
        assert_eq!(songs[0].album_id, Some(456));
        assert_eq!(songs[0].duration, Some(260));
        assert_eq!(songs[0].pay_play, Some(1));
        assert_eq!(
            response.recommend_song_ids.as_deref(),
            Some(&[123, 456][..])
        );
        assert_eq!(
            response.base_song_ids.as_deref(),
            Some(&[789][..]),
            "上游给数字字符串也收成整数"
        );
        assert_eq!(response.has_more, Some(true));
        assert_eq!(response.timestamp, Some(1760000000));
        // `VideoCards` 参考声明为 dict，透传后展开仍是对象。
        let cards: Value =
            serde_json::from_str(response.video_cards.as_deref().expect("有视频卡片")).unwrap();
        assert_eq!(cards["cards"], json!([]));

        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["recommendSongIds"][0], 123);
        assert_eq!(round["hasMore"], true);
        assert_eq!(round["videoCards"]["cards"], json!([]));
        assert_eq!(round["songs"][0]["songMid"], "0039MnYb0qxYhV");
    }

    #[test]
    fn a_radar_has_more_of_one_is_still_true() {
        // 上游有时把布尔写成 1（pydantic 会收），翻页判据不能因此失灵。
        let payload = radar_payload(&json!({"VecSongs": [], "HasMore": 1}));
        let response: RadarRecommendResponse = serde_json::from_value(payload).expect("解析");
        assert_eq!(response.has_more, Some(true));
    }

    #[test]
    fn recommend_songlist_params_turn_page_into_the_reference_offset() {
        assert_eq!(
            recommend_songlist_params(&json!({})),
            json!({"From": 0, "Size": 25}),
            "参考缺省：第 1 页、25 条"
        );
        assert_eq!(
            recommend_songlist_params(&json!({"page": 3, "num": 25})),
            json!({"From": 50, "Size": 25}),
            "参考的 CursorStrategy：From = num * (page - 1)"
        );
        assert_eq!(
            recommend_songlist_params(&json!({"page": 2, "num": 10})),
            json!({"From": 10, "Size": 10})
        );
    }

    #[test]
    fn a_recommend_songlist_page_maps_the_reference_aliases_and_jsonpaths() {
        let payload = recommend_songlist_payload(&json!({
            "List": [{
                "Playlist": {
                    "basic": {
                        "tid": 7011264340_i64,
                        "dirId": 0,
                        "title": "私人订制推荐",
                        "desc": "根据你的口味",
                        "cover": {"default_url": "http://y.gtimg.cn/music/photo_new/T002R300x300M000002fRO0N4FftzY.jpg"},
                        "creator": {"nick": "小Q"},
                        "song_cnt": 30,
                        "play_cnt": 12000
                    }
                }
            }],
            "HasMore": 1,
            "FromLimit": 25,
            "Msg": "ok"
        }));
        let response: RecommendSonglistResponse =
            serde_json::from_value(payload).expect("解析推荐歌单");
        let songlists = response.songlists.as_ref().expect("有歌单");
        assert_eq!(songlists.len(), 1);
        assert_eq!(
            songlists[0].id,
            Some(7011264340_i64),
            "参考的 id 别名含 tid"
        );
        assert_eq!(songlists[0].dirid, Some(0));
        assert_eq!(songlists[0].title.as_deref(), Some("私人订制推荐"));
        assert_eq!(songlists[0].desc.as_deref(), Some("根据你的口味"));
        assert_eq!(songlists[0].songnum, Some(30), "参考别名 song_cnt");
        assert_eq!(songlists[0].listennum, Some(12000), "参考别名 play_cnt");
        assert_eq!(songlists[0].creator_nick.as_deref(), Some("小Q"));
        assert_eq!(
            songlists[0].picurl.as_deref(),
            Some("https://y.gtimg.cn/music/photo_new/T002R300x300M000002fRO0N4FftzY.jpg"),
            "参考的 jsonpath $.cover.default_url，且封面一律转 https"
        );
        assert_eq!(response.has_more, Some(true));
        assert_eq!(response.from_limit, Some(25), "下一页的 From 就是它");
        assert_eq!(response.msg.as_deref(), Some("ok"));

        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["songlists"][0]["creatorNick"], "小Q");
        assert_eq!(round["songlists"][0]["songnum"], 30);
        assert_eq!(round["fromLimit"], 25);
        assert_eq!(round["hasMore"], true);
    }

    #[test]
    fn an_empty_list_is_an_answer_not_a_failure() {
        // 三个接口都不是 require_login 的整表读取：空是合法答案，照常解析。
        let response: RecommendFeedCardResponse =
            serde_json::from_value(home_feed_payload(&json!({"v_shelf": []})))
                .expect("空楼层照常解析");
        assert!(response.shelves.expect("有 shelves").is_empty());

        let response: RadarRecommendResponse =
            serde_json::from_value(radar_payload(&json!({"VecSongs": []})))
                .expect("空歌曲照常解析");
        assert!(response.songs.expect("有 songs").is_empty());

        let response: RecommendSonglistResponse =
            serde_json::from_value(recommend_songlist_payload(&json!({"List": []})))
                .expect("空歌单照常解析");
        assert!(response.songlists.expect("有 songlists").is_empty());
    }

    #[test]
    fn a_missing_list_key_leaves_the_list_absent_not_empty() {
        // 上游没给列表键与「给了空列表」是两件事：前者是形状不对，字段保持 None，
        // 宿主可以据此判断这次不是「没有推荐」而是响应不成形。
        let response: RecommendFeedCardResponse =
            serde_json::from_value(home_feed_payload(&json!({"retcode": 0})))
                .expect("只有 retcode 也能解析");
        assert!(response.shelves.is_none());

        let response: RadarRecommendResponse =
            serde_json::from_value(radar_payload(&json!({"HasMore": false})))
                .expect("只有 hasMore 也能解析");
        assert!(response.songs.is_none());
        assert_eq!(response.has_more, Some(false));

        let response: RecommendSonglistResponse =
            serde_json::from_value(recommend_songlist_payload(&json!({"FromLimit": 0})))
                .expect("只有游标也能解析");
        assert!(response.songlists.is_none());
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(
            METHODS,
            &[
                "fetch_home_feed",
                "fetch_radar_recommend",
                "fetch_recommend_playlists"
            ]
        );
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_recommend_feed",
            &json!({})
        )
        .is_none());
    }
}

//! 评论：数量、热评、新评、推荐评、时刻评论，以及发/删评论。
//!
//! 当前公开契约见 `docs/endpoints.md`，解析与兼容规则见 `docs/parsing.md`。
//!
//! 业务类型（参考 `CommentBizType`）：1=歌曲 2=专辑 3=歌单 4=MV 15=长音频；
//! `bizId` 一律按数字给出，到协议上再转成字符串。
//!
//! 空值判据：评论列表的**空是答案**（这首歌就是还没有评论），照常返回，
//! 不当故障——参考里读取也没有 `require_login`；只有发/删评论需要登录。

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
    "fetch_comment_count",
    "fetch_hot_comments",
    "fetch_new_comments",
    "fetch_recommend_comments",
    "fetch_moment_comments",
    "add_comment",
    "delete_comment",
];

/// 歌曲类型的业务 id（参考 `CommentBizType.SONG`）。
const BIZ_TYPE_SONG: i64 = 1;
/// 评论列表的默认页大小（参考的 `page_size: int = 15`）。
const PAGE_SIZE: i64 = 15;

// MARK: - 模型

/// 参考 `IconTextInfo`：评论数量接口附带的角标文案。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommentIcon {
    /// 角标展示文案.
    pub txt: Option<String>,
    /// 角标唯一标识.
    pub unique_id: Option<String>,
    /// 角标类型（参考的字段就叫 `type`，Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// 关联评论 ID.
    pub cmid: Option<String>,
    /// 是否为动态角标.
    pub is_dynamic: Option<bool>,
}

/// 参考 `CommentCountResponse`：评论数量接口的统计结果.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommentCount {
    pub biz_type: Option<i64>,
    pub biz_id: Option<String>,
    pub biz_sub_type: Option<i64>,
    /// 评论总数.
    pub count: Option<i64>,
    /// 计数字段版本.
    pub count_ver: Option<String>,
    /// 面向展示的计数文案.
    pub count_view: Option<String>,
    pub related_id: Option<String>,
    /// 附加提示文案.
    pub tip: Option<String>,
    pub icon_list: Option<Vec<CommentIcon>>,
    /// 评论标签页类型（在 `response` 的兄弟键 `cmTabType` 上）.
    pub cm_tab_type: Option<i64>,
}

/// 参考 `CommentItem`：标准评论列表里的单条评论.
///
/// `song_ts_elems` / `hash_tag_list` / `little_tails` / `icon_list` /
/// `vip_ui` / `sub_comments` 在参考里是 `list[dict]` / `dict`（原样透传的
/// 上游形状），这里存 JSON 文本：`#[data]` 认不出 `serde_json::Value`，
/// 而经 serde 进出时它们仍是真正的嵌套对象/数组（见 `raw_json_serialize`）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub cmid: Option<String>,
    pub seq_no: Option<String>,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub encrypt_uin: Option<String>,
    pub content: Option<String>,
    pub pub_time: Option<i64>,
    pub praise_num: Option<i64>,
    pub reply_cnt: Option<i64>,
    pub is_praised: Option<i64>,
    pub is_self: Option<i64>,
    pub state: Option<i64>,
    pub hot_score: Option<String>,
    pub rec_score: Option<String>,
    pub song_id: Option<i64>,
    pub song_name: Option<String>,
    pub singer_names: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub song_ts_elems: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub hash_tag_list: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub little_tails: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub icon_list: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub vip_ui: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub sub_comments: Option<String>,
}

/// 参考 `CommentListResponse`：常规评论列表接口的响应体（一页）.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommentList {
    pub comments: Option<Vec<Comment>>,
    pub comment_ids: Option<Vec<String>>,
    /// 是否还有更多结果（0/1）.
    pub has_more: Option<i64>,
    pub next_offset: Option<i64>,
    /// 评论总数.
    pub total: Option<i64>,
    pub total_cm_num: Option<i64>,
    pub comment_tip: Option<String>,
    pub comment_h5_page: Option<String>,
    pub has_ts_cm: Option<i64>,
    pub share_cnt: Option<i64>,
    pub msg: Option<String>,
    pub sub_code: Option<i64>,
}

/// 参考 `MomentCommentItem`：时刻评论流里的单条评论.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MomentComment {
    pub cmid: Option<String>,
    pub seq_no: Option<String>,
    pub content: Option<String>,
    pub encrypt_uin: Option<String>,
    pub pub_time: Option<i64>,
    pub praise_num: Option<i64>,
    pub reply_cnt: Option<i64>,
    pub state: Option<i64>,
    pub is_self: Option<i64>,
    pub location: Option<String>,
    pub phone_type: Option<String>,
    pub pic: Option<String>,
    pub pic_size: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub song_ts_elems: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub hash_tag_list: Option<String>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub little_tails: Option<String>,
}

/// 参考 `MomentCommentResponse`：时刻评论列表接口的响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MomentCommentList {
    pub comments: Option<Vec<MomentComment>>,
    /// 是否还有更多结果（0/1）.
    pub has_more: Option<i64>,
    /// 下一页游标（回填到请求的 `LastPos`）.
    pub next_pos: Option<String>,
    pub hint: Option<String>,
    pub prev_list_loaded: Option<i64>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub map_cm_ext: Option<String>,
}

/// 参考 `AddCommentResponse`：发评论的响应体.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AddedComment {
    pub subcode: Option<i64>,
    pub msg: Option<String>,
    /// 新增评论 ID（参考字段名是 `id`）.
    pub id: Option<String>,
    /// 父评论 ID.
    pub parent: Option<String>,
    /// 楼层号（在 `Floor.Num` 上）.
    pub floor: Option<i64>,
    /// 验证码 URL（如果需要）.
    pub verify_url: Option<String>,
}

// MARK: - 参考模型里 `dict` 形状的字段

/// 把 Rusk 侧存的 JSON 文本按真 JSON 写出（宿主在 JSON 协议上看到的是对象/数组）。
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

/// 宽松的布尔：上游可能给 `true`、`1` 或 `"1"`。
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

/// 单条评论（参考 `CommentItem`）。
fn comment_payload(raw: &Value) -> Value {
    json!({
        "cmid": first_text(raw, &["CmId", "cmid"]),
        "seqNo": first_text(raw, &["SeqNo", "seqNo"]),
        "nick": first_text(raw, &["Nick", "nick"]),
        "avatar": first_text(raw, &["Avatar", "avatar"]),
        "encryptUin": first_text(raw, &["EncryptUin", "encryptUin"]),
        "content": first_text(raw, &["Content", "content"]),
        "pubTime": first_int(raw, &["PubTime", "pubTime"]),
        "praiseNum": first_int(raw, &["PraiseNum", "praiseNum"]),
        "replyCnt": first_int(raw, &["ReplyCnt", "replyCnt"]),
        "isPraised": first_int(raw, &["IsPraised", "isPraised"]),
        "isSelf": first_int(raw, &["IsSelf", "isSelf"]),
        "state": first_int(raw, &["State", "state"]),
        "hotScore": first_text(raw, &["HotScore", "hotScore"]),
        "recScore": first_text(raw, &["RecScore", "recScore"]),
        "songId": first_int(raw, &["SongId", "songId"]),
        "songName": first_text(raw, &["SongName", "songName"]),
        "singerNames": first_text(raw, &["SingerNames", "singerNames"]),
        "songTsElems": raw_field(raw, &["SongTsElems", "songTsElems"]),
        "hashTagList": raw_field(raw, &["HashTagList", "hashTagList"]),
        "littleTails": raw_field(raw, &["LittleTails", "littleTails"]),
        "iconList": raw_field(raw, &["IconList", "iconList"]),
        "vipUi": raw_field(raw, &["VipUI", "vipUi", "VipUi"]),
        "subComments": raw_field(raw, &["SubComments", "subComments"]),
    })
}

/// 常规评论列表的一页（参考 `CommentListResponse`）。
fn comment_list_payload(data: &Value) -> Value {
    // 列表与它的分页字段在 `CommentList` 里，`Msg`/`SubCode`/`TotalCmNum`
    // 在它的兄弟上（参考模型的 jsonpath 就是这么分的）。
    let list = first_object(data, &["CommentList", "commentList"]).unwrap_or(data);
    json!({
        "comments": first_array(list, &["Comments", "comments"])
            .map(|items| items.iter().map(comment_payload).collect::<Vec<_>>()),
        "commentIds": string_list(list, &["CommentIds", "commentIds"]),
        "hasMore": first_int(list, &["HasMore", "hasMore"]),
        "nextOffset": first_int(list, &["NextOffset", "nextOffset"]),
        "total": first_int(list, &["Total", "total"]),
        "totalCmNum": first_int(data, &["TotalCmNum", "totalCmNum"]),
        "commentTip": first_text(data, &["CommentTip", "commentTip"]),
        "commentH5Page": first_text(data, &["CommentH5Page", "commentH5Page"]),
        "hasTsCm": first_int(data, &["HasTsCm", "hasTsCm"]),
        "shareCnt": first_int(data, &["ShareCnt", "shareCnt"]),
        "msg": first_text(data, &["Msg", "msg"]),
        "subCode": first_int(data, &["SubCode", "subCode"]),
    })
}

/// 单条时刻评论（参考 `MomentCommentItem`）。
fn moment_payload(raw: &Value) -> Value {
    json!({
        "cmid": first_text(raw, &["CmId", "cmid"]),
        "seqNo": first_text(raw, &["SeqNo", "seqNo"]),
        "content": first_text(raw, &["Content", "content"]),
        "encryptUin": first_text(raw, &["EncryptUin", "encryptUin"]),
        "pubTime": first_int(raw, &["PubTime", "pubTime"]),
        "praiseNum": first_int(raw, &["PraiseNum", "praiseNum"]),
        "replyCnt": first_int(raw, &["ReplyCnt", "replyCnt"]),
        "state": first_int(raw, &["State", "state"]),
        "isSelf": first_int(raw, &["IsSelf", "isSelf"]),
        "location": first_text(raw, &["Location", "location"]),
        "phoneType": first_text(raw, &["PhoneType", "phoneType"]),
        "pic": first_text(raw, &["Pic", "pic"]),
        "picSize": first_text(raw, &["PicSize", "picSize"]),
        "songTsElems": raw_field(raw, &["SongTsElems", "songTsElems"]),
        "hashTagList": raw_field(raw, &["HashTagList", "hashTagList"]),
        "littleTails": raw_field(raw, &["LittleTails", "littleTails"]),
    })
}

/// 时刻评论的一页（参考 `MomentCommentResponse`）。
fn moment_list_payload(data: &Value) -> Value {
    json!({
        "comments": first_array(data, &["CmList", "cmList"])
            .map(|items| items.iter().map(moment_payload).collect::<Vec<_>>()),
        "hasMore": first_int(data, &["HasMore", "hasMore"]),
        "nextPos": first_text(data, &["NextPos", "nextPos"]),
        "hint": first_text(data, &["Hint", "hint"]),
        "prevListLoaded": first_int(data, &["PrevListLoaded", "prevListLoaded"]),
        "mapCmExt": raw_field(data, &["MapCmExt", "mapCmExt"]),
    })
}

/// 评论数量（参考 `CommentCountResponse`）。
fn count_payload(data: &Value) -> Value {
    // 计数字段在 `response` 里，`cmTabType` 在它的兄弟上。
    let response = first_object(data, &["response", "Response"]).unwrap_or(data);
    json!({
        "bizType": first_int(response, &["biz_type", "bizType"]),
        "bizId": first_text(response, &["biz_id", "bizId"]),
        "bizSubType": first_int(response, &["biz_sub_type", "bizSubType"]),
        "count": first_int(response, &["count"]),
        "countVer": first_text(response, &["count_ver", "countVer"]),
        "countView": first_text(response, &["count_view", "countView"]),
        "relatedId": first_text(response, &["related_id", "relatedId"]),
        "tip": first_text(response, &["tip"]),
        "iconList": first_array(response, &["icon_list", "iconList"])
            .map(|items| items.iter().map(icon_payload).collect::<Vec<_>>()),
        "cmTabType": first_int(data, &["cmTabType", "cm_tab_type"]),
    })
}

/// 评论数量接口的角标（参考 `IconTextInfo`）。
fn icon_payload(raw: &Value) -> Value {
    json!({
        "txt": first_text(raw, &["txt", "Txt"]),
        "uniqueId": first_text(raw, &["unique_id", "uniqueId"]),
        "type": first_int(raw, &["type", "Type"]),
        "cmid": first_text(raw, &["cmid", "CmId"]),
        "isDynamic": bool_field(raw, &["is_dynamic", "isDynamic"]),
    })
}

/// 发表评论的结果（参考 `AddCommentResponse`）。
fn added_payload(data: &Value) -> Value {
    json!({
        "subcode": first_int(data, &["SubCode", "subcode"]),
        "msg": first_text(data, &["Msg", "msg"]),
        "id": first_text(data, &["AddedCmId", "addedCmId"]),
        "parent": first_text(data, &["ParentCmId", "parentCmId"]),
        "floor": first_object(data, &["Floor", "floor"])
            .and_then(|floor| first_int(floor, &["Num", "num"])),
        "verifyUrl": first_text(data, &["VerifyUrl", "verifyUrl"]),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 三个业务的公共身份：`bizId` 是数字，类型缺省是歌曲。
fn business_of(params: &Value) -> Result<(i64, i64, Option<i64>), UpstreamError> {
    let biz_id = first_int(params, &["bizId", "biz_id"])
        .filter(|id| *id > 0)
        .ok_or_else(|| UpstreamError::Upstream("缺少 bizId".into()))?;
    let biz_type = first_int(params, &["bizType", "biz_type"]).unwrap_or(BIZ_TYPE_SONG);
    let biz_sub_type = first_int(params, &["bizSubType", "biz_sub_type"]);
    Ok((biz_id, biz_type, biz_sub_type))
}

/// 评论数量接口的 `request` 块；参考只在歌曲类型上补默认子类型 2。
fn count_request(biz_id: i64, biz_type: i64, biz_sub_type: Option<i64>) -> Value {
    let biz_sub_type = biz_sub_type.or_else(|| (biz_type == BIZ_TYPE_SONG).then_some(2));
    let mut request = json!({
        "biz_id": biz_id.to_string(),
        "biz_type": biz_type,
    });
    if let Some(biz_sub_type) = biz_sub_type {
        request["biz_sub_type"] = json!(biz_sub_type);
    }
    request
}

/// 列表接口的翻页参数：`page` 与 `page_size` 都照参考的缺省（1 / 15），
/// **不做区间钳制**——参考把调用方给的值原样传给上游，`PageNum` 也因此
/// 就是 `page - 1`（哪怕调用方给 0）。
fn page_params(params: &Value) -> (i64, i64) {
    let page = first_int(params, &["page", "pageNum"]).unwrap_or(1);
    let page_size = first_int(params, &["limit", "pageSize"]).unwrap_or(PAGE_SIZE);
    (page, page_size)
}

/// 上一页最后一条评论的序号。
fn last_seq_no(params: &Value) -> String {
    first_text(params, &["lastCommentSeqNo"]).unwrap_or_default()
}

/// 给参数补可选的 `BizSubType`（参考在四个列表接口上都是「给了才带」）。
fn with_sub_type(mut param: Value, biz_sub_type: Option<i64>) -> Value {
    if let Some(biz_sub_type) = biz_sub_type {
        param["BizSubType"] = json!(biz_sub_type);
    }
    param
}

/// 热评参数（参考 `get_hot_comments`）。
///
/// `PageNum` 就是 `page - 1`（第一页传 0）——参考里如此，不要「修正」。
fn hot_params(
    biz_id: i64,
    biz_type: i64,
    biz_sub_type: Option<i64>,
    page: i64,
    page_size: i64,
    last_comment_seq_no: &str,
) -> Value {
    with_sub_type(
        json!({
            "BizType": biz_type,
            "BizId": biz_id.to_string(),
            "LastCommentSeqNo": last_comment_seq_no,
            "PageSize": page_size,
            "PageNum": page - 1,
            "HotType": 1,
            "WithAirborne": 0,
            "PicEnable": 1,
        }),
        biz_sub_type,
    )
}

/// 新评参数（参考 `get_new_comments`）。
fn new_params(
    biz_id: i64,
    biz_type: i64,
    biz_sub_type: Option<i64>,
    page: i64,
    page_size: i64,
    last_comment_seq_no: &str,
) -> Value {
    with_sub_type(
        json!({
            "PageSize": page_size,
            "PageNum": page - 1,
            "HashTagID": "",
            "BizType": biz_type,
            "PicEnable": 1,
            "LastCommentSeqNo": last_comment_seq_no,
            "SelfSeeEnable": 1,
            "BizId": biz_id.to_string(),
            "AudioEnable": 1,
        }),
        biz_sub_type,
    )
}

/// 推荐评参数（参考 `get_recommend_comments`）。
fn recommend_params(
    biz_id: i64,
    biz_type: i64,
    biz_sub_type: Option<i64>,
    page: i64,
    page_size: i64,
    last_comment_seq_no: &str,
) -> Value {
    with_sub_type(
        json!({
            "PageSize": page_size,
            "PageNum": page - 1,
            "BizType": biz_type,
            "PicEnable": 1,
            "Flag": 1,
            "LastCommentSeqNo": last_comment_seq_no,
            "CmListUIVer": 1,
            "BizId": biz_id.to_string(),
            "AudioEnable": 1,
        }),
        biz_sub_type,
    )
}

/// 时刻评论参数（参考 `get_moment_comments`）：游标是 `LastPos`，没有页码。
fn moment_params(
    biz_id: i64,
    biz_type: i64,
    biz_sub_type: Option<i64>,
    page_size: i64,
    last_pos: &str,
) -> Value {
    with_sub_type(
        json!({
            "LastPos": last_pos,
            "HashTagID": "",
            "SeekTs": -1,
            "Size": page_size,
            "BizType": biz_type,
            "BizId": biz_id.to_string(),
        }),
        biz_sub_type,
    )
}

/// 发评论参数（参考 `add_comment`）。
fn add_params(
    biz_id: i64,
    biz_type: i64,
    biz_sub_type: Option<i64>,
    content: &str,
    reply_cmt_id: Option<&str>,
) -> Value {
    let mut param = json!({
        "Content": content,
        "BizType": biz_type,
        "BizId": biz_id.to_string(),
    });
    if let Some(reply_cmt_id) = reply_cmt_id {
        param["RepliedCmId"] = json!(reply_cmt_id);
    }
    if let Some(biz_sub_type) = biz_sub_type {
        param["BizSubType"] = json!(biz_sub_type);
    }
    param
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
        "fetch_comment_count" => comment_count(upstream, credential, platform, params),
        "fetch_hot_comments" => hot_comments(upstream, credential, platform, params),
        "fetch_new_comments" => new_comments(upstream, credential, platform, params),
        "fetch_recommend_comments" => recommend_comments(upstream, credential, platform, params),
        "fetch_moment_comments" => moment_comments(upstream, credential, platform, params),
        "add_comment" => post_comment(upstream, credential, platform, params),
        "delete_comment" => remove_comment(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 写接口的门槛：参考的 `require_login=True` 就是「凭据可用」。
fn require_login(credential: &Credential) -> Result<(), UpstreamError> {
    if credential.is_usable() {
        Ok(())
    } else {
        Err(UpstreamError::Upstream("需要登录后才能读取".into()))
    }
}

/// 评论数量（`music.globalComment.CommentCountSrv / GetCmCount`）。
fn comment_count(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let (biz_id, biz_type, biz_sub_type) = business_of(params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.globalComment.CommentCountSrv",
            method: "GetCmCount",
            param: json!({ "request": count_request(biz_id, biz_type, biz_sub_type) }),
        },
    )?;
    Ok(count_payload(&data))
}

/// 热评（`music.globalComment.CommentRead / GetHotCommentList`）。
fn hot_comments(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let (biz_id, biz_type, biz_sub_type) = business_of(params)?;
    let (page, page_size) = page_params(params);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.globalComment.CommentRead",
            method: "GetHotCommentList",
            param: hot_params(
                biz_id,
                biz_type,
                biz_sub_type,
                page,
                page_size,
                &last_seq_no(params),
            ),
        },
    )?;
    Ok(comment_list_payload(&data))
}

/// 最新评论（`music.globalComment.CommentRead / GetNewCommentList`）。
///
/// 空列表是答案：这首歌就是还没有评论，照常返回。
fn new_comments(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let (biz_id, biz_type, biz_sub_type) = business_of(params)?;
    let (page, page_size) = page_params(params);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.globalComment.CommentRead",
            method: "GetNewCommentList",
            param: new_params(
                biz_id,
                biz_type,
                biz_sub_type,
                page,
                page_size,
                &last_seq_no(params),
            ),
        },
    )?;
    Ok(comment_list_payload(&data))
}

/// 推荐评论（`music.globalComment.CommentRead / GetRecCommentList`）。
fn recommend_comments(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let (biz_id, biz_type, biz_sub_type) = business_of(params)?;
    let (page, page_size) = page_params(params);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.globalComment.CommentRead",
            method: "GetRecCommentList",
            param: recommend_params(
                biz_id,
                biz_type,
                biz_sub_type,
                page,
                page_size,
                &last_seq_no(params),
            ),
        },
    )?;
    Ok(comment_list_payload(&data))
}

/// 时刻评论（`music.globalComment.SongTsComment / GetSongTsCmList`）。
///
/// 游标读法：把回值里的 `nextPos` 回填到 `lastCommentSeqNo` 再请求。
fn moment_comments(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let (biz_id, biz_type, biz_sub_type) = business_of(params)?;
    let (_, page_size) = page_params(params);
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.globalComment.SongTsComment",
            method: "GetSongTsCmList",
            param: moment_params(
                biz_id,
                biz_type,
                biz_sub_type,
                page_size,
                &last_seq_no(params),
            ),
        },
    )?;
    Ok(moment_list_payload(&data))
}

/// 发表评论（`music.globalComment.CommentWriteServer / AddComment`），需要登录。
fn post_comment(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let (biz_id, biz_type, biz_sub_type) = business_of(params)?;
    let content = first_text(params, &["content"]).unwrap_or_default();
    if content.trim().is_empty() {
        return Err(UpstreamError::Upstream("缺少评论内容".into()));
    }
    let reply_cmt_id = first_text(params, &["replyCmtId", "reply_cmt_id"]);
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: "music.globalComment.CommentWriteServer",
            method: "AddComment",
            param: add_params(
                biz_id,
                biz_type,
                biz_sub_type,
                &content,
                reply_cmt_id.as_deref(),
            ),
        },
    )?;
    Ok(added_payload(&data))
}

/// 删除评论（`music.globalComment.CommentWriteServer / DelComment`），需要登录。
///
/// 参考的判据是 `SubCode == 0`：为 0 即成功，**评论不存在也算成功**。
fn remove_comment(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let cm_id = first_text(params, &["cmId", "cm_id"]).unwrap_or_default();
    if cm_id.trim().is_empty() {
        return Err(UpstreamError::Upstream("缺少评论 ID".into()));
    }
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: "music.globalComment.CommentWriteServer",
            method: "DelComment",
            param: json!({ "CommentId": cm_id }),
        },
    )?;
    Ok(json!({
        // Reference defaults a missing SubCode to 0. A present malformed value
        // must not become success; some real successful replies are {}.
        "deleted": if data.get("SubCode").or_else(|| data.get("subCode")).is_none() {
            true
        } else {
            first_int(&data, &["SubCode", "subCode"]) == Some(0)
        },
    }))
}

// MARK: - 宿主包装

/// 包装层共用的业务参数：`bizId` 是数字，类型按缺省走。
fn business_params(biz_id: i64, biz_type: Option<i64>, biz_sub_type: Option<i64>) -> Value {
    let mut params = json!({ "bizId": biz_id });
    if let Some(biz_type) = biz_type {
        params["bizType"] = json!(biz_type);
    }
    if let Some(biz_sub_type) = biz_sub_type {
        params["bizSubType"] = json!(biz_sub_type);
    }
    params
}

/// 列表包装共用：三个业务参数 + 翻页 + 游标。
fn list_params(
    biz_id: i64,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
    last_comment_seq_no: Option<String>,
) -> Value {
    let mut params = business_params(biz_id, biz_type, biz_sub_type);
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(page_size) = page_size {
        params["limit"] = json!(page_size);
    }
    if let Some(last) = last_comment_seq_no {
        params["lastCommentSeqNo"] = json!(last);
    }
    params
}

/// 评论数量。`biz_type`: 1=歌曲 2=专辑 3=歌单 4=MV 15=长音频。
#[export]
pub fn fetch_comment_count(
    biz_id: i64,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
) -> Result<CommentCount, crate::HelperError> {
    crate::port::call(
        "fetch_comment_count",
        business_params(biz_id, biz_type, biz_sub_type),
    )
}

/// 热评一页；`hasMore` 为 1 时把这一页最后一条的 `seqNo` 回填给下一页。
#[export]
pub fn fetch_hot_comments(
    biz_id: i64,
    page: Option<i64>,
    page_size: Option<i64>,
    last_comment_seq_no: Option<String>,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
) -> Result<CommentList, crate::HelperError> {
    crate::port::call(
        "fetch_hot_comments",
        list_params(
            biz_id,
            biz_type,
            biz_sub_type,
            page,
            page_size,
            last_comment_seq_no,
        ),
    )
}

/// 最新评论一页。
#[export]
pub fn fetch_new_comments(
    biz_id: i64,
    page: Option<i64>,
    page_size: Option<i64>,
    last_comment_seq_no: Option<String>,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
) -> Result<CommentList, crate::HelperError> {
    crate::port::call(
        "fetch_new_comments",
        list_params(
            biz_id,
            biz_type,
            biz_sub_type,
            page,
            page_size,
            last_comment_seq_no,
        ),
    )
}

/// 推荐评论一页。
#[export]
pub fn fetch_recommend_comments(
    biz_id: i64,
    page: Option<i64>,
    page_size: Option<i64>,
    last_comment_seq_no: Option<String>,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
) -> Result<CommentList, crate::HelperError> {
    crate::port::call(
        "fetch_recommend_comments",
        list_params(
            biz_id,
            biz_type,
            biz_sub_type,
            page,
            page_size,
            last_comment_seq_no,
        ),
    )
}

/// 时刻评论一页；游标回填 `last_comment_seq_no`（取自 `nextPos`）。
#[export]
pub fn fetch_moment_comments(
    biz_id: i64,
    page_size: Option<i64>,
    last_comment_seq_no: Option<String>,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
) -> Result<MomentCommentList, crate::HelperError> {
    let mut params = business_params(biz_id, biz_type, biz_sub_type);
    if let Some(page_size) = page_size {
        params["limit"] = json!(page_size);
    }
    if let Some(last) = last_comment_seq_no {
        params["lastCommentSeqNo"] = json!(last);
    }
    crate::port::call("fetch_moment_comments", params)
}

/// 发表评论；`reply_cmt_id` 给了就是回复那条评论。
#[export]
pub fn add_comment(
    biz_id: i64,
    content: String,
    reply_cmt_id: Option<String>,
    biz_type: Option<i64>,
    biz_sub_type: Option<i64>,
) -> Result<AddedComment, crate::HelperError> {
    let mut params = business_params(biz_id, biz_type, biz_sub_type);
    params["content"] = json!(content);
    if let Some(reply_cmt_id) = reply_cmt_id {
        params["replyCmtId"] = json!(reply_cmt_id);
    }
    crate::port::call("add_comment", params)
}

/// 删除评论。返回是否删除成功：评论不存在也是 `true`（参考的行为）。
#[export]
pub fn delete_comment(cm_id: String) -> Result<bool, crate::HelperError> {
    let value: Value = crate::port::call("delete_comment", json!({ "cmId": cm_id }))?;
    Ok(value
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一条上游评论 + 它所在的一页，字段名都是上游自己的拼写。
    fn raw_comment_list() -> Value {
        json!({
            "CommentList": {
                "Comments": [{
                    "CmId": "123",
                    "SeqNo": "9",
                    "Nick": "甲",
                    "Avatar": "https://y.gtimg.cn/a.jpg",
                    "EncryptUin": "euin",
                    "Content": "好听",
                    "PubTime": 1_700_000_000,
                    "PraiseNum": 12,
                    "ReplyCnt": 3,
                    "IsPraised": 0,
                    "IsSelf": 1,
                    "State": 0,
                    "HotScore": "9.9",
                    "RecScore": "1.2",
                    "SongId": 42,
                    "SongName": "歌名",
                    "SingerNames": "甲, 乙",
                    "SongTsElems": [{"start": 1, "text": "词"}],
                    "HashTagList": [{"tag": "#话题#"}],
                    "LittleTails": [{"txt": "尾巴"}],
                    "IconList": [{"txt": "角标"}],
                    "VipUI": {"level": 3},
                    "SubComments": [{"CmId": "1"}]
                }],
                "CommentIds": ["123"],
                "HasMore": 1,
                "NextOffset": 20,
                "Total": 7
            },
            "TotalCmNum": 7,
            "CommentTip": "tip",
            "CommentH5Page": "https://y.qq.com/comment",
            "HasTsCm": 1,
            "ShareCnt": 2,
            "SubCode": 0
        })
    }

    #[test]
    fn a_comment_page_maps_every_field_the_reference_model_names() {
        let payload = comment_list_payload(&raw_comment_list());
        assert_eq!(payload["comments"][0]["cmid"], "123");
        assert_eq!(payload["comments"][0]["seqNo"], "9");
        assert_eq!(payload["comments"][0]["nick"], "甲");
        assert_eq!(payload["comments"][0]["encryptUin"], "euin");
        assert_eq!(payload["comments"][0]["pubTime"], 1_700_000_000);
        assert_eq!(payload["comments"][0]["songId"], 42);
        assert_eq!(payload["comments"][0]["singerNames"], "甲, 乙");
        assert_eq!(payload["comments"][0]["songTsElems"][0]["text"], "词");
        assert_eq!(payload["comments"][0]["vipUi"]["level"], 3);
        assert_eq!(payload["commentIds"][0], "123");
        assert_eq!(payload["hasMore"], 1);
        assert_eq!(payload["nextOffset"], 20);
        assert_eq!(payload["total"], 7);
        assert_eq!(payload["totalCmNum"], 7);
        assert_eq!(payload["subCode"], 0);

        let page: CommentList = serde_json::from_value(payload.clone()).expect("解析这一页");
        {
            let comments = page.comments.as_ref().expect("有评论");
            assert_eq!(comments.len(), 1);
            assert_eq!(comments[0].cmid.as_deref(), Some("123"));
            assert_eq!(comments[0].song_name.as_deref(), Some("歌名"));
            assert_eq!(comments[0].praise_num, Some(12));
            // 透传的 dict 字段在 Rust 侧是 JSON 文本，展开后还是原来的对象……
            let sub: Value =
                serde_json::from_str(comments[0].sub_comments.as_deref().unwrap()).unwrap();
            assert_eq!(sub[0]["CmId"], "1");
        }
        // ……而且经 serde 写出去时仍以对象/数组出现，宿主看到的形状不变。
        let round: Value = serde_json::to_value(&page).unwrap();
        assert_eq!(round["comments"][0]["vipUi"]["level"], 3);
        assert_eq!(round["comments"][0]["songTsElems"][0]["start"], 1);
    }

    #[test]
    fn the_count_response_is_read_out_of_its_response_block() {
        let payload = count_payload(&json!({
            "response": {
                "biz_type": 1,
                "biz_id": "102065756",
                "biz_sub_type": 2,
                "count": 4321,
                "count_ver": "v1",
                "count_view": "4321 条评论",
                "related_id": "r-1",
                "tip": "提示",
                "icon_list": [{
                    "txt": "热",
                    "unique_id": "u-1",
                    "type": 1,
                    "cmid": "123",
                    "is_dynamic": true
                }]
            },
            "cmTabType": 3
        }));
        let count: CommentCount = serde_json::from_value(payload).expect("解析数量");
        assert_eq!(count.biz_type, Some(1));
        assert_eq!(count.biz_id.as_deref(), Some("102065756"));
        assert_eq!(count.count, Some(4321));
        assert_eq!(count.count_view.as_deref(), Some("4321 条评论"));
        assert_eq!(count.cm_tab_type, Some(3));
        let icons = count.icon_list.expect("有角标");
        assert_eq!(icons[0].txt.as_deref(), Some("热"));
        assert_eq!(icons[0].unique_id.as_deref(), Some("u-1"));
        assert_eq!(icons[0].kind, Some(1));
        assert_eq!(icons[0].is_dynamic, Some(true));
    }

    #[test]
    fn a_moment_page_keeps_its_cursor_and_hint() {
        let payload = moment_list_payload(&json!({
            "CmList": [{
                "CmId": "9",
                "SeqNo": "1",
                "Content": "时刻",
                "EncryptUin": "e",
                "PubTime": 1_700_000_001,
                "PraiseNum": 1,
                "ReplyCnt": 0,
                "State": 0,
                "IsSelf": 0,
                "Location": "北京",
                "PhoneType": "iPhone",
                "Pic": "https://y.gtimg.cn/p.jpg",
                "PicSize": "1080x1920",
                "SongTsElems": [{"start": 2}],
                "HashTagList": [],
                "LittleTails": []
            }],
            "HasMore": 1,
            "NextPos": "next-1",
            "Hint": "hint",
            "PrevListLoaded": 1,
            "MapCmExt": {"9": {"ext": true}}
        }));
        let page: MomentCommentList = serde_json::from_value(payload).expect("解析时刻评论");
        assert_eq!(page.has_more, Some(1));
        assert_eq!(page.next_pos.as_deref(), Some("next-1"));
        assert_eq!(page.prev_list_loaded, Some(1));
        let comments = page.comments.expect("有评论");
        assert_eq!(comments[0].cmid.as_deref(), Some("9"));
        assert_eq!(comments[0].location.as_deref(), Some("北京"));
        assert_eq!(comments[0].phone_type.as_deref(), Some("iPhone"));
        let map: Value = serde_json::from_str(page.map_cm_ext.as_deref().unwrap()).unwrap();
        assert_eq!(map["9"]["ext"], true);
    }

    #[test]
    fn an_added_comment_reports_its_id_and_floor() {
        let payload = added_payload(&json!({
            "SubCode": 0,
            "Msg": "ok",
            "AddedCmId": "888",
            "ParentCmId": "123",
            "Floor": {"Num": 5},
            "VerifyUrl": ""
        }));
        let added: AddedComment = serde_json::from_value(payload).expect("解析发评论回值");
        assert_eq!(added.subcode, Some(0));
        assert_eq!(added.id.as_deref(), Some("888"));
        assert_eq!(added.parent.as_deref(), Some("123"));
        assert_eq!(added.floor, Some(5));
    }

    #[test]
    fn hot_comments_send_page_minus_one_and_the_reference_flags() {
        let param = hot_params(2314161, 1, None, 2, 10, "seq-9");
        assert_eq!(param["BizType"], 1);
        assert_eq!(param["BizId"], "2314161");
        assert_eq!(param["PageSize"], 10);
        assert_eq!(param["PageNum"], 1, "参考就是 page-1");
        assert_eq!(param["LastCommentSeqNo"], "seq-9");
        assert_eq!(param["HotType"], 1);
        assert_eq!(param["WithAirborne"], 0);
        assert_eq!(param["PicEnable"], 1);
        assert!(param.get("BizSubType").is_none(), "没给就不带");
    }

    #[test]
    fn each_list_sends_its_own_flag_set() {
        let new = new_params(7, 15, Some(113), 1, 15, "");
        assert_eq!(new["HashTagID"], "");
        assert_eq!(new["SelfSeeEnable"], 1);
        assert_eq!(new["AudioEnable"], 1);
        assert_eq!(new["PageNum"], 0);
        assert_eq!(new["BizSubType"], 113);
        assert_eq!(new["BizType"], 15);

        let recommend = recommend_params(7, 2, None, 1, 15, "");
        assert_eq!(recommend["Flag"], 1);
        assert_eq!(recommend["CmListUIVer"], 1);
        assert_eq!(recommend["PicEnable"], 1);
        assert!(recommend.get("HashTagID").is_none());

        let moment = moment_params(7, 1, None, 20, "pos-1");
        assert_eq!(moment["LastPos"], "pos-1");
        assert_eq!(moment["SeekTs"], -1);
        assert_eq!(moment["Size"], 20);
        assert_eq!(moment["HashTagID"], "");
        assert!(moment.get("PageNum").is_none(), "时刻评论是游标，不是页码");
    }

    #[test]
    fn the_count_request_wraps_the_song_sub_type_default() {
        assert_eq!(
            count_request(2314161, 1, None),
            json!({"biz_id": "2314161", "biz_type": 1, "biz_sub_type": 2})
        );
        assert_eq!(
            count_request(2314161, 15, Some(113)),
            json!({"biz_id": "2314161", "biz_type": 15, "biz_sub_type": 113})
        );
        assert_eq!(
            count_request(7, 2, None),
            json!({"biz_id": "7", "biz_type": 2}),
            "只有歌曲补默认子类型"
        );
    }

    #[test]
    fn writes_carry_their_content_and_reply_target() {
        assert_eq!(
            add_params(42, 1, None, "好听", None),
            json!({"Content": "好听", "BizType": 1, "BizId": "42"})
        );
        assert_eq!(
            add_params(42, 1, Some(2), "回复", Some("cm-1")),
            json!({
                "Content": "回复",
                "BizType": 1,
                "BizId": "42",
                "RepliedCmId": "cm-1",
                "BizSubType": 2
            })
        );
    }

    #[test]
    fn the_business_identity_defaults_to_song() {
        let (id, biz_type, sub) = business_of(&json!({"bizId": 2314161})).expect("有 bizId 就够");
        assert_eq!((id, biz_type, sub), (2314161, 1, None));
        let (id, biz_type, sub) = business_of(&json!({
            "bizId": "102065756", "bizType": 15, "bizSubType": 113
        }))
        .expect("数字字符串也行");
        assert_eq!((id, biz_type, sub), (102065756, 15, Some(113)));
        assert!(business_of(&json!({})).is_err(), "没有 bizId 要报错");
    }

    /// 写操作没有凭据时，在发出任何请求之前就应当被拒。
    #[test]
    fn writes_refuse_without_a_login_before_any_request() {
        let upstream = Upstream::new();
        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "add_comment",
            &json!({"bizId": 1, "content": "你好"}),
        )
        .expect("add_comment 是本层的方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("登录"));

        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "delete_comment",
            &json!({"cmId": "1"}),
        )
        .expect("delete_comment 是本层的方法")
        .expect_err("未登录要报错");
        assert!(error.to_string().contains("登录"));
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 7);
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

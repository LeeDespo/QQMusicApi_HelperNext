//! 账号关系：用户主页、VIP、关注的歌手、粉丝、好友、关注的人、TA 创建的歌单。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/user.py` 的
//! `get_homepage` / `get_vip_info` / `get_follow_singers` / `get_fans` /
//! `get_friend` / `get_follow_user` / `get_created_songlist`；回值字段名照
//! `models/user.py` 的同名模型（camelCase），模型层级也一样。同一模块的收藏、
//! 音乐基因、不喜欢列表在 `user_asset.rs`；组件既有的 `fetch_followed_artists`
//! 是「读自己关注的歌手」那条老契约，不动。
//!
//! # 平台档案
//!
//! 七个端点参考都没标 `platform`，按本层约定默认 Web；dispatch 收到调用方传入
//! 的 platform 时原样尊重。
//!
//! # 账号标识（euin 与 uin）
//!
//! 参考的主页与三个关系列表按 euin（加密 uin）寻址。调用方给了就用（空串按
//! 没给），没给退回 [`Upstream::encrypted_uin`]——凭据里有 `encrypt_uin` 时直接
//! 取，没有时会去 `GetLoginUserInfo` 找一次。找不到是**凭据问题**，报错而不是
//! 拿空串去发请求：空串会被上游当「查无此人」，回一份空列表，看起来像「这个
//! 账号没有粉丝」。`get_created_songlist` 按**数字 uin** 寻址，调用方没给时
//! 退回凭据的 `music_id`。
//!
//! # 占位凭证
//!
//! `get_homepage` 是公开读取：参考在缺省凭证时自动补一个占位凭证
//! （`musicid 1` / `musickey "placeholder-musickey"` / encryptUin 全 0），
//! 让未登录的宿主也能看别人的主页。这里照做（见 [`homepage_credential`]）：
//! 凭据可用就用凭据本身，不可用就把占位的那一份交给上游去发请求；euin 的解析
//! 始终用**真实凭据**，占位凭证只当通行证。
//!
//! # 空值判据
//!
//! * 三个关系列表（关注的歌手 / 粉丝 / 关注的人）是**分页**读取，形状锚点是
//!   `List` 键（参考的 jsonpath 是 `$.List[*]`）：键不在 / 为 null 说明回值根本
//!   不是这个端点的形状（账号标识不对、会话过期），**报错**——docs/parsing.md
//!   §13：形状不对的空壳不能当空列表返回，否则会覆盖宿主缓存里的真数据。
//!   键在而空**是答案**：这个账号确实没有，或者这一页已翻过列表末尾（`From`
//!   大于总数时上游就给空页）——分页读的「空」本来就可能是合法的。`Total` 与
//!   `HasMore` 原样转给宿主自己翻页。
//! * 好友列表：参考模型里 `friends` / `has_more` 都没有默认值（必填），键不在时
//!   参考自己就校验失败，这里同样报错；空数组是答案（这个账号没有好友）。
//! * 创建的歌单是一次给全量的**整表读取**（`bFinish` 标记是否完整），参考模型里
//!   `playlists` / `total` / `finished` 也都是必填：`v_playlist` 键不在就报错；
//!   键在而空是答案（这个账号没有创建过歌单）。
//! * 主页：`Info.BaseInfo` 是形状锚点（参考里它必填），不在就报错；`Info.Singer`
//!   与 `TabDetail` 是原样透传的 dict，缺了保持 null（与 `user_asset.rs` 对音乐
//!   基因卡片的处理同款：一个锚点足够，不因为次要字段缺失把整页变成故障）。
//! * VIP：`identity` / `userinfo` 在参考里带 `default_factory`（缺失补空对象），
//!   照做；整份 `data` 是空壳的情况由 `Upstream::call_with` 统一拦成错误。
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
    "fetch_user_homepage",
    "fetch_vip_info",
    "fetch_follow_singers",
    "fetch_fans",
    "fetch_friends",
    "fetch_followed_users",
    "fetch_created_playlists",
];

/// 参考的缺省页码。
const DEFAULT_PAGE: i64 = 1;
/// 参考的缺省每页数量。
const DEFAULT_PAGE_SIZE: i64 = 10;

/// 三个关系列表共用的 module（参考 `music.concern.RelationList`）。
const RELATION_MODULE: &str = "music.concern.RelationList";

/// 参考 `UserApi.PLACEHOLDER_CREDENTIAL` 的 musicid。
const PLACEHOLDER_MUSIC_ID: &str = "1";
/// 参考 `UserApi.PLACEHOLDER_CREDENTIAL` 的 musickey。
const PLACEHOLDER_MUSIC_KEY: &str = "placeholder-musickey";
/// 参考 `UserApi.PLACEHOLDER_CREDENTIAL` 的 encryptUin（32 个 0）。
const PLACEHOLDER_ENCRYPT_UIN: &str = "00000000000000000000000000000000";

// MARK: - 模型

/// 参考 `UserHomepageBaseInfo`：主页头部基础信息.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserHomepageBaseInfo {
    /// 加密 UIN（上游键 `EncryptedUin`）.
    pub encrypted_uin: Option<String>,
    /// 用户名.
    pub name: Option<String>,
    /// 头像地址.
    pub avatar: Option<String>,
    /// 背景图地址.
    pub background_image: Option<String>,
    /// 用户类型标记.
    pub user_type: Option<i64>,
}

/// 参考 `UserHomepageResponse`：用户主页视图响应.
///
/// `singer` 与 `tab_detail` 在参考里是 `dict[str, Any]`（原样透传的上游形状），
/// 这里存 JSON 文本：`#[data]` 认不出 `serde_json::Value`，而经 serde 进出时
/// 它们仍是真正的对象（见 `raw_json_serialize`）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserHomepage {
    /// 主页头部基础信息（`$.Info.BaseInfo`）.
    pub base_info: Option<UserHomepageBaseInfo>,
    /// 主页关联歌手信息（`$.Info.Singer`）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub singer: Option<String>,
    /// 当前账号是否已关注（`$.Info.IsFollowed`）.
    pub is_followed: Option<i64>,
    /// 主页标签页附加信息（上游键 `TabDetail`）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub tab_detail: Option<String>,
}

/// 参考 `VipIdentity`：VIP 信息响应中的会员身份明细块.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VipIdentity {
    /// 绿钻会员标志.
    pub vip: Option<i64>,
    /// 豪华绿钻会员标志（上游键 `HugeVip`）.
    pub huge_vip: Option<i64>,
    /// 豪华绿钻生效时间（上游键 `HugeVipStart`）.
    pub huge_vip_start: Option<String>,
    /// 豪华绿钻到期时间（上游键 `HugeVipEnd`）.
    pub huge_vip_end: Option<String>,
    /// 年费会员标志（上游键 `yearflag`）.
    pub year_flag: Option<i64>,
    /// 豪华年费会员标志（上游键 `HugeYearFlag`）.
    pub huge_year_flag: Option<i64>,
    /// 十二平台会员标志.
    pub twelve: Option<i64>,
    /// 十二平台会员生效时间（上游键 `twelveStart`）.
    pub twelve_start: Option<String>,
    /// 十二平台会员到期时间（上游键 `twelveEnd`）.
    pub twelve_end: Option<String>,
    /// 儿童会员标志（上游键 `ChildVip`）.
    pub child_vip: Option<i64>,
    /// 体验会员标志（上游键 `ExpVip`）.
    pub exp_vip: Option<i64>,
    /// 家庭组会员标志（上游键 `GroupVipFlag`）.
    pub group_vip_flag: Option<i64>,
    /// 家庭组会员生效时间（上游键 `GroupVipStart`）.
    pub group_vip_start: Option<String>,
    /// 家庭组会员到期时间（上游键 `GroupVipEnd`）.
    pub group_vip_end: Option<String>,
    /// 情侣会员标志（上游键 `CPLoverFlag`）.
    pub cp_lover_flag: Option<i64>,
    /// 情侣会员生效时间（上游键 `CPLoverStart`）.
    pub cp_lover_start: Option<String>,
    /// 情侣会员到期时间（上游键 `CPLoverEnd`）.
    pub cp_lover_end: Option<String>,
    /// 广告会员标志（上游键 `AdVipFlag`）.
    pub ad_vip_flag: Option<i64>,
    /// 八平台会员标志.
    pub eight: Option<i64>,
    /// 八平台会员生效时间（上游键 `eightStart`）.
    pub eight_start: Option<String>,
    /// 八平台会员到期时间（上游键 `eightEnd`）.
    pub eight_end: Option<String>,
    /// 会员等级.
    pub level: Option<i64>,
    /// 下一会员等级（上游键 `nextlevel`）.
    pub next_level: Option<i64>,
    /// 官方等级徽章图地址.
    pub icon: Option<String>,
    /// 会员购买页地址（上游键 `purchaseUrl`）.
    pub purchase_url: Option<String>,
}

/// 参考 `VipUserInfo`：VIP 信息响应中的用户权益摘要块.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VipUserInfo {
    /// 开通入口地址（参考同时接受 `buy_url` / `buyurl`）.
    pub buy_url: Option<String>,
    /// 我的会员页地址（参考同时接受 `my_vip_url` / `myvipurl`）.
    pub my_vip_url: Option<String>,
    /// 会员积分.
    pub score: Option<i64>,
    /// 到期时间戳.
    pub expire: Option<i64>,
    /// 音乐等级（上游键 `music_level`）.
    pub music_level: Option<i64>,
}

/// 参考 `UserVipInfoResponse`：VIP 信息视图响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserVipInfo {
    /// 自动下载开关状态（参考同时接受 `auto_down` / `autoDown` / `autodown`）.
    pub auto_down: Option<i64>,
    /// 是否可续费（上游键 `canRenew`）.
    pub can_renew: Option<i64>,
    /// 最大歌单数量（参考同时接受 `max_dir_num` / `maxDirNum` / `maxdirnum`）.
    pub max_dir_num: Option<i64>,
    /// 最大歌曲数量（参考同时接受 `max_song_num` / `maxSongNum` / `maxsongnum`）.
    pub max_song_num: Option<i64>,
    /// 歌曲上限提示文案（参考同时接受 `song_limit_msg` / `songLimitMsg`）.
    pub song_limit_msg: Option<String>,
    /// 超级会员标志.
    pub svip: Option<i64>,
    /// 星级会员标志.
    pub star: Option<i64>,
    /// 星级会员生效时间（上游键 `starstart`）.
    pub star_start: Option<String>,
    /// 星级会员到期时间（上游键 `starend`）.
    pub star_end: Option<String>,
    /// 年费星级会员标志.
    pub ystar: Option<i64>,
    /// 年费星级会员生效时间（上游键 `ystarstart`）.
    pub ystar_start: Option<String>,
    /// 年费星级会员到期时间（上游键 `ystarend`）.
    pub ystar_end: Option<String>,
    /// 会员身份明细（参考里带 `default_factory`，缺了补空对象）.
    pub identity: Option<VipIdentity>,
    /// 用户权益摘要（参考里带 `default_factory`，缺了补空对象）.
    pub userinfo: Option<VipUserInfo>,
}

/// 参考 `RelationUser`：关注或粉丝列表中的单个用户条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelationUser {
    /// 用户 MID（上游键 `MID`）.
    pub mid: Option<String>,
    /// 加密 UIN（上游键 `EncUin`）.
    pub enc_uin: Option<String>,
    /// 用户名称（上游键 `Name`）.
    pub name: Option<String>,
    /// 描述文案（上游键 `Desc`）.
    pub desc: Option<String>,
    /// 头像地址（上游键 `AvatarUrl`）.
    pub avatar_url: Option<String>,
    /// 粉丝数（上游键 `FanNum`）.
    pub fan_num: Option<i64>,
    /// 当前账号是否已关注（上游键 `IsFollow`）.
    pub is_follow: Option<bool>,
}

/// 参考 `UserRelationListResponse`：关注关系分页列表响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserRelationList {
    /// 总数量（上游键 `Total`）.
    pub total: Option<i64>,
    /// 当前页用户列表（上游键 `List`）.
    pub users: Option<Vec<RelationUser>>,
    /// 是否还有更多结果（上游键 `HasMore`）.
    pub has_more: Option<bool>,
    /// 下一页游标（上游键 `LastPos`）.
    pub last_pos: Option<String>,
    /// 附加消息（上游键 `Msg`）.
    pub msg: Option<String>,
    /// 锁定状态标记（上游键 `LockFlag`）.
    pub lock_flag: Option<i64>,
    /// 锁定提示文案（上游键 `LockMsg`）.
    pub lock_msg: Option<String>,
}

/// 参考 `FriendEntry`：好友列表中的单个好友条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FriendEntry {
    /// 加密 UIN（上游键 `EncryptUin`）.
    pub encrypt_uin: Option<String>,
    /// 用户名（上游键 `UserName`）.
    pub user_name: Option<String>,
    /// 头像地址（上游键 `AvatarUrl`）.
    pub avatar_url: Option<String>,
    /// 当前账号是否已关注（上游键 `IsFollow`）.
    pub is_follow: Option<bool>,
}

/// 参考 `UserFriendListResponse`：好友列表视图响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserFriendList {
    /// 当前页好友列表（上游键 `Friends`）.
    pub friends: Option<Vec<FriendEntry>>,
    /// 是否还有更多结果（上游键 `HasMore`）.
    pub has_more: Option<bool>,
}

/// 参考 `UserPlaylistSummary`（继承 `SongList`）：用户创建的歌单摘要.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserPlaylistSummary {
    /// 歌单 ID（参考别名含 `tid` / `dissid`）.
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
    /// 创建时间戳（上游键 `createTime`）.
    pub create_time: Option<i64>,
    /// 更新时间戳（上游键 `updateTime`）.
    pub update_time: Option<i64>,
    /// 创建者 UIN.
    pub uin: Option<String>,
    /// 创建者昵称.
    pub nick: Option<String>,
    /// 大图封面地址（上游键 `bigpicUrl`）.
    pub bigpic_url: Option<String>,
    /// 专辑拼接封面地址（上游键 `albumPicUrl`）.
    pub album_pic_url: Option<String>,
    /// 创建者头像.
    pub avatar: Option<String>,
    /// 身份图标地址（上游键 `identIcon`）.
    pub ident_icon: Option<String>,
    /// 分层装饰地址（上游键 `layerUrl`）.
    pub layer_url: Option<String>,
    /// 是否失效.
    pub invalid: Option<bool>,
    /// 目录展示标记（上游键 `dirShow`）.
    pub dir_show: Option<i64>,
    /// 创建者收藏量（上游键 `fav_cnt`）.
    pub create_fav_cnt: Option<i64>,
    /// 播放量.
    pub play_cnt: Option<i64>,
    /// 评论数.
    pub comment_cnt: Option<i64>,
    /// 操作类型标记（上游键 `opType`）.
    pub op_type: Option<i64>,
    /// 排序权重（上游键 `sortWeight`）.
    pub sort_weight: Option<i64>,
}

/// 参考 `UserCreatedSonglistResponse`：用户创建歌单列表页响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserCreatedSonglistResponse {
    /// 歌单总数.
    pub total: Option<i64>,
    /// 当前页歌单摘要列表（上游键 `v_playlist`）.
    pub playlists: Option<Vec<UserPlaylistSummary>>,
    /// 上游返回的删除歌单 ID 标记（上游键 `v_delTid`）.
    pub deleted_ids: Option<Vec<i64>>,
    /// 是否已经拉取完成（上游键 `bFinish`）.
    pub finished: Option<bool>,
}

// MARK: - 参考模型里 `dict` 形状的字段

/// 把 Rust 侧存的 JSON 文本按真 JSON 写出（宿主在 JSON 协议上看到的是对象/数组）。
///
/// 与 `user_asset.rs` / `comment.rs` 的同名助手同源：每个领域文件自包含，这份是刻意复制的。
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

/// 封面 / 头像类 URL 一律转 https（docs/parsing.md §7：宿主会直接拒绝 http 图片）。
fn artwork(value: Option<String>) -> Option<String> {
    crate::methods::normalized_artwork_url(value.as_deref())
}

/// 一组整数（上游给数字也给数字字符串，例如 `v_delTid`）。
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

/// 主页头部基础信息（参考 `UserHomepageBaseInfo`）。
fn homepage_base_payload(raw: &Value) -> Value {
    json!({
        "encryptedUin": first_text(raw, &["EncryptedUin", "encryptedUin", "encrypted_uin"]),
        "name": first_text(raw, &["Name", "name"]),
        "avatar": artwork(first_text(raw, &["Avatar", "avatar"])),
        "backgroundImage": artwork(first_text(raw, &["BackgroundImage", "backgroundImage"])),
        "userType": first_int(raw, &["UserType", "userType"]),
    })
}

/// 主页回值（参考 `UserHomepageResponse`）。
///
/// 形状锚点是 `Info.BaseInfo`（参考里它必填）：不在说明这根本不是主页的回值，
/// 报错比伪装成「这个用户没有主页」好查。
fn homepage_payload(data: &Value) -> Result<Value, UpstreamError> {
    let info = first_object(data, &["Info", "info"]).ok_or_else(|| {
        UpstreamError::Upstream("上游没有返回用户主页信息（Info）：账号标识可能不对".into())
    })?;
    let base = first_object(info, &["BaseInfo", "baseInfo"]).ok_or_else(|| {
        UpstreamError::Upstream("上游没有返回用户主页基础信息（Info.BaseInfo）".into())
    })?;
    Ok(json!({
        "baseInfo": homepage_base_payload(base),
        "singer": raw_field(info, &["Singer", "singer"]),
        "isFollowed": first_int(info, &["IsFollowed", "isFollowed"]),
        "tabDetail": raw_field(data, &["TabDetail", "tabDetail"]),
    }))
}

/// 会员身份明细（参考 `VipIdentity`，两种拼写照参考的 AliasChoices 收）。
fn vip_identity_payload(raw: &Value) -> Value {
    json!({
        "vip": first_int(raw, &["vip", "Vip"]),
        "hugeVip": first_int(raw, &["HugeVip", "hugeVip"]),
        "hugeVipStart": first_text(raw, &["HugeVipStart", "hugeVipStart"]),
        "hugeVipEnd": first_text(raw, &["HugeVipEnd", "hugeVipEnd"]),
        "yearFlag": first_int(raw, &["yearflag", "yearFlag"]),
        "hugeYearFlag": first_int(raw, &["HugeYearFlag", "hugeYearFlag"]),
        "twelve": first_int(raw, &["twelve", "Twelve"]),
        "twelveStart": first_text(raw, &["twelveStart", "TwelveStart"]),
        "twelveEnd": first_text(raw, &["twelveEnd", "TwelveEnd"]),
        "childVip": first_int(raw, &["ChildVip", "childVip"]),
        "expVip": first_int(raw, &["ExpVip", "expVip"]),
        "groupVipFlag": first_int(raw, &["GroupVipFlag", "groupVipFlag"]),
        "groupVipStart": first_text(raw, &["GroupVipStart", "groupVipStart"]),
        "groupVipEnd": first_text(raw, &["GroupVipEnd", "groupVipEnd"]),
        "cpLoverFlag": first_int(raw, &["CPLoverFlag", "cpLoverFlag"]),
        "cpLoverStart": first_text(raw, &["CPLoverStart", "cpLoverStart"]),
        "cpLoverEnd": first_text(raw, &["CPLoverEnd", "cpLoverEnd"]),
        "adVipFlag": first_int(raw, &["AdVipFlag", "adVipFlag"]),
        "eight": first_int(raw, &["eight", "Eight"]),
        "eightStart": first_text(raw, &["eightStart", "EightStart"]),
        "eightEnd": first_text(raw, &["eightEnd", "EightEnd"]),
        "level": first_int(raw, &["level", "Level"]),
        "nextLevel": first_int(raw, &["nextlevel", "nextLevel"]),
        "icon": artwork(first_text(raw, &["icon", "Icon"])),
        "purchaseUrl": first_text(raw, &["purchaseUrl", "purchase_url"]),
    })
}

/// 用户权益摘要（参考 `VipUserInfo`，两种拼写照参考的 AliasChoices 收）。
fn vip_userinfo_payload(raw: &Value) -> Value {
    json!({
        "buyUrl": first_text(raw, &["buy_url", "buyurl", "buyUrl"]),
        "myVipUrl": first_text(raw, &["my_vip_url", "myvipurl", "myVipUrl"]),
        "score": first_int(raw, &["score", "Score"]),
        "expire": first_int(raw, &["expire", "Expire"]),
        "musicLevel": first_int(raw, &["music_level", "musicLevel"]),
    })
}

/// VIP 信息回值（参考 `UserVipInfoResponse`）。
///
/// `identity` / `userinfo` 缺失时按参考的 `default_factory` 补空对象：
/// 「这个账号没有这项会员」是答案，不是故障。
fn vip_payload(data: &Value) -> Value {
    let empty = Value::Null;
    json!({
        "autoDown": first_int(data, &["auto_down", "autoDown", "autodown"]),
        "canRenew": first_int(data, &["canRenew", "can_renew"]),
        "maxDirNum": first_int(data, &["max_dir_num", "maxDirNum", "maxdirnum"]),
        "maxSongNum": first_int(data, &["max_song_num", "maxSongNum", "maxsongnum"]),
        "songLimitMsg": first_text(data, &["song_limit_msg", "songLimitMsg"]),
        "svip": first_int(data, &["svip", "Svip"]),
        "star": first_int(data, &["star", "Star"]),
        "starStart": first_text(data, &["starstart", "starStart"]),
        "starEnd": first_text(data, &["starend", "starEnd"]),
        "ystar": first_int(data, &["ystar", "Ystar"]),
        "ystarStart": first_text(data, &["ystarstart", "ystarStart"]),
        "ystarEnd": first_text(data, &["ystarend", "ystarEnd"]),
        "identity": vip_identity_payload(
            first_object(data, &["identity", "Identity"]).unwrap_or(&empty)
        ),
        "userinfo": vip_userinfo_payload(
            first_object(data, &["userinfo", "UserInfo", "userInfo"]).unwrap_or(&empty)
        ),
    })
}

/// 关注 / 粉丝列表里的单个用户（参考 `RelationUser`）。
fn relation_user_payload(raw: &Value) -> Value {
    json!({
        "mid": first_text(raw, &["MID", "mid"]),
        "encUin": first_text(raw, &["EncUin", "encUin", "enc_uin"]),
        "name": first_text(raw, &["Name", "name"]),
        "desc": first_text(raw, &["Desc", "desc"]),
        "avatarUrl": artwork(first_text(raw, &["AvatarUrl", "avatarUrl", "avatar_url"])),
        "fanNum": first_int(raw, &["FanNum", "fanNum", "fan_num"]),
        "isFollow": bool_field(raw, &["IsFollow", "isFollow", "is_follow"]),
    })
}

/// 关系分页回值（参考 `UserRelationListResponse`）。
///
/// 形状锚点是 `List` 键（判据见文件头「空值判据」）：键不在 / 为 null 是故障，
/// 键在而空是答案。这里比参考模型的 `default_factory` 更严一档，理由就是
/// docs/parsing.md §13——形状不对的空壳不能当空列表返回。
fn relation_list_payload(data: &Value, what: &str) -> Result<Value, UpstreamError> {
    let users = first_array(data, &["List", "list", "users"]).ok_or_else(|| {
        UpstreamError::Upstream(format!(
            "上游没有返回{what}（List）：账号标识可能不对或登录已过期"
        ))
    })?;
    Ok(json!({
        "total": first_int(data, &["Total", "total"]),
        "users": users.iter().map(relation_user_payload).collect::<Vec<_>>(),
        "hasMore": bool_field(data, &["HasMore", "hasMore"]),
        "lastPos": first_text(data, &["LastPos", "lastPos"]),
        "msg": first_text(data, &["Msg", "msg"]),
        "lockFlag": first_int(data, &["LockFlag", "lockFlag"]),
        "lockMsg": first_text(data, &["LockMsg", "lockMsg"]),
    }))
}

/// 好友列表里的单个条目（参考 `FriendEntry`）。
fn friend_payload(raw: &Value) -> Value {
    json!({
        "encryptUin": first_text(raw, &["EncryptUin", "encryptUin", "encrypt_uin"]),
        "userName": first_text(raw, &["UserName", "userName", "user_name"]),
        "avatarUrl": artwork(first_text(raw, &["AvatarUrl", "avatarUrl", "avatar_url"])),
        "isFollow": bool_field(raw, &["IsFollow", "isFollow", "is_follow"]),
    })
}

/// 好友列表回值（参考 `UserFriendListResponse`）。
///
/// `friends` / `has_more` 在参考里没有默认值（必填），所以键不在就报错；
/// 空数组是答案（这个账号没有好友）。
fn friend_list_payload(data: &Value) -> Result<Value, UpstreamError> {
    let friends = first_array(data, &["Friends", "friends"]).ok_or_else(|| {
        UpstreamError::Upstream("上游没有返回好友列表（Friends）：登录可能已过期".into())
    })?;
    Ok(json!({
        "friends": friends.iter().map(friend_payload).collect::<Vec<_>>(),
        "hasMore": bool_field(data, &["HasMore", "hasMore"]),
    }))
}

/// 创建的歌单摘要（参考 `UserPlaylistSummary`）。
fn playlist_summary_payload(raw: &Value) -> Value {
    json!({
        "id": first_int(raw, &["id", "tid", "dissid", "dissId"]),
        "dirid": first_int(raw, &["dirid", "dirId"]),
        "title": first_text(raw, &["title", "dissname", "name", "dirName"]),
        "picurl": artwork(first_text(raw, &["picurl", "cover", "logo", "picUrl"])),
        "desc": first_text(raw, &["desc", "description"]),
        "songnum": first_int(raw, &["songnum", "songNum", "song_cnt"]),
        "listennum": first_int(raw, &["listennum", "playCnt", "play_cnt"]),
        "createTime": first_int(raw, &["createTime", "createtime", "create_time"]),
        "updateTime": first_int(raw, &["updateTime", "updatetime", "update_time"]),
        "uin": first_text(raw, &["uin"]),
        "nick": first_text(raw, &["nick", "nickname"]),
        "bigpicUrl": artwork(first_text(raw, &["bigpicUrl", "bigPicUrl", "bigpic_url"])),
        "albumPicUrl": artwork(first_text(raw, &["albumPicUrl", "albumpicUrl", "album_pic_url"])),
        "avatar": artwork(first_text(raw, &["avatar"])),
        "identIcon": artwork(first_text(raw, &["identIcon", "ident_icon"])),
        "layerUrl": artwork(first_text(raw, &["layerUrl", "layer_url"])),
        "invalid": bool_field(raw, &["invalid"]),
        "dirShow": first_int(raw, &["dirShow", "dir_show"]),
        "createFavCnt": first_int(raw, &["fav_cnt", "create_fav_cnt", "createFavCnt"]),
        "playCnt": first_int(raw, &["play_cnt", "playCnt"]),
        "commentCnt": first_int(raw, &["comment_cnt", "commentCnt"]),
        "opType": first_int(raw, &["opType", "op_type"]),
        "sortWeight": first_int(raw, &["sortWeight", "sort_weight"]),
    })
}

/// 创建的歌单回值（参考 `UserCreatedSonglistResponse`）。
///
/// 整表读取：`v_playlist` 键不在就报错（参考模型里这个列表必填，判据见文件头）；
/// 键在而空是答案。
fn created_songlists_payload(data: &Value) -> Result<Value, UpstreamError> {
    let playlists =
        first_array(data, &["v_playlist", "vPlaylist", "playlists"]).ok_or_else(|| {
            UpstreamError::Upstream(
                "上游没有返回创建的歌单列表（v_playlist）：账号标识可能不对".into(),
            )
        })?;
    Ok(json!({
        "total": first_int(data, &["total", "Total"]),
        "playlists": playlists.iter().map(playlist_summary_payload).collect::<Vec<_>>(),
        "deletedIds": int_list(data, &["v_delTid", "vDelTid", "deletedIds"]),
        "finished": bool_field(data, &["bFinish", "BFinish", "finished"]),
    }))
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

/// 账号标识：调用方给了就用（空串 / 空白按没给），没给退回 [`Upstream::encrypted_uin`]。
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

/// 数字 uin：调用方给了就用，没给退回凭据的 `music_id`。
///
/// 参考的 `get_created_songlist` 把 uin 声明成必填；组件对外多一层便利——
/// 不给（或 0）时读**当前登录账号**创建的歌单。凭据里连数字 uin 都没有就是
/// 凭据问题，报错而不是拿空串去发请求。
fn resolved_uin(credential: &Credential, params: &Value) -> Result<String, UpstreamError> {
    if let Some(uin) = first_int(params, &["uin"]).filter(|value| *value > 0) {
        return Ok(uin.to_string());
    }
    if credential.music_id.is_empty() {
        return Err(UpstreamError::Upstream(
            "这个凭据没有可用的账号标识（uin）：请先登录或显式给 uin".into(),
        ));
    }
    Ok(credential.music_id.clone())
}

/// 主页用的凭证：参考的 `_resolve_placeholder_credential`。
///
/// 凭据可用就用凭据本身；不可用就换参考的占位凭证（`musicid 1` /
/// `musickey "placeholder-musickey"` / encryptUin 全 0，登录类型 1=QQ），让未登录
/// 的宿主也能看别人的主页。占位凭证只当通行证——euin 始终从**真实凭据**解析。
fn homepage_credential(credential: &Credential) -> Credential {
    if credential.is_usable() {
        return credential.clone();
    }
    Credential {
        music_id: PLACEHOLDER_MUSIC_ID.into(),
        music_key: PLACEHOLDER_MUSIC_KEY.into(),
        encrypted_uin: PLACEHOLDER_ENCRYPT_UIN.into(),
        // 参考的占位凭证带 `loginType: 1`；`Credential` 没有这个字段，
        // 放进 `raw`，读法与真实凭据一致（`singer_extra::display_comm` 就是这么读的）。
        raw: json!({ "loginType": 1 }),
    }
}

/// 页码：参考的缺省是 1，给什么发什么（参考不做区间钳制）。
fn page_of(params: &Value) -> i64 {
    first_int(params, &["page"]).unwrap_or(DEFAULT_PAGE)
}

/// 每页数量：参考叫 `num`，驱动组件的调用方也常用 `limit`/`size`。
fn size_of(params: &Value) -> i64 {
    first_int(params, &["num", "limit", "size"]).unwrap_or(DEFAULT_PAGE_SIZE)
}

/// 主页参数（参考 `get_homepage`）。
fn homepage_params(euin: &str) -> Value {
    json!({ "uin": euin, "IsQueryTabDetail": 1 })
}

/// 关系列表参数（参考三个 `HostUin` 端点共用）：页码换算成偏移量 `From`。
fn relation_params(euin: &str, page: i64, num: i64) -> Value {
    json!({
        "HostUin": euin,
        "From": (page - 1) * num,
        "Size": num,
    })
}

/// 好友列表参数（参考 `get_friend`）：`Page` 就是页码减一。
fn friend_params(page: i64, num: i64) -> Value {
    json!({ "PageSize": num, "Page": page - 1 })
}

/// 创建的歌单参数（参考 `get_created_songlist`）：数字 uin 转成字符串。
fn created_songlist_params(uin: &str) -> Value {
    json!({ "uin": uin })
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
        "fetch_user_homepage" => user_homepage(upstream, credential, platform, params),
        "fetch_vip_info" => vip_info(upstream, credential, platform, params),
        "fetch_follow_singers" => follow_singers(upstream, credential, platform, params),
        "fetch_fans" => fans(upstream, credential, platform, params),
        "fetch_friends" => friends(upstream, credential, platform, params),
        "fetch_followed_users" => followed_users(upstream, credential, platform, params),
        "fetch_created_playlists" => created_playlists(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 用户主页（`music.UnifiedHomepage.UnifiedHomepageSrv / GetHomepageHeader`）。
///
/// 公开读取：参考没有 `require_login`，缺省凭证时用占位凭证（见
/// [`homepage_credential`]）。
fn user_homepage(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let euin = resolved_euin(upstream, credential, params)?;
    let data = upstream.call_with(
        &homepage_credential(credential),
        Class::Read,
        platform,
        Call {
            module: "music.UnifiedHomepage.UnifiedHomepageSrv",
            method: "GetHomepageHeader",
            param: homepage_params(&euin),
        },
    )?;
    homepage_payload(&data)
}

/// VIP 信息（`VipLogin.VipLoginInter / vip_login_base`），参考标了 `require_login`。
///
/// 参数是空对象，账号身份全在 comm 与 cookies 里（参考的 `CgiRequestData(credential=…)`）。
fn vip_info(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    _params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: "VipLogin.VipLoginInter",
            method: "vip_login_base",
            param: json!({}),
        },
    )?;
    Ok(vip_payload(&data))
}

/// 关系列表三个端点的共用实现：模块相同，只有 method 与报错里的名字不同。
fn relation_list(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
    method: &'static str,
    what: &str,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let euin = resolved_euin(upstream, credential, params)?;
    let param = relation_params(&euin, page_of(params), size_of(params));
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: RELATION_MODULE,
            method,
            param,
        },
    )?;
    relation_list_payload(&data, what)
}

/// 关注的歌手（`music.concern.RelationList / GetFollowSingerList`），需要登录。
///
/// 注意别和既有 `fetch_followed_artists` 混了：那条是组件自己的老契约（没有
/// euin 参数），这条按参考的协议寻址任意账号。
fn follow_singers(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    relation_list(
        upstream,
        credential,
        platform,
        params,
        "GetFollowSingerList",
        "关注的歌手列表",
    )
}

/// 粉丝（`music.concern.RelationList / GetFansList`），需要登录。
fn fans(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    relation_list(
        upstream,
        credential,
        platform,
        params,
        "GetFansList",
        "粉丝列表",
    )
}

/// 关注的人（`music.concern.RelationList / GetFollowUserList`），需要登录。
fn followed_users(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    relation_list(
        upstream,
        credential,
        platform,
        params,
        "GetFollowUserList",
        "关注的用户列表",
    )
}

/// 好友（`music.homepage.Friendship / GetFriendList`），需要登录。
fn friends(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: "music.homepage.Friendship",
            method: "GetFriendList",
            param: friend_params(page_of(params), size_of(params)),
        },
    )?;
    friend_list_payload(&data)
}

/// 用户创建的歌单（`music.musicasset.PlaylistBaseRead / GetPlaylistByUin`）。
///
/// 参考没有 `require_login`：显式给了数字 uin 就是公开读取；没给时才需要
/// 当前账号的 `music_id`（见 [`resolved_uin`]）。
fn created_playlists(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let uin = resolved_uin(credential, params)?;
    let data = upstream.call_with(
        credential,
        Class::Account,
        platform,
        Call {
            module: "music.musicasset.PlaylistBaseRead",
            method: "GetPlaylistByUin",
            param: created_songlist_params(&uin),
        },
    )?;
    created_songlists_payload(&data)
}

// MARK: - 宿主包装
//
// euin 不给（或给空串）时由协议层退回 `Upstream::encrypted_uin`；uin 不给时
// 退回凭据的数字 uin。省略的 page/num 按参考的缺省（第 1 页、每页 10 条）。

/// 三个关系列表包装共用的参数拼装。
fn relation_wrapper_params(euin: Option<String>, page: Option<i64>, num: Option<i64>) -> Value {
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
    params
}

/// 用户主页；`euin` 不给时退回凭据的加密 uin。
#[export]
pub fn fetch_user_homepage(euin: Option<String>) -> Result<UserHomepage, crate::HelperError> {
    let mut params = json!({});
    if let Some(euin) = euin.filter(|value| !value.trim().is_empty()) {
        params["euin"] = json!(euin);
    }
    crate::port::call("fetch_user_homepage", params)
}

/// 当前登录账号的 VIP 信息；需要登录。
#[export]
pub fn fetch_vip_info() -> Result<UserVipInfo, crate::HelperError> {
    crate::port::call("fetch_vip_info", json!({}))
}

/// 某个账号关注的歌手一页；需要登录，`total` / `hasMore` 用来翻页。
#[export]
pub fn fetch_follow_singers(
    euin: Option<String>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserRelationList, crate::HelperError> {
    crate::port::call(
        "fetch_follow_singers",
        relation_wrapper_params(euin, page, num),
    )
}

/// 某个账号的粉丝一页；需要登录。
#[export]
pub fn fetch_fans(
    euin: Option<String>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserRelationList, crate::HelperError> {
    crate::port::call("fetch_fans", relation_wrapper_params(euin, page, num))
}

/// 好友一页；需要登录，`hasMore` 用来翻页。
#[export]
pub fn fetch_friends(
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserFriendList, crate::HelperError> {
    let mut params = json!({});
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    crate::port::call("fetch_friends", params)
}

/// 某个账号关注的人一页；需要登录。
#[export]
pub fn fetch_followed_users(
    euin: Option<String>,
    page: Option<i64>,
    num: Option<i64>,
) -> Result<UserRelationList, crate::HelperError> {
    crate::port::call(
        "fetch_followed_users",
        relation_wrapper_params(euin, page, num),
    )
}

/// 某个账号创建的歌单一览；`uin` 不给时读当前登录账号的。
#[export]
pub fn fetch_created_playlists(
    uin: Option<i64>,
) -> Result<UserCreatedSonglistResponse, crate::HelperError> {
    let mut params = json!({});
    if let Some(uin) = uin {
        params["uin"] = json!(uin);
    }
    crate::port::call("fetch_created_playlists", params)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份主页回值，键名照上游自己的拼写。
    fn raw_homepage() -> Value {
        json!({
            "Info": {
                "BaseInfo": {
                    "EncryptedUin": "7eEFNeSlNKns",
                    "Name": "甲",
                    "Avatar": "http://y.gtimg.cn/avatar.jpg",
                    "BackgroundImage": "http://y.gtimg.cn/background.jpg",
                    "UserType": 1
                },
                "Singer": {"SingerMid": "0025NhlN2yWrP4", "Name": "甲"},
                "IsFollowed": 0
            },
            "TabDetail": {"TabList": [{"TabID": "song"}]}
        })
    }

    /// 一份关系列表回值（关注歌手 / 粉丝 / 关注的人共用形状）。
    fn raw_relation_page() -> Value {
        json!({
            "Total": 1,
            "List": [{
                "MID": "0025NhlN2yWrP4",
                "EncUin": "7eEFNeSlNKns",
                "Name": "甲",
                "Desc": "这个人很懒",
                "AvatarUrl": "http://y.gtimg.cn/avatar.jpg",
                "FanNum": 12,
                "IsFollow": true
            }],
            "HasMore": 1,
            "LastPos": "pos-1",
            "Msg": "",
            "LockFlag": 0,
            "LockMsg": ""
        })
    }

    /// 一份好友列表回值。
    fn raw_friend_page() -> Value {
        json!({
            "Friends": [{
                "EncryptUin": "euin-1",
                "UserName": "乙",
                "AvatarUrl": "http://y.gtimg.cn/friend.jpg",
                "IsFollow": false
            }],
            "HasMore": false
        })
    }

    /// 一份创建的歌单回值，键名照上游与参考别名的拼写。
    fn raw_created_playlists() -> Value {
        json!({
            "total": 2,
            "v_playlist": [{
                "tid": 9578424174i64,
                "dirid": 0,
                "dissname": "我的歌单",
                "picurl": "http://y.gtimg.cn/music/photo_new/T002R300x300M0000041WVfh2vtlJE.jpg",
                "desc": "",
                "songnum": 3,
                "listennum": 5,
                "createTime": 1_600_000_000,
                "updateTime": 1_600_000_100,
                "uin": "2651932936",
                "nick": "甲",
                "bigpicUrl": "http://y.gtimg.cn/big.jpg",
                "albumPicUrl": "",
                "avatar": "http://y.gtimg.cn/avatar.jpg",
                "identIcon": "",
                "layerUrl": "http://y.gtimg.cn/layer.png",
                "invalid": 0,
                "dirShow": 1,
                "fav_cnt": 2,
                "play_cnt": 3,
                "comment_cnt": 4,
                "opType": 1,
                "sortWeight": 9
            }],
            "v_delTid": [1, "2"],
            "bFinish": true
        })
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 7);
        let upstream = Upstream::new();
        // 既有的两条老契约不能被这一层抢走。
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_followed_artists",
            &json!({})
        )
        .is_none());
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "fetch_fav_playlists",
            &json!({})
        )
        .is_none());
    }

    #[test]
    fn every_request_is_the_reference_one() {
        assert_eq!(
            homepage_params("EUIN"),
            json!({ "uin": "EUIN", "IsQueryTabDetail": 1 })
        );
        assert_eq!(
            relation_params("EUIN", 1, 10),
            json!({ "HostUin": "EUIN", "From": 0, "Size": 10 })
        );
        assert_eq!(
            relation_params("EUIN", 3, 5),
            json!({ "HostUin": "EUIN", "From": 10, "Size": 5 }),
            "参考把页码换算成 From = (page - 1) * num"
        );
        assert_eq!(
            friend_params(1, 10),
            json!({ "PageSize": 10, "Page": 0 }),
            "参考的 Page 就是页码减一"
        );
        assert_eq!(friend_params(3, 5), json!({ "PageSize": 5, "Page": 2 }));
        assert_eq!(
            created_songlist_params("2651932936"),
            json!({ "uin": "2651932936" })
        );
        assert_eq!(page_of(&json!({})), 1, "参考的缺省页码");
        assert_eq!(size_of(&json!({})), 10, "参考的缺省每页数量");
        assert_eq!(size_of(&json!({ "limit": 5 })), 5);
        assert_eq!(page_of(&json!({ "page": 4 })), 4);
    }

    #[test]
    fn a_homepage_maps_base_info_singer_and_tab_detail() {
        let payload = homepage_payload(&raw_homepage()).expect("解析主页");
        let page: UserHomepage = serde_json::from_value(payload.clone()).expect("模型解析");
        let base = page.base_info.expect("有基础信息");
        assert_eq!(base.encrypted_uin.as_deref(), Some("7eEFNeSlNKns"));
        assert_eq!(base.name.as_deref(), Some("甲"));
        assert_eq!(
            base.avatar.as_deref(),
            Some("https://y.gtimg.cn/avatar.jpg"),
            "头像一律 https（docs/parsing.md §7）"
        );
        assert_eq!(
            base.background_image.as_deref(),
            Some("https://y.gtimg.cn/background.jpg")
        );
        assert_eq!(base.user_type, Some(1));
        assert_eq!(page.is_followed, Some(0));
        // 透传的 dict 在 Rust 侧是 JSON 文本，展开后还是原来的对象……
        let singer: Value = serde_json::from_str(page.singer.as_deref().expect("有歌手")).unwrap();
        assert_eq!(singer["SingerMid"], "0025NhlN2yWrP4");
        // ……经 serde 写出去时仍以对象出现，宿主看到的形状不变。
        assert_eq!(payload["baseInfo"]["encryptedUin"], "7eEFNeSlNKns");
        assert_eq!(payload["tabDetail"]["TabList"][0]["TabID"], "song");
    }

    #[test]
    fn a_homepage_without_base_info_is_a_shape_fault() {
        let error = homepage_payload(&json!({ "Info": {} })).expect_err("缺 BaseInfo 要报错");
        assert!(error.to_string().contains("BaseInfo"), "{error}");
        let error = homepage_payload(&json!({})).expect_err("缺 Info 要报错");
        assert!(error.to_string().contains("Info"), "{error}");
    }

    #[test]
    fn the_vip_response_accepts_both_spellings() {
        let payload = vip_payload(&json!({
            "auto_down": 1,
            "canRenew": 0,
            "max_dir_num": 100,
            "maxsongnum": 5000,
            "song_limit_msg": "",
            "svip": 1,
            "star": 0,
            "starstart": "2025-01-01",
            "starend": "2026-01-01",
            "ystar": 1,
            "ystarstart": "",
            "ystarend": "",
            "identity": {
                "vip": 1,
                "HugeVip": 1,
                "HugeVipStart": "a",
                "HugeVipEnd": "b",
                "yearflag": 1,
                "HugeYearFlag": 0,
                "twelve": 1,
                "twelveStart": "c",
                "twelveEnd": "d",
                "ChildVip": 0,
                "ExpVip": 0,
                "GroupVipFlag": 0,
                "CPLoverFlag": 0,
                "AdVipFlag": 0,
                "eight": 1,
                "eightStart": "e",
                "eightEnd": "f",
                "level": 9,
                "nextlevel": 10,
                "icon": "http://y.gtimg.cn/vip.png",
                "purchaseUrl": "https://y.qq.com/vip"
            },
            "userinfo": {
                "buy_url": "https://y.qq.com/buy",
                "myvipurl": "https://y.qq.com/my",
                "score": 10,
                "expire": 1_700_000_000,
                "music_level": 8
            }
        }));
        let vip: UserVipInfo = serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(vip.auto_down, Some(1), "上游键是 auto_down");
        assert_eq!(vip.can_renew, Some(0), "上游键是 canRenew");
        assert_eq!(vip.max_dir_num, Some(100));
        assert_eq!(
            vip.max_song_num,
            Some(5000),
            "上游键是 maxsongnum（参考的第三种拼写）"
        );
        assert_eq!(vip.svip, Some(1));
        assert_eq!(payload["maxSongNum"], 5000, "宿主按 camelCase 读");
        let identity = vip.identity.expect("有身份明细");
        assert_eq!(identity.huge_vip, Some(1));
        assert_eq!(identity.year_flag, Some(1), "上游键是 yearflag");
        assert_eq!(identity.next_level, Some(10), "上游键是 nextlevel");
        assert_eq!(
            identity.icon.as_deref(),
            Some("https://y.gtimg.cn/vip.png"),
            "徽章图一律 https"
        );
        assert_eq!(
            identity.purchase_url.as_deref(),
            Some("https://y.qq.com/vip")
        );
        let userinfo = vip.userinfo.expect("有权益摘要");
        assert_eq!(
            userinfo.buy_url.as_deref(),
            Some("https://y.qq.com/buy"),
            "参考同时接受 buy_url 与 buyurl"
        );
        assert_eq!(
            userinfo.my_vip_url.as_deref(),
            Some("https://y.qq.com/my"),
            "参考同时接受 my_vip_url 与 myvipurl"
        );
        assert_eq!(userinfo.music_level, Some(8), "上游键是 music_level");
    }

    #[test]
    fn a_vip_response_without_the_optional_blocks_still_parses() {
        // identity / userinfo 在参考里带 default_factory：缺了补空对象，不是故障。
        let vip: UserVipInfo =
            serde_json::from_value(vip_payload(&json!({ "svip": 0 }))).expect("解析空壳 VIP");
        let identity = vip.identity.expect("有身份字段");
        assert_eq!(identity.vip, None);
        let userinfo = vip.userinfo.expect("有权益字段");
        assert_eq!(userinfo.score, None);
    }

    #[test]
    fn a_relation_page_reads_the_capital_list_key() {
        let payload =
            relation_list_payload(&raw_relation_page(), "粉丝列表").expect("解析关系列表");
        let page: UserRelationList = serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(page.total, Some(1));
        assert_eq!(page.has_more, Some(true), "上游给 1 也收成布尔");
        assert_eq!(page.last_pos.as_deref(), Some("pos-1"));
        assert_eq!(page.lock_flag, Some(0));
        let users = page.users.expect("有用户");
        assert_eq!(users[0].mid.as_deref(), Some("0025NhlN2yWrP4"));
        assert_eq!(users[0].enc_uin.as_deref(), Some("7eEFNeSlNKns"));
        assert_eq!(users[0].name.as_deref(), Some("甲"));
        assert_eq!(users[0].desc.as_deref(), Some("这个人很懒"));
        assert_eq!(
            users[0].avatar_url.as_deref(),
            Some("https://y.gtimg.cn/avatar.jpg")
        );
        assert_eq!(users[0].fan_num, Some(12));
        assert_eq!(users[0].is_follow, Some(true));
        assert_eq!(payload["hasMore"], true);
    }

    #[test]
    fn a_missing_list_key_is_a_shape_fault_but_an_empty_list_is_an_answer() {
        // 键在而空：这一页没有行（账号确实没有，或页码已翻过末尾）——是答案。
        let page: UserRelationList = serde_json::from_value(
            relation_list_payload(&json!({ "List": [], "Total": 0 }), "粉丝列表").unwrap(),
        )
        .expect("解析空页");
        assert!(page.users.expect("有 users 字段").is_empty());
        assert_eq!(page.total, Some(0));

        // 键不在 / 为 null：形状不对（账号标识不对、会话过期），报错而不是回空列表。
        let error =
            relation_list_payload(&json!({ "Total": 3 }), "粉丝列表").expect_err("缺 List 要报错");
        assert!(error.to_string().contains("List"), "{error}");
        let error = relation_list_payload(&json!({ "List": null }), "粉丝列表")
            .expect_err("List 为 null 也算缺");
        assert!(error.to_string().contains("List"), "{error}");

        // 好友列表的 friends 在参考里必填：同样判据。
        let list: UserFriendList =
            serde_json::from_value(friend_list_payload(&raw_friend_page()).unwrap())
                .expect("解析好友列表");
        assert_eq!(list.has_more, Some(false));
        let friend = &list.friends.expect("有好友")[0];
        assert_eq!(friend.encrypt_uin.as_deref(), Some("euin-1"));
        assert_eq!(friend.user_name.as_deref(), Some("乙"));
        assert_eq!(
            friend.avatar_url.as_deref(),
            Some("https://y.gtimg.cn/friend.jpg")
        );
        assert_eq!(friend.is_follow, Some(false));
        let error =
            friend_list_payload(&json!({ "HasMore": false })).expect_err("缺 Friends 要报错");
        assert!(error.to_string().contains("Friends"), "{error}");
    }

    #[test]
    fn a_created_playlist_page_decodes_its_summaries() {
        let payload = created_songlists_payload(&raw_created_playlists()).expect("解析创建的歌单");
        let response: UserCreatedSonglistResponse =
            serde_json::from_value(payload.clone()).expect("模型解析");
        assert_eq!(response.total, Some(2));
        assert_eq!(response.finished, Some(true), "上游键是 bFinish");
        assert_eq!(response.deleted_ids.as_deref(), Some(&[1, 2][..]));
        let playlists = response.playlists.expect("有歌单");
        assert_eq!(playlists[0].id, Some(9578424174), "参考的 id 别名含 tid");
        assert_eq!(playlists[0].title.as_deref(), Some("我的歌单"));
        assert_eq!(playlists[0].songnum, Some(3));
        assert_eq!(playlists[0].listennum, Some(5));
        assert_eq!(
            playlists[0].create_time,
            Some(1_600_000_000),
            "上游键是 createTime"
        );
        assert_eq!(playlists[0].uin.as_deref(), Some("2651932936"));
        assert_eq!(playlists[0].nick.as_deref(), Some("甲"));
        assert_eq!(playlists[0].create_fav_cnt, Some(2), "上游键是 fav_cnt");
        assert_eq!(playlists[0].invalid, Some(false));
        assert_eq!(playlists[0].sort_weight, Some(9));
        assert_eq!(
            payload["playlists"][0]["picurl"],
            "https://y.gtimg.cn/music/photo_new/T002R300x300M0000041WVfh2vtlJE.jpg",
            "封面一律 https"
        );
        assert_eq!(payload["playlists"][0]["createTime"], 1_600_000_000u64);

        // 键在而空是答案（这个账号没有创建过歌单）；键不在是形状故障。
        let empty: UserCreatedSonglistResponse = serde_json::from_value(
            created_songlists_payload(&json!({ "v_playlist": [], "total": 0 })).unwrap(),
        )
        .expect("解析空表");
        assert!(empty.playlists.expect("有 playlists 字段").is_empty());
        let error =
            created_songlists_payload(&json!({ "total": 0 })).expect_err("缺 v_playlist 要报错");
        assert!(error.to_string().contains("v_playlist"), "{error}");
    }

    #[test]
    fn an_account_handle_is_the_given_one_or_the_credentials() {
        let upstream = Upstream::new();
        let credential = Credential {
            encrypted_uin: "EUIN".into(),
            ..Credential::default()
        };
        // 给了非空 euin 就用它；空白 euin 按没给，退回凭据里的 `encrypt_uin`。
        assert_eq!(
            resolved_euin(&upstream, &credential, &json!({ "euin": "GIVEN" })).unwrap(),
            "GIVEN"
        );
        assert_eq!(
            resolved_euin(&upstream, &credential, &json!({ "euin": "  " })).unwrap(),
            "EUIN"
        );
        assert_eq!(
            resolved_euin(&upstream, &credential, &json!({})).unwrap(),
            "EUIN",
            "凭据里有 encrypt_uin 时不需要再问上游"
        );
    }

    #[test]
    fn a_numeric_uin_falls_back_to_the_credentials_music_id() {
        let credential = Credential {
            music_id: "2651932936".into(),
            ..Credential::default()
        };
        assert_eq!(
            resolved_uin(&credential, &json!({ "uin": 42 })).unwrap(),
            "42"
        );
        assert_eq!(
            resolved_uin(&credential, &json!({ "uin": "42" })).unwrap(),
            "42",
            "数字字符串也收"
        );
        assert_eq!(resolved_uin(&credential, &json!({})).unwrap(), "2651932936");
        assert_eq!(
            resolved_uin(&credential, &json!({ "uin": 0 })).unwrap(),
            "2651932936",
            "0 按没给"
        );
        let error =
            resolved_uin(&Credential::default(), &json!({})).expect_err("没有账号标识要报错");
        assert!(error.to_string().contains("凭据"), "{error}");
    }

    #[test]
    fn the_homepage_falls_back_to_the_reference_placeholder_credential() {
        let real = Credential {
            music_id: "2651932936".into(),
            music_key: "W_X_key".into(),
            ..Credential::default()
        };
        let chosen = homepage_credential(&real);
        assert_eq!(chosen.music_id, "2651932936", "登录了就用自己的凭据");

        let placeholder = homepage_credential(&Credential::default());
        assert_eq!(placeholder.music_id, "1", "参考的 musicid");
        assert_eq!(placeholder.music_key, "placeholder-musickey");
        assert_eq!(
            placeholder.encrypted_uin, "00000000000000000000000000000000",
            "参考的 encryptUin 是全 0"
        );
        assert_eq!(placeholder.raw["loginType"], 1, "参考的占位登录类型");
        assert!(placeholder.is_usable(), "占位凭证本身就是可用形状");
    }

    #[test]
    fn the_account_reads_refuse_without_a_login_before_any_request() {
        let upstream = Upstream::new();
        let credential = Credential::default();
        for method in [
            "fetch_vip_info",
            "fetch_follow_singers",
            "fetch_fans",
            "fetch_friends",
            "fetch_followed_users",
        ] {
            let error = dispatch(&upstream, &credential, Platform::Web, method, &json!({}))
                .expect("是本层方法")
                .expect_err("未登录要报错");
            assert!(error.to_string().contains("需要登录"), "{method}: {error}");
        }
        // 创建的歌单不需要登录，但没给 uin、凭据里也没有数字 uin 时要报凭据问题，
        // 而不是拿空串去发请求。
        let error = dispatch(
            &upstream,
            &credential,
            Platform::Web,
            "fetch_created_playlists",
            &json!({}),
        )
        .expect("是本层方法")
        .expect_err("没有账号标识要报错");
        assert!(error.to_string().contains("凭据"), "{error}");
    }
}

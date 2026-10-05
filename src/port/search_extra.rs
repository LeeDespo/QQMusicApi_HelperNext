//! 搜索扩展：热词、联想补全、快速搜索、综合搜索，以及既有四类之外的搜索类型。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/search.py`
//! （`get_hotkey` / `complete` / `quick_search` / `general_search` /
//! `search_by_type` 的全部类型），回值字段名照
//! `models/search.py` 的同名模型（camelCase）。既有的 `search_songs` 等四个
//! 方法不动，这里补其余类型与另几条搜索路。
//!
//! # 五个端点
//!
//! * 热搜词 `music.musicsearch.HotkeyService / GetHotkeyForQQMusicMobile`；
//! * 联想补全 `music.smartboxCgi.SmartBoxCgi / GetSmartBoxResult`；
//! * 快速搜索 `GET c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg`（参考的
//!   `@http_endpoint`，`key` 是唯一的 query 参数）；
//! * 综合搜索 `music.adaptor.SearchAdaptor / do_search_v2`；
//! * 类型搜索 `music.search.SearchCgiService / DoSearchForQQMusicMobile`。
//!
//! # 平台档案
//!
//! 参考只给 `search_by_type` 标了 `platform=ANDROID`；其余没标，按本层约定
//! 默认 Web。但实测（2026-10-03，真实账号）这两个端点都认**登录过的 android
//! 会话**：`search_by_type` 在 web 档案下回 `meta.sum = 0`（空目录，看起来
//! 像「没这首歌」），`do_search_v2` 更直接——web 档案回 `code 2001` 风控。
//! 所以这两个 CGI 端点在没有 `params.platform` 时都走 Android 档案，与既有
//! `search_songs` 等四个（`methods.rs` 里同样缺省 Android）保持一致；
//! `dispatch` 收到调用方显式传入的 platform 时原样尊重。
//! `get_hotkey` / `complete` / `quick_search` 是公开路，web 档案实测可用，
//! 保持 Web。
//!
//! # 空值判据
//!
//! 五个接口在参考里都没有 `require_login`，读的是公开曲库/公开联想：
//! **空列表是答案**（这个关键词就是没有命中），照常返回，不报错。
//! 「整表读取空即故障」那条规则针对的是账号自己的列表，不适用于这里。
//! 但「列表键不在」与「给了空列表」是两件事：前者形状不对，模型字段保持
//! `None`，宿主可以据此判断这次不是「没有命中」而是响应不成形。
//!
//! # 翻页（参考的 pager 策略；组件只做单次请求，状态由宿主持有）
//!
//! * 综合搜索：参考的 `MultiFieldContinuationStrategy` 按回值续参——下一次的
//!   `searchid` 用回值 `meta.sid`、`page_id` 用 `meta.nextpage`、
//!   `page_start` 用 `meta.nextpage_start`；`nextpage == -1` 即取完；
//! * 类型搜索：页码翻页（`page_num`），`meta.nextpage` 为 `-1` 即取完，总数在
//!   `meta.sum`。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::models::{Singer, Track};
use crate::upstream::{
    first_array, first_object, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "fetch_search_hotkeys",
    "complete_search",
    "quick_search",
    "general_search",
    "search_extra",
];

/// 快速搜索的 URL（参考 `@http_endpoint` 的 `url` 逐字抄）。
const QUICK_SEARCH_URL: &str = "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg";

/// 参考 `SearchType` 的成员值。`search_extra` 的 `search_type` 就按这些数字
/// 解释（0 歌曲 1 歌手 2 专辑 3 歌单 4 MV 7 歌词 8 用户 10 彩铃 15 节目专辑 18 节目）。
const SEARCH_TYPE_SONG: i64 = 0;
const SEARCH_TYPE_SINGER: i64 = 1;
const SEARCH_TYPE_ALBUM: i64 = 2;
const SEARCH_TYPE_SONGLIST: i64 = 3;
const SEARCH_TYPE_MV: i64 = 4;
const SEARCH_TYPE_LYRIC: i64 = 7;
const SEARCH_TYPE_USER: i64 = 8;
const SEARCH_TYPE_RINGTONE: i64 = 10;
const SEARCH_TYPE_AUDIO_ALBUM: i64 = 15;
const SEARCH_TYPE_AUDIO: i64 = 18;

/// 参考 `search_by_type` 的缺省值：每页 10 条、第 1 页、高亮开。
const SEARCH_PAGE_SIZE: i64 = 10;
const SEARCH_DEFAULT_PAGE: i64 = 1;
/// 参考 `general_search` 的缺省值：每页 15 条、第 1 页、高亮开。
const GENERAL_PAGE_SIZE: i64 = 15;
/// 综合搜索写死的 `search_type`（参考逐字写的 100 = 全部类型）。
const GENERAL_SEARCH_TYPE: i64 = 100;

// MARK: - 模型

/// 参考 `Hotkey`：热搜词条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Hotkey {
    /// 热搜词唯一标识.
    pub hotkey_id: Option<String>,
    /// 热搜关键词.
    pub query: Option<String>,
    /// 热搜展示标题.
    pub title: Option<String>,
    /// 热度得分.
    pub score: Option<String>,
    /// 条目类别（上游有时给数字字符串，pydantic 会转成 int）.
    pub kind: Option<i64>,
    /// 条目类型（参考字段名 `type`，Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub type_id: Option<i64>,
    /// 数据来源.
    pub source: Option<i64>,
    /// 是否置顶.
    pub need_top: Option<i64>,
    /// 排序位置.
    pub subpos: Option<i64>,
    /// 关联歌曲类型.
    pub song_type: Option<i64>,
    /// 直接关联的歌曲 ID.
    pub direct_id: Option<i64>,
    /// 跳转标签页（参考缺省 `"0"`）.
    pub jump_tab: Option<String>,
    /// 跳转链接.
    pub jump_url: Option<String>,
    /// 封面图片地址.
    pub cover_pic_url: Option<String>,
    /// 图片地址.
    pub pic_url: Option<String>,
    /// 描述文案.
    pub description: Option<String>,
}

/// 参考 `HotkeyResponse`：热搜词列表响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyResponse {
    /// 返回码.
    pub ret_code: Option<i64>,
    /// 热搜时间戳.
    pub hotkey_time: Option<String>,
    /// 歌单 ID.
    pub track_list_id: Option<String>,
    /// 热搜词列表.
    pub vec_hotkey: Option<Vec<Hotkey>>,
    /// 推荐搜索词列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub vec_reckey: Option<String>,
}

/// 参考 `CompleteItem`：搜索补全建议条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompleteItem {
    /// 补全提示文本.
    pub hint: Option<String>,
    /// 高亮后的提示文本（含 `<em>` 标签）.
    pub hint_hilight: Option<String>,
    /// docid.
    pub docid: Option<String>,
    /// 条目类型（参考字段名 `type`，Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub type_id: Option<i64>,
    /// 结果类型.
    pub res_type: Option<String>,
    /// 匹配得分（上游有时给整数）。
    pub score: Option<f64>,
    /// 是否直接发起搜索.
    pub pre_search: Option<bool>,
    /// 图标地址.
    pub icon: Option<String>,
    /// 图标类型.
    pub icon_type: Option<i64>,
    /// 跳转标签页（参考缺省 -1）.
    pub jumptab: Option<i64>,
    /// 跳转类型.
    pub jump_type: Option<i64>,
    /// 跳转链接.
    pub jump_url: Option<String>,
    /// 图片地址.
    pub pic_url: Option<String>,
}

/// 参考 `CompleteResponse`：搜索词补全响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompleteResponse {
    /// 补全建议条目列表.
    pub items: Option<Vec<CompleteItem>>,
    /// 补全结果总数.
    pub total_num: Option<i64>,
    /// 搜索会话 ID.
    pub search_id: Option<String>,
    /// 过期时间戳.
    pub expire_time: Option<i64>,
    /// 是否使用默认搜索词.
    pub use_default_search: Option<i64>,
    /// 调试信息.
    pub debug_info: Option<String>,
    /// 实验 ID.
    pub expid: Option<String>,
    /// 历史搜索词列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub history_items: Option<String>,
    /// 直达结果列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub vec_direct_items: Option<String>,
    /// 相关搜索词列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub vec_related_items: Option<String>,
}

/// 参考 `QuickSearchItem`：快速搜索条目.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuickSearchItem {
    /// docid.
    pub docid: Option<String>,
    /// 条目 ID.
    pub id: Option<String>,
    /// 条目 MID.
    pub mid: Option<String>,
    /// 名称.
    pub name: Option<String>,
    /// 歌手名称.
    pub singer: Option<String>,
    /// 封面图片地址.
    pub pic: Option<String>,
    /// MV ID.
    pub vid: Option<String>,
}

/// 参考 `QuickSearchCategory`：快速搜索分类.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuickSearchCategory {
    /// 命中数量.
    pub count: Option<i64>,
    /// 条目列表.
    pub itemlist: Option<Vec<QuickSearchItem>>,
    /// 分类名称.
    pub name: Option<String>,
    /// 排序权重.
    pub order: Option<i64>,
    /// 分类类型（参考字段名 `type`，Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub type_id: Option<i64>,
}

/// 参考 `QuickSearchResponse`：快速搜索响应.
///
/// 参考模型的四个分类都用 jsonpath `$.data.<name>` 从 `data` 里取；这里在
/// 搬运时就展开到顶层，字段名与参考一致（`song` / `singer` / `album` / `mv`）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuickSearchResponse {
    /// 单曲结果.
    pub song: Option<QuickSearchCategory>,
    /// 歌手结果.
    pub singer: Option<QuickSearchCategory>,
    /// 专辑结果.
    pub album: Option<QuickSearchCategory>,
    /// MV 结果.
    pub mv: Option<QuickSearchCategory>,
}

/// 参考 `SearchSelector`：搜索筛选器选项.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchSelector {
    /// 选项 ID.
    pub id: Option<i64>,
    /// 选项名称.
    pub name: Option<String>,
    /// 选项类型（参考字段名 `type`，Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub type_id: Option<i64>,
}

/// 参考搜索里的歌手条目（`SingerSearch`，继承基础模型 `Singer`）：基础字段 + 搜索附加字段.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerSearchItem {
    /// 歌手数字 ID.
    pub id: Option<i64>,
    /// 歌手 Media MID.
    pub mid: Option<String>,
    /// 歌手名称.
    pub name: Option<String>,
    /// 歌手展示标题（参考回退到名称）.
    pub title: Option<String>,
    /// 歌手类型（参考字段名 `type`，别名 `SingerType`/`vt`；Rust 侧换个名字）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// 与歌手关联的用户 ID.
    pub uin: Option<i64>,
    /// 图片 Media ID.
    pub pmid: Option<String>,
    /// 歌手头像地址（参考别名 `singerPic`）.
    pub pic: Option<String>,
    /// 歌曲数量（参考别名 `songNum`）.
    pub song_num: Option<i64>,
    /// 专辑数量（参考别名 `albumNum`）.
    pub album_num: Option<i64>,
    /// MV 数量（参考别名 `mvNum`）.
    pub mv_num: Option<i64>,
    /// 补充描述文案.
    pub subtitle: Option<String>,
}

/// 参考搜索里的专辑条目（`AlbumSearch`，继承基础模型 `Album`）.
///
/// 参考模型里的 `desc_detail` / `hotness` / `label_new` / `audio_play` 是
/// `dict`，`singer_list` 是歌手列表——前四个在本层存 JSON 文本（见
/// `raw_json_serialize`），写出去仍是对象。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSearchItem {
    /// 专辑数字 ID（参考别名 `albumID`）.
    pub id: Option<i64>,
    /// 专辑 Media MID（参考别名 `albumMid`/`albumMID`/`albummid`）.
    pub mid: Option<String>,
    /// 专辑名称.
    pub name: Option<String>,
    /// 专辑展示标题.
    pub title: Option<String>,
    /// 专辑副标题（参考别名 `albumTranName`）.
    pub subtitle: Option<String>,
    /// 发行日期（参考别名 `publish_date`/`publishDate`）.
    pub time_public: Option<String>,
    /// 图片 Media ID（参考别名 `logo`）.
    pub pmid: Option<String>,
    /// 专辑类型（参考 jsonpath `$.core_album_config.album_type`；JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// 勋章文案（参考 jsonpath `$.core_album_config.award_label`）.
    pub award_label: Option<String>,
    /// 详尽描述对象（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub desc_detail: Option<String>,
    /// 简短描述文案.
    pub description: Option<String>,
    /// 备用描述文案.
    pub description2: Option<String>,
    /// 热度数据对象（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub hotness: Option<String>,
    /// 热度简述文案.
    pub hotness_desc: Option<String>,
    /// 专辑关联的特性标签对象（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub label_new: Option<String>,
    /// 播放排行信息（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub audio_play: Option<String>,
    /// 专辑封面地址.
    pub pic: Option<String>,
    /// 封面配套的勋章/类型图标.
    pub pic_icon: Option<String>,
    /// 搜索命中的高亮歌手名称.
    pub singer: Option<String>,
    /// 结构化的歌手对象列表（复用组件既有的歌手模型）.
    pub singer_list: Option<Vec<Singer>>,
    /// 专辑标签列表（参考声明为 `list[str]`）.
    pub tag_list: Option<Vec<String>>,
    /// 静态元数据下载链接.
    pub url: Option<String>,
}

/// 参考搜索里的歌单条目（`SongListSearch`，继承基础模型 `SongList`）.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongListSearchItem {
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
    /// 歌单创建者昵称.
    pub nickname: Option<String>,
    /// 歌单目录类型标识.
    pub dirtype: Option<i64>,
}

/// 参考搜索里的 MV 条目（`MvSearch`，继承基础模型 `MV`）.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvSearchItem {
    /// MV 数字 ID（参考别名 `sid`/`mvid`/`singerId`）.
    pub id: Option<i64>,
    /// MV VID.
    pub vid: Option<String>,
    /// MV 类型（参考别名 `vt`；JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// MV 名称（参考别名 `mvname`/`title`）.
    pub name: Option<String>,
    /// MV 展示标题（参考别名 `title_main`/`name`）.
    pub title: Option<String>,
    /// MV 封面地址.
    pub pic: Option<String>,
    /// MV 播放量（参考别名 `play_count`）.
    pub play_count: Option<i64>,
    /// MV 时长.
    pub duration: Option<i64>,
    /// 发布时间（参考别名 `publish_date`）.
    pub publish_date: Option<String>,
    /// 歌手 ID（参考别名 `singerid`）.
    pub singer_id: Option<i64>,
    /// 歌手 MID（参考别名 `singermid`）.
    pub singer_mid: Option<String>,
    /// 歌手名称（参考别名 `singername`）.
    pub singer_name: Option<String>,
}

/// 参考搜索里的歌曲条目（`SongSearch`，继承基础模型 `Song`）：曲目本体 +
/// 搜索场景附加字段.
///
/// 前一组字段与组件既有的 [`crate::models::Track`] 同源（同一个曲目解码），
/// 后一组是参考 `SongSearch` 自己的（`search_title` / `hotness` / …）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongSearchItem {
    /// 歌曲数字 ID.
    pub song_id: Option<i64>,
    /// 歌曲 Media MID.
    pub song_mid: Option<String>,
    /// 基础媒体标识符.
    pub media_mid: Option<String>,
    /// 歌曲名称.
    pub title: Option<String>,
    /// 全部歌手名，用 `", "` 连接.
    pub artist: Option<String>,
    /// 专辑名称.
    pub album: Option<String>,
    /// 专辑 Media MID.
    pub album_mid: Option<String>,
    /// 专辑数字 ID.
    pub album_id: Option<i64>,
    /// 封面地址（上游键 `imageURL`）.
    #[serde(rename = "imageURL")]
    pub image_url: Option<String>,
    /// 时长（秒）.
    pub duration: Option<i64>,
    /// 播放付费标识.
    pub pay_play: Option<i64>,
    /// 首位歌手的 Media MID.
    pub singer_mid: Option<String>,
    /// 全部歌手.
    pub singers: Option<Vec<Singer>>,
    /// 搜索命中的标题（可能含 `<em>` 高亮标签）.
    pub search_title: Option<String>,
    /// 歌曲主标题.
    pub title_main: Option<String>,
    /// 歌曲附加标题.
    pub title_extra: Option<String>,
    /// 收藏数展示文案.
    pub fav_show: Option<String>,
    /// 歌曲描述文案.
    pub desc: Option<String>,
    /// 描述文案前的图标链接.
    pub desc_icon: Option<String>,
    /// 搜索结果内容摘要（命中歌词/评论时的片段）.
    pub content: Option<String>,
    /// 热度数据对象（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub hotness: Option<String>,
    /// 热度描述（如榜单名）.
    pub hotness_desc: Option<String>,
    /// 热度榜单详情列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub vec_hotness: Option<String>,
    /// 新版状态位（2: 正常）.
    pub new_status: Option<i64>,
    /// 是否受到版权保护.
    pub protect: Option<i64>,
    /// 相关搜索词推荐组（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub relatedword_group: Option<String>,
}

/// 参考 `GeneralSearchRequestBody[SongSearch]`：综合搜索的单曲结果容器.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongSearchBucket {
    /// 搜索命中的预估总记录数.
    pub estimate_sum: Option<i64>,
    /// 搜索命中的确切总记录数.
    pub total_num: Option<i64>,
    /// 当前分类下已展开的曲目.
    pub items: Option<Vec<SongSearchItem>>,
    /// 继续加载该分类结果时需要回传的翻页上下文（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more_info: Option<String>,
}

/// 参考 `GeneralSearchRequestBody[SingerSearch]`：综合搜索的歌手结果容器.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SingerSearchBucket {
    pub estimate_sum: Option<i64>,
    pub total_num: Option<i64>,
    pub items: Option<Vec<SingerSearchItem>>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more_info: Option<String>,
}

/// 参考 `GeneralSearchRequestBody[AlbumSearch]`：综合搜索的专辑/节目结果容器.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSearchBucket {
    pub estimate_sum: Option<i64>,
    pub total_num: Option<i64>,
    pub items: Option<Vec<AlbumSearchItem>>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more_info: Option<String>,
}

/// 参考 `GeneralSearchRequestBody[SongListSearch]`：综合搜索的歌单结果容器.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongListSearchBucket {
    pub estimate_sum: Option<i64>,
    pub total_num: Option<i64>,
    pub items: Option<Vec<SongListSearchItem>>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more_info: Option<String>,
}

/// 参考 `GeneralSearchRequestBody[MvSearch]`：综合搜索的 MV 结果容器.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MvSearchBucket {
    pub estimate_sum: Option<i64>,
    pub total_num: Option<i64>,
    pub items: Option<Vec<MvSearchItem>>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more_info: Option<String>,
}

/// 参考 `GeneralSearchRequestBody[RelatedSearchWord]`：相关搜索词结果容器.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelatedSearchBucket {
    pub estimate_sum: Option<i64>,
    pub total_num: Option<i64>,
    pub items: Option<Vec<RelatedSearchWord>>,
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub more_info: Option<String>,
}

/// 参考 `RelatedSearchWord`：相关搜索词推荐.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelatedSearchWord {
    /// 相关搜索词展示文案.
    pub display: Option<String>,
    /// 相关搜索词实际搜索关键词.
    pub search: Option<String>,
}

/// 参考 `GeneralSearchResponse`：综合搜索响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSearchResponse {
    /// 搜索会话 ID（上游 `meta.sid`）.
    pub searchid: Option<String>,
    /// 每页结果数量.
    pub perpage: Option<i64>,
    /// 下一页页码，-1 表示已加载全部结果.
    pub nextpage: Option<i64>,
    /// 综合搜索继续翻页的关键参数（参考声明为 `dict`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub nextpage_start: Option<String>,
    /// 单曲结果容器（曲目复用组件既有的曲目模型）.
    pub song: Option<SongSearchBucket>,
    /// 歌手结果容器.
    pub singer: Option<SingerSearchBucket>,
    /// MV 结果容器.
    pub mv: Option<MvSearchBucket>,
    /// 专辑结果容器.
    pub album: Option<AlbumSearchBucket>,
    /// 歌单结果容器.
    pub songlist: Option<SongListSearchBucket>,
    /// 节目结果容器.
    pub audio: Option<AlbumSearchBucket>,
    /// 直接命中结果分组（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub direct: Option<String>,
    /// 相关搜索词推荐结果容器.
    pub related: Option<RelatedSearchBucket>,
}

/// 参考 `SearchByTypeResponse`：按指定类型搜索时的响应模型.
///
/// 列表里的条目按类型不同：歌曲/歌词/彩铃/节目是曲目模型，歌手/专辑/歌单/MV
/// 各是各自的基础模型，用户是原样透传的上游形状（JSON 文本）。参考对每个类型
/// 各写一遍，这里的字段与参考同名。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchByTypeResponse {
    /// 搜索会话 ID，用于后续相关请求（上游 `meta.searchid`）.
    pub searchid: Option<String>,
    /// 每页结果数量.
    pub perpage: Option<i64>,
    /// 下一页页码，-1 表示已加载全部结果.
    pub nextpage: Option<i64>,
    /// 搜索命中的预估总记录数.
    pub estimate_sum: Option<i64>,
    /// 搜索命中的确切总记录数（上游 `meta.sum`）.
    pub total_num: Option<i64>,
    /// 单曲、歌词或节目类型下的结果列表.
    pub song: Option<Vec<Track>>,
    /// 歌手结果列表.
    pub singer: Option<Vec<SingerSearchItem>>,
    /// 专辑结果列表.
    pub album: Option<Vec<AlbumSearchItem>>,
    /// 歌单结果列表.
    pub songlist: Option<Vec<SongListSearchItem>>,
    /// 用户结果列表（参考声明为 `list[dict]`，原样透传）.
    #[serde(
        default,
        serialize_with = "raw_json_serialize",
        deserialize_with = "raw_json_deserialize"
    )]
    pub user: Option<String>,
    /// 节目专辑结果列表.
    pub audio_alum: Option<Vec<AlbumSearchItem>>,
    /// MV 结果列表.
    pub mv: Option<Vec<MvSearchItem>>,
    /// 搜索筛选器列表（上游 `body.multi_extern_info.selectors`，每组一个数组）.
    pub selectors: Option<Vec<Vec<SearchSelector>>>,
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

/// 取第一个出现的键，原样保留（不透传形状的字段用）。
fn raw_field(raw: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .filter(|found| !found.is_null())
        .cloned()
}

/// 宽松的整数：上游给数字也给数字字符串（pydantic 的 `int` 同样会转）。
fn coerce_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 一次键查找的宽松整数（`coerce_int` 的候选键版）。
fn int_of(raw: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .and_then(coerce_int)
}

/// 宽松的浮点：上游的 `score` 有时给整数、有时给小数、有时给字符串。
fn number_of(raw: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| match raw.get(*key) {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
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

/// 封面 / 头像类 URL 一律转 https（docs/parsing.md §7：宿主会直接拒绝 http 图片）。
fn artwork(value: Option<String>) -> Option<String> {
    crate::methods::normalized_artwork_url(value.as_deref())
}

// MARK: - 回值搬运（上游形状 → 参考模型形状）

/// 单个热搜词（参考 `Hotkey`）。
fn hotkey_payload(raw: &Value) -> Value {
    json!({
        "hotkeyId": first_text(raw, &["hotkey_id", "hotkeyId"]),
        "query": first_text(raw, &["query"]),
        "title": first_text(raw, &["title"]),
        "score": first_text(raw, &["score"]),
        "kind": int_of(raw, &["kind"]),
        "type": int_of(raw, &["type"]),
        "source": int_of(raw, &["source"]),
        "needTop": int_of(raw, &["need_top", "needTop"]),
        "subpos": int_of(raw, &["subpos"]),
        "songType": int_of(raw, &["song_type", "songType"]),
        "directId": int_of(raw, &["direct_id", "directId"]),
        "jumpTab": first_text(raw, &["jump_tab", "jumpTab"]),
        "jumpUrl": first_text(raw, &["jump_url", "jumpUrl"]),
        "coverPicUrl": artwork(first_text(raw, &["cover_pic_url", "coverPicUrl"])),
        "picUrl": artwork(first_text(raw, &["pic_url", "picUrl"])),
        "description": first_text(raw, &["description"]),
    })
}

/// 热搜词回值（参考 `HotkeyResponse`）。
fn hotkeys_payload(data: &Value) -> Value {
    json!({
        "retCode": int_of(data, &["ret_code", "retCode"]),
        "hotkeyTime": first_text(data, &["hotkey_time", "hotkeyTime"]),
        "trackListId": first_text(data, &["track_list_id", "trackListId"]),
        // 列表键在不在与「给了空列表」是两件事：键不在时保持 None。
        "vecHotkey": first_array(data, &["vec_hotkey", "vecHotkey"])
            .map(|items| items.iter().map(hotkey_payload).collect::<Vec<_>>()),
        // 参考的 `vec_reckey` 缺省是空列表（`default_factory=list`）。
        "vecReckey": raw_field(data, &["vec_reckey", "vecReckey"]).unwrap_or(json!([])),
    })
}

/// 单条补全建议（参考 `CompleteItem`）。
fn complete_item_payload(raw: &Value) -> Value {
    json!({
        "hint": first_text(raw, &["hint"]),
        "hintHilight": first_text(raw, &["hint_hilight", "hintHilight"]),
        "docid": first_text(raw, &["docid"]),
        "type": int_of(raw, &["type"]),
        "resType": first_text(raw, &["res_type", "resType"]),
        "score": number_of(raw, &["score"]),
        "preSearch": bool_field(raw, &["pre_search", "preSearch"]),
        "icon": first_text(raw, &["icon"]),
        "iconType": int_of(raw, &["icon_type", "iconType"]),
        "jumptab": int_of(raw, &["jumptab"]),
        "jumpType": int_of(raw, &["jump_type", "jumpType"]),
        "jumpUrl": first_text(raw, &["jump_url", "jumpUrl"]),
        "picUrl": artwork(first_text(raw, &["pic_url", "picUrl"])),
    })
}

/// 补全回值（参考 `CompleteResponse`）。
fn complete_payload(data: &Value) -> Value {
    json!({
        "items": first_array(data, &["items"])
            .map(|items| items.iter().map(complete_item_payload).collect::<Vec<_>>()),
        "totalNum": int_of(data, &["total_num", "totalNum"]),
        "searchId": first_text(data, &["search_id", "searchId"]),
        "expireTime": int_of(data, &["expire_time", "expireTime"]),
        "useDefaultSearch": int_of(data, &["use_default_search", "useDefaultSearch"]),
        "debugInfo": first_text(data, &["debug_info", "debugInfo"]),
        "expid": first_text(data, &["expid"]),
        // 参考里这三个的缺省都是空列表（`default_factory=list`）。
        "historyItems": raw_field(data, &["history_items", "historyItems"]).unwrap_or(json!([])),
        "vecDirectItems": raw_field(data, &["vec_direct_items", "vecDirectItems"])
            .unwrap_or(json!([])),
        "vecRelatedItems": raw_field(data, &["vec_related_items", "vecRelatedItems"])
            .unwrap_or(json!([])),
    })
}

/// 快速搜索的单个条目（参考 `QuickSearchItem`）。
fn quick_item_payload(raw: &Value) -> Value {
    json!({
        "docid": first_text(raw, &["docid"]),
        "id": first_text(raw, &["id"]),
        "mid": first_text(raw, &["mid"]),
        "name": first_text(raw, &["name"]),
        "singer": first_text(raw, &["singer"]),
        "pic": artwork(first_text(raw, &["pic"])),
        "vid": first_text(raw, &["vid"]),
    })
}

/// 快速搜索的单个分类（参考 `QuickSearchCategory`）。
fn quick_category_payload(raw: &Value) -> Value {
    json!({
        "count": int_of(raw, &["count"]),
        "itemlist": first_array(raw, &["itemlist"])
            .map(|items| items.iter().map(quick_item_payload).collect::<Vec<_>>()),
        "name": first_text(raw, &["name"]),
        "order": int_of(raw, &["order"]),
        "type": int_of(raw, &["type"]),
    })
}

/// 快速搜索回值（参考 `QuickSearchResponse`）：四个分类都在 `data` 上（参考模型
/// 的 jsonpath 就是 `$.data.<name>`），这里在搬运时展开到顶层。
fn quick_search_payload(response: &Value) -> Value {
    // 有些形状把结果再包一层 `data`；命中就往下走，否则顶层就是 `data` 的内容。
    let data = first_object(response, &["data"]).unwrap_or(response);
    json!({
        "song": first_object(data, &["song"]).map(quick_category_payload),
        "singer": first_object(data, &["singer"]).map(quick_category_payload),
        "album": first_object(data, &["album"]).map(quick_category_payload),
        "mv": first_object(data, &["mv"]).map(quick_category_payload),
    })
}

/// 搜索筛选器（参考 `SearchSelector`）。
fn selector_payload(raw: &Value) -> Value {
    json!({
        "id": int_of(raw, &["id"]),
        "name": first_text(raw, &["name"]),
        "type": int_of(raw, &["type"]),
    })
}

/// 一组筛选器分组（上游 `multi_extern_info.selectors` 是「组数组的数组」，
/// 参考的 `list[list[SearchSelector]]` 也是这个形状）。
fn selector_groups(data: &Value) -> Option<Vec<Vec<Value>>> {
    let groups = first_object(data, &["multi_extern_info", "multiExternInfo"])
        .and_then(|info| first_array(info, &["selectors"]))?;
    Some(
        groups
            .iter()
            .map(|group| {
                group
                    .as_array()
                    .map(|items| items.iter().map(selector_payload).collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect(),
    )
}

/// 类型搜索里的单曲条目：参考的 `SongSearch`（继承 `Song`）多出一批搜索字段，
/// 曲目本体复用组件既有的解码（`methods::decoded_tracks`），再补上这些字段。
fn song_search_payload(raw: &Value) -> Option<Value> {
    let mut track = crate::methods::decoded_tracks(&json!({ "list": [raw] }))
        .into_iter()
        .next()?;
    let object = track.as_object_mut()?;
    object.insert(
        "searchTitle".into(),
        json!(first_text(raw, &["search_title", "searchTitle"])),
    );
    object.insert(
        "titleMain".into(),
        json!(first_text(raw, &["title_main", "titleMain"])),
    );
    object.insert(
        "titleExtra".into(),
        json!(first_text(raw, &["title_extra", "titleExtra"])),
    );
    object.insert(
        "favShow".into(),
        json!(first_text(raw, &["fav_show", "favShow"])),
    );
    object.insert("desc".into(), json!(first_text(raw, &["desc"])));
    object.insert(
        "descIcon".into(),
        json!(first_text(raw, &["desc_icon", "descIcon"])),
    );
    object.insert("content".into(), json!(first_text(raw, &["content"])));
    object.insert(
        "hotnessDesc".into(),
        json!(first_text(raw, &["hotness_desc", "hotnessDesc"])),
    );
    object.insert(
        "newStatus".into(),
        json!(int_of(raw, &["newStatus", "new_status"])),
    );
    object.insert("protect".into(), json!(int_of(raw, &["protect"])));
    // 参考的 `hotness` / `relatedword_group` 声明为 `dict`、`vec_hotness` 声明为
    // `list[dict]`——都是原样透传的上游形状，一个字段都不加。
    object.insert(
        "hotness".into(),
        raw_field(raw, &["hotness"]).unwrap_or(json!({})),
    );
    object.insert(
        "vecHotness".into(),
        raw_field(raw, &["vec_hotness", "vecHotness"]).unwrap_or(json!([])),
    );
    object.insert(
        "releaseDate".into(),
        json!(first_text(
            raw,
            &["time_public", "timePublic", "publish_date"]
        )),
    );
    object.insert(
        "relatedwordGroup".into(),
        raw_field(raw, &["relatedword_group", "relatedwordGroup"]).unwrap_or(json!({})),
    );
    Some(track)
}

/// 类型搜索里的歌手条目：参考的 `SingerSearch`（继承 `Singer`，别名见参考模型）。
fn singer_search_payload(raw: &Value) -> Value {
    json!({
        "id": int_of(raw, &["id", "singerID", "singerId", "SingerID", "singer_id"]),
        "mid": first_text(raw, &["mid", "singerMid", "singerMID", "SingerMid", "singer_mid"]),
        "name": first_text(raw, &["name", "singerName", "singer_name"]),
        "title": first_text(raw, &["title", "singerName", "name"]),
        "type": int_of(raw, &["type", "SingerType", "vt"]),
        "uin": int_of(raw, &["uin"]),
        "pmid": first_text(raw, &["pmid", "singerPmid", "singer_pmid", "pic_mid"]),
        "pic": artwork(first_text(raw, &["singerPic", "pic"])),
        "songNum": int_of(raw, &["songNum", "songnum"]),
        "albumNum": int_of(raw, &["albumNum", "albumnum"]),
        "mvNum": int_of(raw, &["mvNum", "mvnum"]),
        "subtitle": first_text(raw, &["subtitle"]),
    })
}

/// 类型搜索里的专辑条目：参考的 `AlbumSearch`（继承 `Album`）；`type` 与
/// `award_label` 在参考里走 `$.core_album_config.*`。
fn album_search_payload(raw: &Value) -> Value {
    let core = first_object(raw, &["core_album_config", "coreAlbumConfig"]).unwrap_or(raw);
    json!({
        "id": int_of(raw, &["id", "albumID"]),
        "mid": first_text(raw, &["mid", "albumMid", "albumMID", "albummid"]),
        "name": first_text(raw, &["name", "albumName"]),
        "title": first_text(raw, &["title", "albumName", "name"]),
        "subtitle": first_text(raw, &["subtitle", "albumTranName"]),
        "timePublic": first_text(raw, &["time_public", "publish_date", "publishDate"]),
        "pmid": first_text(raw, &["pmid", "logo"]),
        "type": int_of(core, &["album_type", "albumType"]).or_else(|| int_of(raw, &["type"])),
        "awardLabel": first_text(core, &["award_label", "awardLabel"]),
        "descDetail": raw_field(raw, &["desc_detail", "descDetail"]).unwrap_or(json!({})),
        "description": first_text(raw, &["description"]),
        "description2": first_text(raw, &["description2"]),
        "hotness": raw_field(raw, &["hotness"]).unwrap_or(json!({})),
        "hotnessDesc": first_text(raw, &["hotness_desc", "hotnessDesc"]),
        "labelNew": raw_field(raw, &["label_new", "labelNew"]).unwrap_or(json!({})),
        "audioPlay": raw_field(raw, &["audio_play", "audioPlay"]).unwrap_or(json!({})),
        "pic": artwork(first_text(raw, &["pic"])),
        "picIcon": artwork(first_text(raw, &["pic_icon", "picIcon"])),
        "singer": first_text(raw, &["singer"]),
        "singerList": first_array(raw, &["singer_list", "singerList"])
            .map(|items| {
                items
                    .iter()
                    .map(|item| json!({
                        "mid": first_text(item, &["mid"]),
                        "name": first_text(item, &["name"]),
                    }))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "tagList": first_array(raw, &["tag_list", "tagList"])
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "url": first_text(raw, &["url"]),
    })
}

/// 类型搜索里的歌单条目：参考的 `SongListSearch`（继承 `SongList`，别名见参考模型）。
fn songlist_search_payload(raw: &Value) -> Value {
    json!({
        "id": int_of(raw, &["id", "tid", "dissid", "dissId"]),
        "dirid": int_of(raw, &["dirid", "dirId"]),
        "title": first_text(raw, &["title", "dissname", "name", "dirName"]),
        "picurl": artwork(first_text(raw, &["picurl", "cover", "logo", "picUrl"])),
        "desc": first_text(raw, &["desc", "description"]),
        "songnum": int_of(raw, &["songnum", "songNum", "song_cnt"]),
        "listennum": int_of(raw, &["listennum", "playCnt", "play_cnt"]),
        "nickname": first_text(raw, &["nickname"]),
        "dirtype": int_of(raw, &["dirtype"]),
    })
}

/// 类型搜索里的 MV 条目：参考的 `MvSearch`（继承 `MV`，别名见参考模型）。
fn mv_search_payload(raw: &Value) -> Value {
    json!({
        // 上游的 `id` 是数字字符串；参考模型声明成 `int`，收不进数字就留空。
        "id": int_of(raw, &["id", "sid", "mvid", "singerId"]),
        "vid": first_text(raw, &["vid"]),
        "type": int_of(raw, &["vt", "type"]),
        "name": first_text(raw, &["name", "mvname", "title"]),
        "title": first_text(raw, &["title", "title_main", "name"]),
        "pic": artwork(first_text(raw, &["pic"])),
        "playCount": int_of(raw, &["play_count", "playCount"]),
        "duration": int_of(raw, &["duration"]),
        "publishDate": first_text(raw, &["publish_date", "publishDate"]),
        "singerId": int_of(raw, &["singerid", "singerId"]),
        "singerMid": first_text(raw, &["singermid", "singerMid"]),
        "singerName": first_text(raw, &["singername", "singerName"]),
    })
}

/// 类型搜索回值（参考 `SearchByTypeResponse`）：元信息在 `meta` 上、列表在
/// `body` 上（参考模型的 jsonpath 就是这么分的）。
fn search_by_type_payload(data: &Value) -> Value {
    let body = first_object(data, &["body"]).unwrap_or(data);
    json!({
        "searchid": first_object(data, &["meta"])
            .and_then(|meta| first_text(meta, &["searchid", "sid"])),
        "perpage": first_object(data, &["meta"])
            .and_then(|meta| int_of(meta, &["perpage"])),
        "nextpage": first_object(data, &["meta"])
            .and_then(|meta| int_of(meta, &["nextpage"])),
        "estimateSum": first_object(data, &["meta"])
            .and_then(|meta| int_of(meta, &["estimate_sum", "estimateSum"])),
        "totalNum": first_object(data, &["meta"])
            .and_then(|meta| int_of(meta, &["sum", "total"])),
        "song": first_array(body, &["item_song", "itemSong"])
            .map(|items| items.iter().filter_map(song_search_payload).collect::<Vec<_>>()),
        "singer": first_array(body, &["singer"])
            .map(|items| items.iter().map(singer_search_payload).collect::<Vec<_>>()),
        "album": first_array(body, &["item_album", "itemAlbum"])
            .map(|items| items.iter().map(album_search_payload).collect::<Vec<_>>()),
        "songlist": first_array(body, &["item_songlist", "itemSonglist"])
            .map(|items| items.iter().map(songlist_search_payload).collect::<Vec<_>>()),
        // 用户是原样透传的上游形状（参考的 `list[dict]`）。
        "user": first_array(body, &["item_user", "itemUser"])
            .map(|items| Value::Array(items.clone())),
        "audioAlum": first_array(body, &["item_audio", "itemAudio"])
            .map(|items| items.iter().map(album_search_payload).collect::<Vec<_>>()),
        "mv": first_array(body, &["item_mv", "itemMv"])
            .map(|items| items.iter().map(mv_search_payload).collect::<Vec<_>>()),
        "selectors": selector_groups(body),
    })
}

/// 综合搜索的单个分类桶（参考 `GeneralSearchRequestBody`）。
fn bucket_payload(raw: &Value, decode: fn(&Value) -> Value) -> Value {
    json!({
        "estimateSum": int_of(raw, &["estimate_sum", "estimateSum"]),
        "totalNum": int_of(raw, &["total_num", "totalNum"]),
        "items": first_array(raw, &["items"]).map(|items| {
            items.iter().map(decode).collect::<Vec<_>>()
        }),
        "moreInfo": raw_field(raw, &["more_info", "moreInfo"]),
    })
}

/// 综合搜索里的曲目条目：参考的 `SongSearch`（继承 `Song`），与类型搜索同一套
/// 搜索字段，这里复用同一个搬运。
fn general_song_payload(raw: &Value) -> Value {
    song_search_payload(raw).unwrap_or_else(|| json!({}))
}

/// 综合搜索里的歌手条目：参考的 `SingerSearch`。
fn general_singer_payload(raw: &Value) -> Value {
    singer_search_payload(raw)
}

/// 综合搜索里的专辑/节目条目：参考的 `AlbumSearch`。
fn general_album_payload(raw: &Value) -> Value {
    album_search_payload(raw)
}

/// 综合搜索里的歌单条目：参考的 `SongListSearch`。
fn general_songlist_payload(raw: &Value) -> Value {
    songlist_search_payload(raw)
}

/// 综合搜索里的 MV 条目：参考的 `MvSearch`。
fn general_mv_payload(raw: &Value) -> Value {
    mv_search_payload(raw)
}

/// 综合搜索里的相关搜索词（参考 `RelatedSearchWord`）。
fn related_word_payload(raw: &Value) -> Value {
    json!({
        "display": first_text(raw, &["display_word", "displayWord", "display"]),
        "search": first_text(raw, &["search_word", "searchWord", "search"]),
    })
}

/// 综合搜索回值（参考 `GeneralSearchResponse`）：`sid` / `nextpage` /
/// `nextpage_start` 都在 `meta` 上，分类在 `body` 上。
fn general_search_payload(data: &Value) -> Value {
    let body = first_object(data, &["body"]).unwrap_or(data);
    let meta = first_object(data, &["meta"]).unwrap_or(data);
    json!({
        "searchid": first_text(meta, &["sid", "searchid"]),
        "perpage": int_of(meta, &["perpage"]),
        "nextpage": int_of(meta, &["nextpage"]),
        "nextpageStart": raw_field(meta, &["nextpage_start", "nextPageStart"])
            .unwrap_or(json!({})),
        "song": first_object(body, &["item_song", "itemSong"])
            .map(|bucket| bucket_payload(bucket, general_song_payload)),
        "singer": first_object(body, &["singer"])
            .map(|bucket| bucket_payload(bucket, general_singer_payload)),
        "mv": first_object(body, &["item_mv", "itemMv"])
            .map(|bucket| bucket_payload(bucket, general_mv_payload)),
        "album": first_object(body, &["item_album", "itemAlbum"])
            .map(|bucket| bucket_payload(bucket, general_album_payload)),
        "songlist": first_object(body, &["item_songlist", "itemSonglist"])
            .map(|bucket| bucket_payload(bucket, general_songlist_payload)),
        "audio": first_object(body, &["item_audio", "itemAudio"])
            .map(|bucket| bucket_payload(bucket, general_album_payload)),
        "direct": first_object(body, &["direct_result", "directResult"])
            .and_then(|direct| raw_field(direct, &["direct_group", "directGroup"]))
            .unwrap_or(json!([])),
        "related": first_object(body, &["item_related", "itemRelated"])
            .map(|bucket| bucket_payload(bucket, related_word_payload)),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 参考的 `get_searchID()`：三段随机/时钟拼出的 19 位会话 ID。
///
/// 上游不校验它的形状，只要求「像那么一个」；既有 `catalog.rs` 里另有一个
/// 生成器（`search_id`），那是 `methods.rs` 的既有实现，本文件不复用它的私有
/// 函数，按参考的公式在这里重写一份。
fn search_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let e: i64 = rng.gen_range(1..=20);
    let t = e * 18_014_398_509_481_984;
    let n: i64 = rng.gen_range(0..=4_194_304) * 4_294_967_296;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    let r = millis % (24 * 60 * 60 * 1000);
    (t + n + r).to_string()
}

/// 调用方给的 `searchid`，没给就现生成一个（参考的 `searchid or get_searchID()`）。
fn searchid_of(params: &Value) -> String {
    first_text(params, &["searchid", "searchId"])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(search_id)
}

/// 调用方给的 `keyword`（三个搜索接口共用）。
fn keyword_of(params: &Value) -> String {
    first_text(params, &["keyword", "query"]).unwrap_or_default()
}

/// 是否高亮：参考的 `highlight: bool = True`，本层显式给了就用调用方的。
fn highlight_of(params: &Value) -> bool {
    bool_field(params, &["highlight"]).unwrap_or(true)
}

/// 补全参数（参考 `complete`）：`num_per_page` 与 `page_idx` 都写死 0。
fn complete_params(keyword: &str) -> Value {
    json!({
        "search_id": search_id(),
        "query": keyword,
        "num_per_page": 0,
        "page_idx": 0,
    })
}

/// 快速搜索的 query（参考 `HttpRequestData(params={"key": keyword})`）。
///
/// URL 只有这一个参数，所以按表单规则在这里编码一次，不用碰 `upstream.rs`
/// 的内部编码函数。
fn quick_search_query(keyword: &str) -> String {
    format!("{QUICK_SEARCH_URL}?key={}", percent_encode(keyword))
}

/// 表单式百分号编码（与 `upstream.rs` 里签名路的编码规则同一套）。
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 综合搜索参数（参考 `general_search`）：`search_type=100` 写死，
/// `page_start` 给了才带（参考的 `if page_start is not None`）。
fn general_params(params: &Value) -> Value {
    let mut param = json!({
        "searchid": searchid_of(params),
        "search_type": GENERAL_SEARCH_TYPE,
        "page_num": int_of(params, &["num", "pageNum", "page_num"]).unwrap_or(GENERAL_PAGE_SIZE),
        "query": keyword_of(params),
        "page_id": int_of(params, &["page", "pageId", "page_id"]).unwrap_or(SEARCH_DEFAULT_PAGE),
        "highlight": highlight_of(params),
        "grp": true,
    });
    if let Some(page_start) = params.get("pageStart").or_else(|| params.get("page_start")) {
        if !page_start.is_null() {
            param["page_start"] = page_start.clone();
        }
    }
    param
}

/// 类型搜索参数（参考 `search_by_type`）：`selectors` 的两份形状照参考逐字抄
/// （`{type: id}` 映射 + `[{type, name, id}]` 数组），没给就都是空。
fn search_type_params(params: &Value) -> Result<Value, UpstreamError> {
    let search_type = int_of(params, &["searchType", "search_type"]).unwrap_or(SEARCH_TYPE_SONG);
    if !matches!(
        search_type,
        SEARCH_TYPE_SONG
            | SEARCH_TYPE_SINGER
            | SEARCH_TYPE_ALBUM
            | SEARCH_TYPE_SONGLIST
            | SEARCH_TYPE_MV
            | SEARCH_TYPE_LYRIC
            | SEARCH_TYPE_USER
            | SEARCH_TYPE_RINGTONE
            | SEARCH_TYPE_AUDIO_ALBUM
            | SEARCH_TYPE_AUDIO
    ) {
        return Err(UpstreamError::Upstream(format!(
            "不支持的 search_type：{search_type}（可用 0/1/2/3/4/7/8/10/15/18）"
        )));
    }
    let selectors = first_array(params, &["selectors"])
        .cloned()
        .unwrap_or_default();
    let mut map = serde_json::Map::new();
    let mut vec_selectors: Vec<Value> = Vec::new();
    for selector in &selectors {
        let name = first_text(selector, &["name"]).unwrap_or_default();
        let kind = int_of(selector, &["type"]);
        let id = int_of(selector, &["id"]);
        if let (Some(kind), Some(id)) = (kind, id) {
            map.insert(kind.to_string(), json!(id.to_string()));
        }
        vec_selectors.push(json!({
            "type": kind,
            "name": name,
            "id": id,
        }));
    }
    Ok(json!({
        "searchid": searchid_of(params),
        "query": keyword_of(params),
        "search_type": search_type,
        "num_per_page": int_of(params, &["num", "numPerPage", "num_per_page"]).unwrap_or(SEARCH_PAGE_SIZE),
        "page_num": int_of(params, &["page", "pageNum", "page_num"]).unwrap_or(SEARCH_DEFAULT_PAGE),
        "highlight": highlight_of(params),
        "grp": true,
        "selectors": Value::Object(map),
        "vec_selectors": vec_selectors,
    }))
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
        "fetch_search_hotkeys" => hotkeys(upstream, credential, platform),
        "complete_search" => complete(upstream, credential, platform, params),
        "quick_search" => quick_search_call(upstream, credential, params),
        "general_search" => general_search_call(upstream, credential, platform, params),
        "search_extra" => search_by_type_call(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 两个搜索 CGI 端点缺省用 Android 档案（见文件头「平台档案」；实测 web 档案
/// 下 `search_by_type` 回空目录、`do_search_v2` 回 2001 风控）。
/// 调用方显式给了 `platform` 就用它的。
fn search_platform(params: &Value, platform: Platform) -> Platform {
    if params.get("platform").is_some() {
        platform
    } else {
        Platform::Android
    }
}

/// 热搜词（`music.musicsearch.HotkeyService / GetHotkeyForQQMusicMobile`）。
///
/// 空热搜列表是答案：这一刻上游就是没有热词。
fn hotkeys(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
) -> Result<Value, UpstreamError> {
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.musicsearch.HotkeyService",
            method: "GetHotkeyForQQMusicMobile",
            param: json!({ "search_id": search_id() }),
        },
    )?;
    Ok(hotkeys_payload(&data))
}

/// 搜索词补全（`music.smartboxCgi.SmartBoxCgi / GetSmartBoxResult`）。
///
/// 空补全列表是答案：这个前缀上游给不出建议。
fn complete(
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
            module: "music.smartboxCgi.SmartBoxCgi",
            method: "GetSmartBoxResult",
            param: complete_params(&keyword_of(params)),
        },
    )?;
    Ok(complete_payload(&data))
}

/// 快速搜索（`GET c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg`）。
///
/// 这是参考里唯一的 HTTP 路：`get_fcgi` 拿回整份 JSON（`code`/`subcode` 与
/// `data` 都在顶层），搬运时按 `$.data.<分类>` 展开。
/// 空分类是答案：这个关键词上游没有对应结果。
fn quick_search_call(
    upstream: &Upstream,
    credential: &Credential,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let keyword = keyword_of(params);
    let response = upstream.get_fcgi(
        credential,
        Class::Interactive,
        &quick_search_query(&keyword),
    )?;
    Ok(quick_search_payload(&response))
}

/// 综合搜索（`music.adaptor.SearchAdaptor / do_search_v2`）。
///
/// 一次请求一个平台档案，**不改写 `comm`**（参考的 web 层也是这么发的）。
/// 空分类是答案：这个关键词在该分类下没有命中。
fn general_search_call(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let data = upstream.call_with(
        credential,
        Class::Interactive,
        search_platform(params, platform),
        Call {
            module: "music.adaptor.SearchAdaptor",
            method: "do_search_v2",
            param: general_params(params),
        },
    )?;
    Ok(general_search_payload(&data))
}

/// 类型搜索（`music.search.SearchCgiService / DoSearchForQQMusicMobile`）。
///
/// 空列表是答案：这个关键词在该类型下没有命中。
fn search_by_type_call(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = search_type_params(params)?;
    let data = upstream.call_with(
        credential,
        Class::Interactive,
        search_platform(params, platform),
        Call {
            module: "music.search.SearchCgiService",
            method: "DoSearchForQQMusicMobile",
            param,
        },
    )?;
    Ok(search_by_type_payload(&data))
}

// MARK: - 宿主包装

/// 热搜词列表。
#[export]
pub fn fetch_search_hotkeys() -> Result<HotkeyResponse, crate::HelperError> {
    crate::port::call("fetch_search_hotkeys", json!({}))
}

/// 搜索词补全建议；`keyword` 是输入到一半的词。
#[export]
pub fn complete_search(keyword: String) -> Result<CompleteResponse, crate::HelperError> {
    crate::port::call("complete_search", json!({ "keyword": keyword }))
}

/// 快速搜索；返回单曲/歌手/专辑/MV 四个分类的条目。
#[export]
pub fn quick_search(keyword: String) -> Result<QuickSearchResponse, crate::HelperError> {
    crate::port::call("quick_search", json!({ "keyword": keyword }))
}

/// 综合搜索包装层的参数拼装（抽出来是为了能不打网络地测「宿主给的 highlight
/// 真的走到了协议参数上」）。
fn general_search_request(
    keyword: String,
    page: Option<i64>,
    num: Option<i64>,
    searchid: Option<String>,
    page_start: Option<String>,
    highlight: Option<bool>,
) -> Result<Value, crate::HelperError> {
    let mut params = json!({ "keyword": keyword });
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    if let Some(searchid) = searchid.filter(|value| !value.trim().is_empty()) {
        params["searchid"] = json!(searchid);
    }
    if let Some(page_start) = page_start.filter(|value| !value.trim().is_empty()) {
        // 宿主从上一页的 `nextpageStart` 拿到的就是一段 JSON 文本，原样透传；
        // 解析不了就报错，别把坏参数发给上游。
        let parsed: Value = serde_json::from_str(&page_start).map_err(|error| {
            crate::HelperError::InvalidRequest(format!("page_start 不是合法的 JSON：{error}"))
        })?;
        params["pageStart"] = parsed;
    }
    if let Some(highlight) = highlight {
        params["highlight"] = json!(highlight);
    }
    Ok(params)
}

/// 综合搜索；`page_start` 是上一页回带的续参（透传的对象），
/// `highlight` 给了就是参考的关键字参数（缺省 true）。
#[export]
pub fn general_search(
    keyword: String,
    page: Option<i64>,
    num: Option<i64>,
    searchid: Option<String>,
    page_start: Option<String>,
    highlight: Option<bool>,
) -> Result<GeneralSearchResponse, crate::HelperError> {
    crate::port::call(
        "general_search",
        general_search_request(keyword, page, num, searchid, page_start, highlight)?,
    )
}

/// 类型搜索包装层的参数拼装（同上，为了能不打网络地测 `highlight` 与 `selectors`
/// 真的走到了协议参数上）。
fn search_extra_request(
    keyword: String,
    search_type: Option<i64>,
    page: Option<i64>,
    num: Option<i64>,
    searchid: Option<String>,
    selectors: Option<Vec<SearchSelector>>,
    highlight: Option<bool>,
) -> Result<Value, crate::HelperError> {
    let mut params = json!({ "keyword": keyword });
    if let Some(search_type) = search_type {
        params["searchType"] = json!(search_type);
    }
    if let Some(page) = page {
        params["page"] = json!(page);
    }
    if let Some(num) = num {
        params["num"] = json!(num);
    }
    if let Some(searchid) = searchid.filter(|value| !value.trim().is_empty()) {
        params["searchid"] = json!(searchid);
    }
    if let Some(selectors) = selectors {
        params["selectors"] = serde_json::to_value(selectors)
            .map_err(|error| crate::HelperError::InvalidRequest(error.to_string()))?;
    }
    if let Some(highlight) = highlight {
        params["highlight"] = json!(highlight);
    }
    Ok(params)
}

/// 类型搜索；`search_type`: 0 歌曲 1 歌手 2 专辑 3 歌单 4 MV 7 歌词 8 用户
/// 10 彩铃 15 节目专辑 18 节目。`selectors` 是筛选器列表（参考 `SearchSelector`），
/// `highlight` 给了就是参考的关键字参数（缺省 true）。
#[export]
pub fn search_extra(
    keyword: String,
    search_type: Option<i64>,
    page: Option<i64>,
    num: Option<i64>,
    searchid: Option<String>,
    selectors: Option<Vec<SearchSelector>>,
    highlight: Option<bool>,
) -> Result<SearchByTypeResponse, crate::HelperError> {
    crate::port::call(
        "search_extra",
        search_extra_request(
            keyword,
            search_type,
            page,
            num,
            searchid,
            selectors,
            highlight,
        )?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份热搜回值：键名照上游自己的拼写（2026-10-03 的实测形状）。
    fn raw_hotkeys() -> Value {
        json!({
            "expid": "1462002",
            "hotkey_time": "20261003102",
            "ret_code": 0,
            "track_info": [],
            "track_list_id": "20261003102",
            "vec_hotkey": [{
                "cover_pic_url": "http://y.gtimg.cn/music/photo_new/T002R180x180M000002iWKlh2DcjFL_3.jpg",
                "custom_param": {"track_id": "4936030"},
                "description": "正在热搜",
                "direct_id": 4936030,
                "hotkey_id": "3.2.2.0:茶汤 郁可唯",
                "jump_tab": "0",
                "jump_url": "",
                "kind": 2,
                "need_top": 1,
                "pic_url": "",
                "query": "茶汤 郁可唯",
                "score": "853663",
                "song_type": 0,
                "source": 2,
                "subpos": 0,
                "title": "茶汤 郁可唯",
                "type": 3
            }],
            "vec_reckey": []
        })
    }

    #[test]
    fn a_hotkey_list_maps_every_field_the_reference_model_names() {
        let payload = hotkeys_payload(&raw_hotkeys());
        assert_eq!(payload["retCode"], 0);
        assert_eq!(payload["hotkeyTime"], "20261003102");
        assert_eq!(payload["trackListId"], "20261003102");
        assert_eq!(payload["vecHotkey"][0]["hotkeyId"], "3.2.2.0:茶汤 郁可唯");
        assert_eq!(payload["vecHotkey"][0]["query"], "茶汤 郁可唯");
        assert_eq!(payload["vecHotkey"][0]["score"], "853663");
        assert_eq!(payload["vecHotkey"][0]["kind"], 2);
        assert_eq!(payload["vecHotkey"][0]["type"], 3);
        assert_eq!(payload["vecHotkey"][0]["source"], 2);
        assert_eq!(payload["vecHotkey"][0]["needTop"], 1);
        assert_eq!(payload["vecHotkey"][0]["songType"], 0);
        assert_eq!(payload["vecHotkey"][0]["directId"], 4936030);
        assert_eq!(payload["vecHotkey"][0]["jumpTab"], "0");
        // 封面一律 https：http 的图宿主会直接拒绝。
        assert_eq!(
            payload["vecHotkey"][0]["coverPicUrl"],
            "https://y.gtimg.cn/music/photo_new/T002R180x180M000002iWKlh2DcjFL_3.jpg"
        );

        let response: HotkeyResponse = serde_json::from_value(payload).expect("解析热搜");
        assert_eq!(response.ret_code, Some(0));
        let hotkeys = response.vec_hotkey.clone().expect("有热词");
        assert_eq!(hotkeys.len(), 1);
        assert_eq!(hotkeys[0].query.as_deref(), Some("茶汤 郁可唯"));
        assert_eq!(hotkeys[0].kind, Some(2));
        // `vec_reckey` 是透传的 dict 列表：Rust 侧是 JSON 文本，写出去仍是数组。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["vecReckey"], json!([]));
    }

    /// 一份补全回值：`items` 里的条目字段照实测。
    fn raw_complete() -> Value {
        json!({
            "debug_info": "",
            "expid": "",
            "expire_time": 1790993381,
            "history_items": [],
            "items": [{
                "docid": "1666111162897402599",
                "hint": "周杰伦 晴天",
                "hint_hilight": "<em>周杰伦</em> 晴天",
                "icon": "",
                "icon_type": 1,
                "jump_type": 0,
                "jump_url": "",
                "jumptab": -1,
                "pic_url": "",
                "pre_search": false,
                "res_type": "search",
                "score": 7906.11572265625,
                "type": 0
            }],
            "search_id": "1234567890123456789",
            "search_query_info": {},
            "total_num": 178,
            "use_default_search": 0,
            "vec_direct_items": [],
            "vec_related_items": []
        })
    }

    #[test]
    fn a_complete_page_keeps_its_hint_and_the_session_id() {
        let payload = complete_payload(&raw_complete());
        assert_eq!(payload["totalNum"], 178);
        assert_eq!(payload["searchId"], "1234567890123456789");
        assert_eq!(payload["expireTime"], 1790993381);
        assert_eq!(payload["useDefaultSearch"], 0);
        assert_eq!(payload["items"][0]["hint"], "周杰伦 晴天");
        assert_eq!(payload["items"][0]["hintHilight"], "<em>周杰伦</em> 晴天");
        assert_eq!(payload["items"][0]["docid"], "1666111162897402599");
        assert_eq!(payload["items"][0]["iconType"], 1);
        assert_eq!(payload["items"][0]["jumptab"], -1);
        assert_eq!(payload["items"][0]["preSearch"], false);
        assert_eq!(payload["items"][0]["score"], 7906.11572265625);

        let response: CompleteResponse = serde_json::from_value(payload).expect("解析补全");
        let items = response.items.clone().expect("有建议");
        assert_eq!(items[0].type_id, Some(0));
        assert_eq!(items[0].res_type.as_deref(), Some("search"));
        assert_eq!(items[0].score, Some(7906.11572265625));
        // 三个透传列表在 JSON 上仍是数组。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["historyItems"], json!([]));
        assert_eq!(round["vecDirectItems"], json!([]));
        assert_eq!(round["vecRelatedItems"], json!([]));
    }

    /// 一份快速搜索回值（实测形状：四个分类在 `data` 上）。
    fn raw_quick_search() -> Value {
        json!({
            "code": 0,
            "subcode": 0,
            "data": {
                "album": {
                    "count": 2,
                    "itemlist": [{
                        "docid": "60671",
                        "id": "60671",
                        "mid": "0024bjiL2aocxT",
                        "name": "十一月的萧邦",
                        "pic": "http://y.gtimg.cn/music/photo_new/T002R180x180M0000024bjiL2aocxT_5.jpg",
                        "singer": "周杰伦"
                    }],
                    "name": "专辑",
                    "order": 2,
                    "type": 3
                },
                "mv": {
                    "count": 1,
                    "itemlist": [{
                        "docid": "293791",
                        "id": "293791",
                        "mid": "00061J2t0b0PPW",
                        "name": "晴天",
                        "singer": "周杰伦",
                        "vid": "w0026q7f01a"
                    }],
                    "name": "MV",
                    "order": 3,
                    "type": 4
                },
                "singer": {
                    "count": 1,
                    "itemlist": [{
                        "docid": "4558",
                        "id": "4558",
                        "mid": "0025NhlN2yWrP4",
                        "name": "周杰伦",
                        "pic": "http://y.gtimg.cn/music/photo_new/T001R150x150M0000025NhlN2yWrP4_11.jpg",
                        "singer": "周杰伦"
                    }],
                    "name": "歌手",
                    "order": 1,
                    "type": 2
                },
                "song": {
                    "count": 1,
                    "itemlist": [{
                        "docid": "102065750",
                        "id": "102065750",
                        "mid": "001Bbywq2gicae",
                        "name": "搁浅",
                        "singer": "周杰伦"
                    }],
                    "name": "单曲",
                    "order": 0,
                    "type": 1
                }
            }
        })
    }

    #[test]
    fn quick_search_unwraps_the_four_categories_out_of_data() {
        let payload = quick_search_payload(&raw_quick_search());
        assert_eq!(payload["song"]["count"], 1);
        assert_eq!(payload["song"]["itemlist"][0]["name"], "搁浅");
        assert_eq!(payload["song"]["itemlist"][0]["mid"], "001Bbywq2gicae");
        assert_eq!(payload["singer"]["itemlist"][0]["name"], "周杰伦");
        assert_eq!(payload["album"]["itemlist"][0]["name"], "十一月的萧邦");
        assert_eq!(payload["mv"]["itemlist"][0]["vid"], "w0026q7f01a");
        // 分类的封面转 https。
        assert_eq!(
            payload["singer"]["itemlist"][0]["pic"],
            "https://y.gtimg.cn/music/photo_new/T001R150x150M0000025NhlN2yWrP4_11.jpg"
        );

        let response: QuickSearchResponse = serde_json::from_value(payload).expect("解析快速搜索");
        assert_eq!(response.song.as_ref().map(|c| c.count), Some(Some(1)));
        let song = response.song.expect("有单曲分类");
        assert_eq!(song.type_id, Some(1));
        assert_eq!(song.order, Some(0));
        let items = song.itemlist.expect("有条目");
        assert_eq!(items[0].id.as_deref(), Some("102065750"));
        assert_eq!(
            response.mv.expect("有 MV 分类").itemlist.unwrap()[0]
                .vid
                .as_deref(),
            Some("w0026q7f01a")
        );
    }

    /// 一份类型搜索（歌曲）回值：`meta` 与 `body` 都是实测形状。
    ///
    /// 拆成两段构造：`json!` 的递归层数有上限，整份写在一起会顶到 limit。
    fn raw_search_by_type_song() -> Value {
        let mut song = json!({
            "id": 102065750,
            "mid": "001Bbywq2gicae",
            "name": "搁浅",
            "title": "<em>搁浅</em>",
            "title_main": "<em>搁浅</em>",
            "title_extra": "",
            "search_title": "<em>搁浅</em>",
            "fav_show": "",
            "desc": "",
            "desc_icon": "",
            "content": "",
            "newStatus": 2,
            "protect": 0,
            "hotness": {"desc": "", "icon_url": "", "jump_type": 0, "tag_id": ""},
            "hotness_desc": "",
            "vec_hotness": [],
            "relatedword_group": {"is_show": 0, "title": "", "word_list": []},
            "interval": 260,
            "type": 0
        });
        // 字段太多，分开拼：`json!` 的递归深度随字段数增长，一次写全会顶到 limit。
        let tail = json!({
            "isonly": 0,
            "language": 0,
            "genre": 0,
            "index_cd": 0,
            "index_album": 0,
            "time_public": "2003-07-31",
            "status": 0,
            "label": "",
            "bpm": 0,
            "ov": 0,
            "sa": 0,
            "es": "",
            "album": {"id": 8220, "mid": "000MkMni19ClKG", "name": "叶惠美", "pmid": "000MkMni19ClKG_5"},
            "singer": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦", "title": "<em>周杰伦</em>", "type": 0, "uin": 0}],
            "mv": {"id": 293791, "vid": "w0026q7f01a", "vt": 0, "name": "", "title": ""},
            "file": {"media_mid": "003Qui1q2u1Zho", "size_128mp3": 4317292},
            "pay": {"pay_down": 1, "pay_month": 1, "pay_play": 1, "price_track": 200}
        });
        if let (Some(object), Some(extra)) = (song.as_object_mut(), tail.as_object()) {
            object.extend(extra.clone());
        }
        json!({
            "meta": {
                "estimate_sum": 44984,
                "nextpage": 2,
                "perpage": 10,
                "searchid": "290132681600515942",
                "sum": 999
            },
            "body": {
                "item_song": [song],
                "multi_extern_info": {
                    "selectors": [[{"id": 4558, "name": "周杰伦", "type": 0}]]
                }
            }
        })
    }

    #[test]
    fn a_song_search_page_maps_the_meta_and_the_track() {
        let source = raw_search_by_type_song();
        let payload = search_by_type_payload(&source);
        assert_eq!(payload["searchid"], "290132681600515942");
        assert_eq!(payload["perpage"], 10);
        assert_eq!(payload["nextpage"], 2);
        assert_eq!(payload["estimateSum"], 44984);
        assert_eq!(payload["totalNum"], 999);
        assert_eq!(payload["song"][0]["songMid"], "001Bbywq2gicae");
        assert_eq!(payload["song"][0]["searchTitle"], "<em>搁浅</em>");
        assert_eq!(payload["song"][0]["titleMain"], "<em>搁浅</em>");
        assert_eq!(payload["song"][0]["newStatus"], 2);
        assert_eq!(payload["song"][0]["protect"], 0);
        assert_eq!(payload["song"][0]["relatedwordGroup"]["is_show"], 0);
        assert_eq!(payload["selectors"][0][0]["name"], "周杰伦");

        let response: SearchByTypeResponse = serde_json::from_value(payload).expect("解析类型搜索");
        assert_eq!(response.total_num, Some(999));
        assert_eq!(response.nextpage, Some(2));
        let songs = response.song.expect("有歌曲");
        assert_eq!(songs[0].song_mid, "001Bbywq2gicae");
        assert_eq!(songs[0].duration, Some(260));
        assert_eq!(songs[0].singers.as_ref().map(Vec::len), Some(1));
        // Reuse this captured upstream-shaped source fixture for the shared
        // playback track mapper; these fields used to be discarded there.
        let decoded = crate::methods::decode_track(&source["body"]["item_song"][0])
            .expect("catalogue track decodes");
        assert_eq!(decoded["mediaMid"], "003Qui1q2u1Zho");
        assert_eq!(decoded["genre"], 0);
        assert_eq!(
            decoded["fileSizes"],
            json!([{"name":"128mp3","bytes":4317292}])
        );
        let groups = response.selectors.expect("有筛选器");
        assert_eq!(groups[0][0].id, Some(4558));
        assert_eq!(groups[0][0].type_id, Some(0));
    }

    #[test]
    fn a_singer_search_page_uses_the_search_spellings() {
        let payload = search_by_type_payload(&json!({
            "meta": {"sum": 11, "nextpage": 2, "perpage": 10, "searchid": "s-1"},
            "body": {
                "singer": [{
                    "singerID": 4558,
                    "singerMID": "0025NhlN2yWrP4",
                    "singerName": "周杰伦",
                    "singerPic": "http://y.gtimg.cn/music/photo_new/T001R150x150M0000025NhlN2yWrP4_11.jpg",
                    "songNum": 1012,
                    "albumNum": 43,
                    "mvNum": 10424,
                    "subtitle": "歌曲:1012  专辑:43  视频:10424"
                }]
            }
        }));
        let response: SearchByTypeResponse = serde_json::from_value(payload).expect("解析歌手搜索");
        let singers = response.singer.expect("有歌手");
        assert_eq!(singers[0].mid.as_deref(), Some("0025NhlN2yWrP4"));
        assert_eq!(singers[0].name.as_deref(), Some("周杰伦"));
    }

    #[test]
    fn an_album_search_page_reads_the_core_album_config() {
        let payload = search_by_type_payload(&json!({
            "meta": {"sum": 497, "nextpage": -1},
            "body": {
                "item_album": [{
                    "id": 87495226,
                    "albummid": "0041WVfh2vtlJE",
                    "name": "太阳之子",
                    "pic": "http://y.gtimg.cn/music/photo_new/T002R180x180M0000041WVfh2vtlJE_1.jpg",
                    "publish_date": "2026-03-25",
                    "core_album_config": {"album_type": 1, "award_label": "殿堂史诗唱片"},
                    "description": "<em>周杰伦</em>  2026-03-25",
                    "singer": "<em>周杰伦</em>",
                    "song_num": 12,
                    "singer_list": [{"id": 4558, "mid": "", "name": "周杰伦", "title": "<em>周杰伦</em>"}],
                    "tag_list": []
                }]
            }
        }));
        let response: SearchByTypeResponse = serde_json::from_value(payload).expect("解析专辑搜索");
        let albums = response.album.expect("有专辑");
        assert_eq!(albums[0].id, Some(87495226));
        assert_eq!(albums[0].mid.as_deref(), Some("0041WVfh2vtlJE"));
        assert_eq!(
            albums[0].kind,
            Some(1),
            "type 来自 core_album_config.album_type"
        );
        assert_eq!(albums[0].award_label.as_deref(), Some("殿堂史诗唱片"));
        assert_eq!(
            albums[0]
                .singer_list
                .as_ref()
                .map(|list| list[0].name.as_deref()),
            Some(Some("周杰伦"))
        );
        assert_eq!(albums[0].time_public.as_deref(), Some("2026-03-25"));
        assert_eq!(
            albums[0].pic.as_deref(),
            Some("https://y.gtimg.cn/music/photo_new/T002R180x180M0000041WVfh2vtlJE_1.jpg")
        );
        let library_album = crate::catalog::map_album(&json!({
            "id": 87495226,
            "albummid": "0041WVfh2vtlJE",
            "name": "太阳之子",
            "song_num": 12
        }))
        .expect("album maps to shared model");
        assert_eq!(library_album["songCount"], 12);
        assert_eq!(response.nextpage, Some(-1), "取完了");
    }

    #[test]
    fn a_songlist_search_page_keeps_the_creator_and_the_cover() {
        let payload = search_by_type_payload(&json!({
            "meta": {"sum": 299},
            "body": {
                "item_songlist": [{
                    "dissid": "7039749142",
                    "dissname": "百听不厌的<em>周杰伦</em>",
                    "logo": "http://qpic.y.qq.com/music_cover/x/300?n=1",
                    "nickname": "今晚月色很美",
                    "songnum": 99,
                    "listennum": 413879414,
                    "description": "99首  今晚月色很美  4.1亿次播放"
                }]
            }
        }));
        let response: SearchByTypeResponse = serde_json::from_value(payload).expect("解析歌单搜索");
        let songlists = response.songlist.expect("有歌单");
        assert_eq!(songlists[0].id, Some(7039749142));
        assert_eq!(
            songlists[0].title.as_deref(),
            Some("百听不厌的<em>周杰伦</em>")
        );
        assert_eq!(songlists[0].nickname.as_deref(), Some("今晚月色很美"));
        assert_eq!(songlists[0].songnum, Some(99));
        assert_eq!(
            songlists[0].picurl.as_deref(),
            Some("https://qpic.y.qq.com/music_cover/x/300?n=1")
        );
    }

    #[test]
    fn an_mv_search_page_reads_the_mv_spellings() {
        let payload = search_by_type_payload(&json!({
            "meta": {"sum": 598},
            "body": {
                "item_mv": [{
                    "id": "293791",
                    "vid": "w0026q7f01a",
                    "mvname": "晴天",
                    "title_main": "晴天",
                    "pic": "http://y.gtimg.cn/music/photo_new/T015R640x360M10300061J2t0b0PPW.jpg",
                    "play_count": 120407698,
                    "duration": 317,
                    "publish_date": "2003-07-29",
                    "singerid": 4558,
                    "singermid": "0025NhlN2yWrP4",
                    "singername": "<em>周杰伦</em>",
                    "type": 0
                }]
            }
        }));
        let response: SearchByTypeResponse = serde_json::from_value(payload).expect("解析 MV 搜索");
        let mvs = response.mv.expect("有 MV");
        assert_eq!(mvs[0].id, Some(293791), "数字字符串的 id 收成数字");
        assert_eq!(mvs[0].vid.as_deref(), Some("w0026q7f01a"));
        assert_eq!(mvs[0].name.as_deref(), Some("晴天"));
        assert_eq!(mvs[0].play_count, Some(120407698));
        assert_eq!(mvs[0].singer_id, Some(4558));
        assert_eq!(mvs[0].singer_name.as_deref(), Some("<em>周杰伦</em>"));
    }

    #[test]
    fn a_user_search_page_passes_the_upstream_shape_through() {
        let payload = search_by_type_payload(&json!({
            "meta": {"sum": 423},
            "body": {
                "item_user": [{
                    "uin": "4344553598",
                    "encrypt_uin": "7eoP7e4koi4qNn**",
                    "title": "周杰伦",
                    "pic": "https://y.qq.com/music/photo_new/T001R300x300M0000025NhlN2yWrP4_10.jpg",
                    "subtitle": "5082.7万人关注",
                    "concern_status": 1
                }]
            }
        }));
        let response: SearchByTypeResponse = serde_json::from_value(payload).expect("解析用户搜索");
        let users: Value = serde_json::from_str(response.user.as_deref().expect("有用户")).unwrap();
        assert_eq!(users[0]["uin"], "4344553598");
        assert_eq!(users[0]["title"], "周杰伦");
    }

    #[test]
    fn an_audio_album_search_page_reuses_the_album_mapping() {
        let payload = search_by_type_payload(&json!({
            "meta": {"sum": 300},
            "body": {
                "item_audio": [{
                    "id": 17890588,
                    "albummid": "000liYZP2PVjbw",
                    "name": "<em>周杰伦</em>歌迷电台",
                    "category": "音乐节目",
                    "song_num": 32,
                    "publish_date": "2021-02-26",
                    "core_album_config": {"album_type": 0}
                }]
            }
        }));
        let response: SearchByTypeResponse =
            serde_json::from_value(payload).expect("解析节目专辑搜索");
        let audio = response.audio_alum.expect("有节目专辑");
        assert_eq!(audio[0].id, Some(17890588));
        assert_eq!(audio[0].name.as_deref(), Some("<em>周杰伦</em>歌迷电台"));
        assert_eq!(audio[0].kind, Some(0));
    }

    #[test]
    fn a_general_search_page_reads_meta_and_every_bucket() {
        let payload = general_search_payload(&json!({
            "meta": {
                "sid": "290132681600515942",
                "perpage": 15,
                "nextpage": 2,
                "nextpage_start": {"a": 1},
                "sum": 999
            },
            "body": {
                "item_song": {
                    "estimate_sum": 44984,
                    "total_num": 999,
                    "more_info": {"b": 2},
                    "items": [{
                        "id": 102065750,
                        "mid": "001Bbywq2gicae",
                        "name": "搁浅",
                        "search_title": "<em>搁浅</em>",
                        "album": {"id": 8220, "mid": "000MkMni19ClKG", "name": "叶惠美"},
                        "singer": [{"id": 4558, "mid": "0025NhlN2yWrP4", "name": "周杰伦"}]
                    }]
                },
                "singer": {"items": []},
                "item_album": {"items": []},
                "item_mv": {"items": []},
                "item_songlist": {"items": []},
                "item_audio": {"items": []},
                "direct_result": {"direct_group": [{"x": 1}]},
                "item_related": {
                    "items": [{"display_word": "周杰伦 歌单", "search_word": "周杰伦"}]
                }
            }
        }));
        assert_eq!(payload["searchid"], "290132681600515942");
        assert_eq!(payload["perpage"], 15);
        assert_eq!(payload["nextpage"], 2);
        assert_eq!(payload["nextpageStart"]["a"], 1);
        assert_eq!(payload["song"]["totalNum"], 999);
        assert_eq!(payload["song"]["estimateSum"], 44984);
        assert_eq!(payload["song"]["items"][0]["songMid"], "001Bbywq2gicae");
        assert_eq!(payload["song"]["items"][0]["searchTitle"], "<em>搁浅</em>");
        assert_eq!(payload["related"]["items"][0]["display"], "周杰伦 歌单");
        assert_eq!(payload["related"]["items"][0]["search"], "周杰伦");
        assert_eq!(payload["direct"][0]["x"], 1);

        let response: GeneralSearchResponse =
            serde_json::from_value(payload).expect("解析综合搜索");
        assert_eq!(response.nextpage, Some(2));
        let song = response.song.clone().expect("有单曲桶");
        assert_eq!(song.total_num, Some(999));
        assert_eq!(song.items.as_ref().map(Vec::len), Some(1));
        assert_eq!(
            song.items
                .as_ref()
                .map(|items| items[0].song_mid.as_deref()),
            Some(Some("001Bbywq2gicae"))
        );
        assert_eq!(
            song.items
                .as_ref()
                .map(|items| items[0].search_title.as_deref()),
            Some(Some("<em>搁浅</em>"))
        );
        // `moreInfo` / `nextpageStart` 是透传的 dict：写出去仍是对象。
        let round: Value = serde_json::to_value(&response).unwrap();
        assert_eq!(round["nextpageStart"]["a"], 1);
        assert_eq!(round["song"]["moreInfo"]["b"], 2);
        assert_eq!(round["related"]["items"][0]["search"], "周杰伦");
    }

    #[test]
    fn an_empty_bucket_is_an_answer_not_a_missing_key() {
        let payload = general_search_payload(&json!({
            "meta": {"sid": "s", "nextpage": -1},
            "body": {"item_song": {"items": []}}
        }));
        // 键在、给了空列表：这是答案（这个关键词这一页就是没有）。
        assert_eq!(payload["song"]["items"], json!([]));
        // 键不在：字段保持 null，宿主据此判断响应不成形。
        assert_eq!(payload["album"], Value::Null);
        assert_eq!(payload["related"], Value::Null);
    }

    #[test]
    fn the_general_params_carry_the_reference_defaults() {
        let param = general_params(&json!({"keyword": "周杰伦", "searchid": "s-1"}));
        assert_eq!(param["searchid"], "s-1");
        assert_eq!(param["search_type"], 100);
        assert_eq!(param["page_num"], 15);
        assert_eq!(param["query"], "周杰伦");
        assert_eq!(param["page_id"], 1);
        assert_eq!(param["highlight"], true);
        assert_eq!(param["grp"], true);
        assert!(param.get("page_start").is_none(), "没给就不带");

        let with_start = general_params(&json!({
            "keyword": "周杰伦",
            "page": 2,
            "num": 10,
            "highlight": false,
            "pageStart": {"a": 1}
        }));
        assert_eq!(with_start["page_id"], 2);
        assert_eq!(with_start["page_num"], 10);
        assert_eq!(with_start["highlight"], false);
        assert_eq!(with_start["page_start"], json!({"a": 1}));
    }

    #[test]
    fn the_type_params_carry_both_selector_shapes() {
        let param = search_type_params(&json!({
            "keyword": "周杰伦",
            "searchType": 1,
            "page": 2,
            "num": 20,
            "searchid": "s-1",
            "selectors": [{"id": 4558, "name": "周杰伦", "type": 0}]
        }))
        .expect("是支持的 search_type");
        assert_eq!(param["search_type"], 1);
        assert_eq!(param["page_num"], 2);
        assert_eq!(param["num_per_page"], 20);
        assert_eq!(param["searchid"], "s-1");
        assert_eq!(param["grp"], true);
        assert_eq!(param["highlight"], true);
        assert_eq!(param["selectors"], json!({"0": "4558"}));
        assert_eq!(
            param["vec_selectors"],
            json!([{"type": 0, "name": "周杰伦", "id": 4558}])
        );

        let bare = search_type_params(&json!({"keyword": "x"})).expect("缺省歌曲");
        assert_eq!(bare["search_type"], 0, "缺省 SearchType.SONG");
        assert_eq!(bare["num_per_page"], 10);
        assert_eq!(bare["page_num"], 1);
        assert_eq!(bare["selectors"], json!({}), "没给就是空映射");
        assert_eq!(bare["vec_selectors"], json!([]));
        assert_eq!(bare["highlight"], true);
    }

    #[test]
    fn the_host_can_turn_highlight_off_through_the_wrappers() {
        // 参考的两个方法是关键字参数 `highlight: bool = True`，缺省不改；
        // 宿主显式给了就逐字走到 param 上（关掉高亮必须做得到）。
        let general_off =
            general_search_request("周杰伦".into(), None, None, None, None, Some(false))
                .expect("拼参数");
        assert_eq!(general_off["keyword"], "周杰伦");
        assert_eq!(general_off["highlight"], false);

        let general_default =
            general_search_request("周杰伦".into(), None, None, None, None, None).expect("拼参数");
        assert!(general_default.get("highlight").is_none(), "没给就交给缺省");

        let type_off = search_extra_request(
            "周杰伦".into(),
            Some(SEARCH_TYPE_SONG),
            None,
            None,
            None,
            None,
            Some(false),
        )
        .expect("拼参数");
        assert_eq!(type_off["highlight"], false);
        // 走到 dispatch 侧时，电台/搜索参数照参考读出这个 false。
        let param = search_type_params(&type_off).expect("supported type");
        assert_eq!(param["highlight"], false, "关高亮的开关真的生效了");

        let type_default =
            search_extra_request("周杰伦".into(), None, None, None, None, None, None)
                .expect("拼参数");
        let param = search_type_params(&type_default).expect("supported type");
        assert_eq!(param["highlight"], true, "缺省仍是参考的 True");

        // 宿主的 selectors 也真的走到了 `selectors` / `vec_selectors` 两份形状上。
        let with_selectors = search_extra_request(
            "周杰伦".into(),
            None,
            None,
            None,
            None,
            Some(vec![SearchSelector {
                id: Some(4558),
                name: Some("周杰伦".into()),
                type_id: Some(0),
            }]),
            None,
        )
        .expect("拼参数");
        let param = search_type_params(&with_selectors).expect("supported type");
        assert_eq!(param["selectors"], json!({"0": "4558"}));
        assert_eq!(
            param["vec_selectors"],
            json!([{"type": 0, "name": "周杰伦", "id": 4558}])
        );
    }

    #[test]
    fn the_general_wrapper_still_carries_page_start_as_json() {
        let params = general_search_request(
            "周杰伦".into(),
            Some(2),
            Some(10),
            Some("s-1".into()),
            Some("{\"song\": 30}".into()),
            None,
        )
        .expect("拼参数");
        assert_eq!(params["pageStart"], json!({"song": 30}));
        assert_eq!(params["searchid"], "s-1");

        let error = general_search_request(
            "周杰伦".into(),
            None,
            None,
            None,
            Some("不是 JSON".into()),
            None,
        )
        .expect_err("坏 page_start 要在发请求前拒掉");
        assert!(error.to_string().contains("page_start"));
    }

    #[test]
    fn every_search_type_has_its_number_and_the_rest_are_refused() {
        for (name, number) in [
            ("歌曲", SEARCH_TYPE_SONG),
            ("歌手", SEARCH_TYPE_SINGER),
            ("专辑", SEARCH_TYPE_ALBUM),
            ("歌单", SEARCH_TYPE_SONGLIST),
            ("MV", SEARCH_TYPE_MV),
            ("歌词", SEARCH_TYPE_LYRIC),
            ("用户", SEARCH_TYPE_USER),
            ("彩铃", SEARCH_TYPE_RINGTONE),
            ("节目专辑", SEARCH_TYPE_AUDIO_ALBUM),
            ("节目", SEARCH_TYPE_AUDIO),
        ] {
            assert!(
                search_type_params(&json!({"keyword": "x", "searchType": number})).is_ok(),
                "{name}（{number}）应当被接受"
            );
        }
        let error = search_type_params(&json!({"keyword": "x", "searchType": 5}))
            .expect_err("5 不是 SearchType 的成员");
        assert!(error.to_string().contains("search_type"));
    }

    #[test]
    fn the_complete_params_carry_the_reference_zeros() {
        let param = complete_params("周杰伦");
        assert_eq!(param["query"], "周杰伦");
        assert_eq!(param["num_per_page"], 0);
        assert_eq!(param["page_idx"], 0);
        // search_id 是按参考公式（`e * 2^54 + n * 2^32 + 当日毫秒`）算出的纯数字串。
        let id = param["search_id"].as_str().expect("有 search_id");
        assert!(id.len() >= 17, "参考公式的产物至少 17 位：{id}");
        assert!(id.chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn the_quick_search_url_is_the_reference_one_with_one_query_parameter() {
        let url = quick_search_query("周杰伦");
        assert!(url.starts_with(QUICK_SEARCH_URL));
        assert!(url.contains("key=%E5%91%A8%E6%9D%B0%E4%BC%A6"));
        assert_eq!(url.matches('?').count(), 1);
    }

    #[test]
    fn the_two_search_calls_default_to_android_unless_the_caller_says_otherwise() {
        assert_eq!(
            search_platform(&json!({}), Platform::Web),
            Platform::Android,
            "参考只给 search_by_type 标了 ANDROID，do_search_v2 实测也要它"
        );
        // 调用方在参数里给了 `platform` 时，`methods.rs` 已经把它解析成了
        // 传入的 platform，这里照原样透传（`singer_extra::tab_platform` 同款判据）。
        assert_eq!(
            search_platform(&json!({"platform": "web"}), Platform::Web),
            Platform::Web,
            "调用方显式给了就尊重"
        );
        assert_eq!(
            search_platform(&json!({"platform": "android"}), Platform::Android),
            Platform::Android
        );
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 5);
        // 正向只断言「登记在册」：`dispatch` 认领后就真发请求，这里不打网络。
        for method in METHODS {
            assert!(crate::port::is_known(method), "{method} 没有登记到本层");
        }
        // 反向：不是本领域的方法要原样放行（`dispatch` 返 `None`），不发请求。
        let upstream = Upstream::new();
        for foreign in [
            "search_songs",
            "search_artists",
            "search_albums",
            "search_playlists",
            "fetch_cdn_dispatch",
        ] {
            assert!(
                dispatch(
                    &upstream,
                    &Credential::default(),
                    Platform::Web,
                    foreign,
                    &json!({})
                )
                .is_none(),
                "{foreign} 不是 search_extra 的方法"
            );
        }
    }
}

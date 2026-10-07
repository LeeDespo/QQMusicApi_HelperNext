//! 歌曲资产：批量歌曲信息、CDN 调度、批量取流地址、其他版本、制作人、收藏数。
//!
//! 当前公开契约见 `docs/endpoints.md`，解析与兼容规则见 `docs/parsing.md`。
//!
//! * `query_songs` —— `music.trackInfo.UniformRuleCtrl / CgiGetTrackInfo`，
//!   每项给 `id` 或 `mid` 之一，`types` 与 `modify_stamp` 同长；
//! * `fetch_cdn_dispatch` —— `music.audioCdnDispatch.cdnDispatch / GetCdnDispatch`；
//! * `resolve_song_urls` —— `music.vkey.GetVkey / UrlGetVkey`；顶层 `file_type`
//!   属于加密类时改走 `music.vkey.GetEVkey / CgiGetEVkey`；
//! * `fetch_other_versions` —— `music.musichallSong.OtherVersionServer / GetOtherVersionSongs`；
//! * `fetch_song_producer` —— `music.sociality.KolWorksTag / SongProducer`；
//! * `fetch_song_fav_count` —— `music.musicasset.SongFavRead / GetSongFansNumberById`。
//!
//! # 平台档案
//!
//! 取流依赖设备会话（参考库里 `get_song_urls` 走的是 android 那套 `comm`，
//! `docs/parsing.md` §1 的"取流"一档），所以 `resolve_song_urls` 在调用方没显式
//! 指定档案时默认 android——与既有 `resolve_song_url` 同款处理。其余五个参考没标
//! `platform`，按本层约定默认 Web。dispatch 收到调用方传入的 `platform` 时尊重它。
//!
//! # 空值判据
//!
//! 六个接口在参考里都没有 `require_login`，读的是公开曲库或按项授权的取流结果：
//! **空列表 / 每项的结果码是答案**（这批 id 上游没有曲目、这首歌没有其他版本、
//! 账号没有该音质权限），照常返回，不当故障。真正不成立的请求（`song_info` 为空、
//! id 与 mid 同给或都不给、超过批量上限、认不出的文件类型）在**发请求之前**报错：
//! 上游对这类输入多半回一份"成功但空"，静默空比报错更难查。
//!
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。

use crate::credential::Credential;
use crate::upstream::{
    first_array, first_int, first_text, Call, Platform, Upstream, UpstreamError,
};
use crate::Class;
use boltffi::{data, export};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[
    "query_songs",
    "fetch_cdn_dispatch",
    "resolve_song_urls",
    "fetch_other_versions",
    "fetch_song_producer",
    "fetch_song_fav_count",
];

/// 参考 `_GET_SONG_URLS_MAX_MID`：一次取流最多带这么多 mid。
///
/// 超限时上游回错误且无结果（参考的 `ValueError`），所以提前拒掉；恰好 100 个
/// 是允许的——参考比的是 `>`。
const GET_SONG_URLS_MAX_MID: usize = 100;

// MARK: - 文件类型表（参考 `SongFileType` / `EncryptedSongFileType` / `SpecialSongFileType` / `RingSongFileType`）

/// 参考 `BaseSongFileType` 的一个成员：编码前缀 + 后缀，外加它属于哪个枚举类。
///
/// `encrypted` 就是参考的 `isinstance(file_type, EncryptedSongFileType)`：它决定
/// 走 `GetVkey` 还是 `GetEVkey`（参考只对**顶层** `file_type` 做这个判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileTypeSpec {
    /// 参考枚举里的成员名（如 `MP3_128`）。
    name: &'static str,
    /// 成员所属的参考枚举类名。
    class: &'static str,
    /// 歌曲文件编码前缀（参考的 `.s`）。
    prefix: &'static str,
    /// 歌曲文件后缀（参考的 `.e`）。
    extension: &'static str,
    /// 是否属于加密类。
    encrypted: bool,
}

/// 参考四个枚举的全部成员，逐字照抄（声明次序也照参考）。
///
/// 普通类放在最前：同名的成员（`FLAC`、`MASTER`、`OGG_320`…）分属普通与加密两个
/// 枚举，短名字按这份表的先后取到普通版——加密版要选时用类名前缀
/// （`EncryptedSongFileType.FLAC`）或直接用 4 位前缀（`F0M0`）。
const FILE_TYPES: &[FileTypeSpec] = &[
    // SongFileType
    FileTypeSpec {
        name: "DTS_X",
        class: "SongFileType",
        prefix: "DT03",
        extension: ".mp4",
        encrypted: false,
    },
    FileTypeSpec {
        name: "MASTER",
        class: "SongFileType",
        prefix: "AI00",
        extension: ".flac",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ATMOS_2",
        class: "SongFileType",
        prefix: "Q000",
        extension: ".flac",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ATMOS_51",
        class: "SongFileType",
        prefix: "Q001",
        extension: ".flac",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ATMOS_71",
        class: "SongFileType",
        prefix: "Q003",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ATMOS_DB",
        class: "SongFileType",
        prefix: "D004",
        extension: ".mp4",
        encrypted: false,
    },
    FileTypeSpec {
        name: "NAC",
        class: "SongFileType",
        prefix: "TL01",
        extension: ".nac",
        encrypted: false,
    },
    FileTypeSpec {
        name: "FLAC",
        class: "SongFileType",
        prefix: "F000",
        extension: ".flac",
        encrypted: false,
    },
    FileTypeSpec {
        name: "OGG_640",
        class: "SongFileType",
        prefix: "O801",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "OGG_320",
        class: "SongFileType",
        prefix: "O800",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "OGG_192",
        class: "SongFileType",
        prefix: "O600",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "OGG_96",
        class: "SongFileType",
        prefix: "O400",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "MP3_320",
        class: "SongFileType",
        prefix: "M800",
        extension: ".mp3",
        encrypted: false,
    },
    FileTypeSpec {
        name: "MP3_128",
        class: "SongFileType",
        prefix: "M500",
        extension: ".mp3",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ACC_192",
        class: "SongFileType",
        prefix: "C600",
        extension: ".m4a",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ACC_96",
        class: "SongFileType",
        prefix: "C400",
        extension: ".m4a",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ACC_48",
        class: "SongFileType",
        prefix: "C200",
        extension: ".m4a",
        encrypted: false,
    },
    // EncryptedSongFileType
    FileTypeSpec {
        name: "DTS_X",
        class: "EncryptedSongFileType",
        prefix: "DTM3",
        extension: ".mmp4",
        encrypted: true,
    },
    FileTypeSpec {
        name: "VINYL",
        class: "EncryptedSongFileType",
        prefix: "V0M0",
        extension: ".mflac",
        encrypted: true,
    },
    FileTypeSpec {
        name: "MASTER",
        class: "EncryptedSongFileType",
        prefix: "AIM0",
        extension: ".mflac",
        encrypted: true,
    },
    FileTypeSpec {
        name: "ATMOS_2",
        class: "EncryptedSongFileType",
        prefix: "Q0M0",
        extension: ".mflac",
        encrypted: true,
    },
    FileTypeSpec {
        name: "ATMOS_51",
        class: "EncryptedSongFileType",
        prefix: "Q0M1",
        extension: ".mflac",
        encrypted: true,
    },
    FileTypeSpec {
        name: "ATMOS_71",
        class: "EncryptedSongFileType",
        prefix: "Q0M3",
        extension: ".mgg",
        encrypted: true,
    },
    FileTypeSpec {
        name: "ATMOS_DB",
        class: "EncryptedSongFileType",
        prefix: "D0M4",
        extension: ".mmp4",
        encrypted: true,
    },
    FileTypeSpec {
        name: "NAC",
        class: "EncryptedSongFileType",
        prefix: "TLM1",
        extension: ".mnac",
        encrypted: true,
    },
    FileTypeSpec {
        name: "FLAC",
        class: "EncryptedSongFileType",
        prefix: "F0M0",
        extension: ".mflac",
        encrypted: true,
    },
    FileTypeSpec {
        name: "OGG_640",
        class: "EncryptedSongFileType",
        prefix: "O8M1",
        extension: ".mgg",
        encrypted: true,
    },
    FileTypeSpec {
        name: "OGG_320",
        class: "EncryptedSongFileType",
        prefix: "O8M0",
        extension: ".mgg",
        encrypted: true,
    },
    FileTypeSpec {
        name: "OGG_192",
        class: "EncryptedSongFileType",
        prefix: "O6M0",
        extension: ".mgg",
        encrypted: true,
    },
    FileTypeSpec {
        name: "OGG_96",
        class: "EncryptedSongFileType",
        prefix: "O4M0",
        extension: ".mgg",
        encrypted: true,
    },
    // SpecialSongFileType
    FileTypeSpec {
        name: "TRY",
        class: "SpecialSongFileType",
        prefix: "RS02",
        extension: ".mp3",
        encrypted: false,
    },
    FileTypeSpec {
        name: "TRY_OGG_640",
        class: "SpecialSongFileType",
        prefix: "O802",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "ACCOM",
        class: "SpecialSongFileType",
        prefix: "O801",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "MULTI",
        class: "SpecialSongFileType",
        prefix: "O601",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "PIANO",
        class: "SpecialSongFileType",
        prefix: "AI01",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "BAYIN",
        class: "SpecialSongFileType",
        prefix: "AI02",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "GUZHENG",
        class: "SpecialSongFileType",
        prefix: "AI03",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "QUDI",
        class: "SpecialSongFileType",
        prefix: "AI04",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "HULUSI",
        class: "SpecialSongFileType",
        prefix: "AI05",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "SUONA",
        class: "SpecialSongFileType",
        prefix: "AI06",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "SHOUDIE",
        class: "SpecialSongFileType",
        prefix: "AI07",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "GUITAR",
        class: "SpecialSongFileType",
        prefix: "AI08",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "DRUMS",
        class: "SpecialSongFileType",
        prefix: "AI09",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "KAZOO",
        class: "SpecialSongFileType",
        prefix: "A200",
        extension: ".ogg",
        encrypted: false,
    },
    FileTypeSpec {
        name: "THERAPY",
        class: "SpecialSongFileType",
        prefix: "AA01",
        extension: ".ogg",
        encrypted: false,
    },
    // RingSongFileType
    FileTypeSpec {
        name: "RING_128",
        class: "RingSongFileType",
        prefix: "R500",
        extension: ".mp3",
        encrypted: false,
    },
    FileTypeSpec {
        name: "RING_96",
        class: "RingSongFileType",
        prefix: "R400",
        extension: ".m4a",
        encrypted: false,
    },
    FileTypeSpec {
        name: "RING_48",
        class: "RingSongFileType",
        prefix: "R200",
        extension: ".m4a",
        encrypted: false,
    },
];

/// 参考 `get_song_urls` 的缺省文件类型：`SongFileType.MP3_128`。
fn default_file_type() -> FileTypeSpec {
    file_type_spec("MP3_128").expect("参考的缺省文件类型一定在表里")
}

/// 认一个文件类型：成员名（`MP3_128`）、4 位前缀（`M500`），或
/// `类名.成员名`（`EncryptedSongFileType.FLAC`——同名成员必须这样才能选到加密版）。
fn file_type_spec(value: &str) -> Result<FileTypeSpec, UpstreamError> {
    let text = value.trim();
    if text.is_empty() {
        return Err(file_type_error(value));
    }
    if let Some((class, member)) = text.split_once('.') {
        return FILE_TYPES
            .iter()
            .copied()
            .find(|spec| {
                spec.class.eq_ignore_ascii_case(class) && spec.name.eq_ignore_ascii_case(member)
            })
            .ok_or_else(|| file_type_error(text));
    }
    FILE_TYPES
        .iter()
        .copied()
        .find(|spec| spec.name.eq_ignore_ascii_case(text))
        .or_else(|| {
            FILE_TYPES
                .iter()
                .copied()
                .find(|spec| spec.prefix.eq_ignore_ascii_case(text))
        })
        .ok_or_else(|| file_type_error(text))
}

fn file_type_error(value: &str) -> UpstreamError {
    UpstreamError::Upstream(format!(
        "不支持的文件类型：{value}（可用参考枚举的成员名如 MP3_128、FLAC、OGG_640，\
或 4 位前缀如 M500、F0M0；同名成员分属普通与加密两个类，加密版写成 \
EncryptedSongFileType.FLAC 或直接用它的前缀）"
    ))
}

/// 目标文件名（参考 `get_song_urls` 的拼法）：有 `media_mid` 就用它，
/// 否则把歌曲 mid 写两遍。
fn file_name(spec: FileTypeSpec, mid: &str, media_mid: Option<&str>) -> String {
    match media_mid.map(str::trim).filter(|media| !media.is_empty()) {
        Some(media) => format!("{}{}{}", spec.prefix, media, spec.extension),
        None => format!("{}{}{}{}", spec.prefix, mid, mid, spec.extension),
    }
}

// MARK: - 模型

/// 参考 `SongQueryInfo`：`query_songs` 的单项查询条件.
///
/// `id` 与 `mid` 必须**二选一**（参考的 `ValueError`），`song_type` 缺省 0。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongQueryInfo {
    /// 歌曲 ID.
    pub id: Option<i64>,
    /// 歌曲 MID.
    pub mid: Option<String>,
    /// 歌曲类型.
    pub song_type: Option<i64>,
}

/// 参考 `SongFileInfo`：`resolve_song_urls` 的单项取流条件.
///
/// `file_type` 见 [`SongFileInfo::file_type`] 的说明；不给就用方法级的 `file_type`
/// （缺省 `MP3_128`），`song_type` 缺省 0。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongFileInfo {
    /// 歌曲 MID.
    pub mid: String,
    /// 歌曲文件类型：参考枚举的成员名（`MP3_128` / `FLAC` /
    /// `EncryptedSongFileType.FLAC` / `RingSongFileType.RING_96` …）或 4 位前缀
    /// （`M500` / `F0M0`）。
    pub file_type: Option<String>,
    /// 歌曲类型.
    pub song_type: Option<i64>,
    /// 媒体文件 mid；给了就按它拼文件名（否则把歌曲 mid 写两遍）.
    pub media_mid: Option<String>,
}

/// 参考 `QuerySongResponse`：批量歌曲查询响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuerySongResponse {
    /// 按请求条件返回的曲目列表（复用组件既有的曲目模型）.
    pub tracks: Option<Vec<crate::models::Track>>,
}

/// 参考 `UrlinfoItem`：单个文件的授权结果.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UrlinfoItem {
    /// 歌曲 mid（上游键 `songmid`）.
    pub mid: Option<String>,
    /// 请求的目标文件名.
    pub filename: Option<String>,
    /// 相对下载路径；要与 CDN 域名（`fetch_cdn_dispatch` 的 `sip`）拼接后才能访问.
    pub purl: Option<String>,
    /// 资源访问令牌.
    pub vkey: Option<String>,
    /// 加密资源解密密钥.
    pub ekey: Option<String>,
    /// 单个文件的业务结果码：`0` 成功、`104003` 无权限、`104004` 取票失败、
    /// `104013` 播放设备受限.
    pub result: Option<i64>,
}

/// 参考 `GetSongUrlsResponse`：歌曲播放地址响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetSongUrlsResponse {
    /// 链接过期时间（秒）.
    pub expiration: Option<i64>,
    /// 每个目标文件对应的授权与路径信息（上游键 `midurlinfo`）.
    pub data: Option<Vec<UrlinfoItem>>,
}

/// 参考 `CdnDispatchSipInfo`：CDN 调度中的单个节点信息.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CdnDispatchSipInfo {
    /// CDN 节点地址.
    pub cdn: Option<String>,
    /// 是否支持 QUIC.
    pub quic: Option<i64>,
    /// IP 栈类型.
    pub ipstack: Option<i64>,
    /// QUIC 主机名.
    pub quichost: Option<String>,
    /// 是否支持明文 QUIC（上游键 `plaintextquic`）.
    pub plaintext_quic: Option<i64>,
    /// 是否支持加密 QUIC（上游键 `encryptquic`）.
    pub encrypt_quic: Option<i64>,
}

/// 参考 `GetCdnDispatchResponse`：获取音频 CDN 调度响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetCdnDispatchResponse {
    /// 接口返回码.
    pub retcode: Option<i64>,
    /// 可用 CDN 根地址列表（取流时把 `UrlinfoItem.purl` 拼在它后面）.
    pub sip: Option<Vec<String>>,
    /// 可用 CDN 节点明细列表.
    pub sipinfo: Option<Vec<CdnDispatchSipInfo>>,
    /// 用于测试 CDN 可用性的文件路径（上游键 `keepalivefile`）.
    pub test_file: Option<String>,
    /// 数据有效期（秒）.
    pub expiration: Option<i64>,
    /// 建议刷新间隔（秒）.
    pub refresh_time: Option<i64>,
    /// 建议缓存时长（秒）.
    pub cache_time: Option<i64>,
}

/// 参考 `GetOtherVersionResponse`：获取歌曲其他版本结果.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetOtherVersionResponse {
    /// 其他版本歌曲列表（上游键 `versionList`；复用组件既有的曲目模型）.
    pub data: Option<Vec<crate::models::Track>>,
}

/// 参考 `SongProducer`：歌曲制作人项.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongProducer {
    /// 制作人类型（上游键 `Type`；Rust 侧换个名字、JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// 制作人名称（上游键 `Name`）.
    pub name: Option<String>,
    /// 制作人头像（上游键 `Icon`）.
    pub icon: Option<String>,
    /// 制作人跳转链接（上游键 `Scheme`）.
    pub scheme: Option<String>,
    /// 制作人 singer mid（上游键 `SingerMid`）.
    pub singer_mid: Option<String>,
    /// 关注状态（上游键 `Follow`）.
    pub follow: Option<i64>,
}

/// 参考 `SongProducerGroup`：歌曲制作人信息分组.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongProducerGroup {
    /// 分组标题（上游键 `Title`）.
    pub title: Option<String>,
    /// 该分组下的制作人列表（上游键 `Producers`）.
    pub producers: Option<Vec<SongProducer>>,
    /// 分组类型（上游键 `Type`；JSON 上仍是 `type`）.
    #[serde(rename = "type")]
    pub kind: Option<i64>,
}

/// 参考 `GetProducerResponse`：歌曲制作人响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetProducerResponse {
    /// 按职责分组的制作人列表（上游键 `Lst`）.
    pub data: Option<Vec<SongProducerGroup>>,
    /// 附带的摘要说明文案（上游键 `ReinforceMsg`）.
    pub reinforce_msg: Option<String>,
}

/// 参考 `GetFavNumResponse`：歌曲收藏人数响应.
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetFavNumResponse {
    /// 以歌曲标识为键的收藏人数原始值映射（上游键 `m_numbers`）.
    #[serde(default, deserialize_with = "int_map")]
    pub numbers: Option<HashMap<String, i64>>,
    /// 对应的收藏人数展示文案映射（上游键 `m_show`）.
    #[serde(default, deserialize_with = "text_map")]
    pub show: Option<HashMap<String, String>>,
}

/// 收藏人数：上游给数字也给数字字符串（pydantic 的 `dict[str, int]` 会转），
/// 非数字的值丢掉——那是上游形状变了，不是"这个人有 0 个收藏"。
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

/// 展示文案：上游给字符串也给数字，都收成字符串。
fn text_map<'de, D>(deserializer: D) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Object(entries) => Some(
            entries
                .into_iter()
                .filter_map(|(key, value)| match value {
                    Value::String(text) => Some((key, text)),
                    Value::Number(number) => Some((key, number.to_string())),
                    _ => None,
                })
                .collect(),
        ),
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

// MARK: - 回值搬运（上游形状 → 参考模型形状）
//
// 参考模型里带缺省的字段按缺省给出（列表给 `[]`、字符串给 `""`）；必填字段在
// Rust 侧是 `Option`——上游缺了就是 `null`，不假装有值。

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

/// 批量歌曲查询回值（参考 `QuerySongResponse`）。
///
/// 曲目本体复用组件既有的曲目解码（`methods::decoded_tracks`，候选列表键已覆盖
/// `tracks`/`songlist`/`songList` 等拼写）。
fn query_song_payload(data: &Value) -> Value {
    json!({ "tracks": crate::methods::decoded_tracks(data) })
}

/// 单个 CDN 节点（参考 `CdnDispatchSipInfo`）。
fn cdn_sip_payload(raw: &Value) -> Value {
    json!({
        "cdn": first_text(raw, &["cdn"]),
        "quic": first_int(raw, &["quic"]),
        "ipstack": first_int(raw, &["ipstack"]),
        "quichost": first_text(raw, &["quichost"]),
        "plaintextQuic": first_int(raw, &["plaintextquic", "plaintextQuic"]),
        "encryptQuic": first_int(raw, &["encryptquic", "encryptQuic"]),
    })
}

/// CDN 调度回值（参考 `GetCdnDispatchResponse`）：`testFile` 在参考模型里的别名
/// 是上游的 `keepalivefile`。
fn cdn_dispatch_payload(data: &Value) -> Value {
    json!({
        "retcode": first_int(data, &["retcode"]),
        "sip": string_list(data, &["sip"]).unwrap_or_default(),
        "sipinfo": first_array(data, &["sipinfo"])
            .map(|items| items.iter().map(cdn_sip_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
        "testFile": first_text(data, &["keepalivefile", "testFile"]),
        "expiration": first_int(data, &["expiration"]),
        "refreshTime": first_int(data, &["refreshTime", "refresh_time"]),
        "cacheTime": first_int(data, &["cacheTime", "cache_time"]),
    })
}

/// 单个文件的授权结果（参考 `UrlinfoItem`）。
fn url_info_payload(raw: &Value) -> Value {
    json!({
        "mid": first_text(raw, &["songmid", "songMid", "mid"]),
        "filename": first_text(raw, &["filename"]),
        "purl": first_text(raw, &["purl", "url"]),
        "vkey": first_text(raw, &["vkey"]),
        "ekey": first_text(raw, &["ekey"]),
        "result": first_int(raw, &["result"]),
    })
}

/// 取流回值（参考 `GetSongUrlsResponse`）：列表在 `midurlinfo` 上。
fn song_urls_payload(data: &Value) -> Value {
    json!({
        "expiration": first_int(data, &["expiration"]),
        "data": first_array(data, &["midurlinfo", "data"])
            .map(|items| items.iter().map(url_info_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

/// 其他版本回值（参考 `GetOtherVersionResponse`）：列表在 `versionList` 上。
fn other_version_payload(data: &Value) -> Value {
    json!({
        "data": first_array(data, &["versionList", "version_list", "songList", "list"])
            .map(|items| crate::methods::decoded_tracks(&json!({ "songList": items })))
            .unwrap_or_default(),
    })
}

/// 单个制作人（参考 `SongProducer`）：上游的键首字母大写。
fn producer_payload(raw: &Value) -> Value {
    json!({
        "type": first_int(raw, &["Type", "type"]),
        "name": first_text(raw, &["Name", "name"]),
        "icon": first_text(raw, &["Icon", "icon"]),
        "scheme": first_text(raw, &["Scheme", "scheme"]),
        "singerMid": first_text(raw, &["SingerMid", "singerMid", "singer_mid"]),
        "follow": first_int(raw, &["Follow", "follow"]),
    })
}

/// 一个制作人分组（参考 `SongProducerGroup`）。
fn producer_group_payload(raw: &Value) -> Value {
    json!({
        "title": first_text(raw, &["Title", "title"]),
        "producers": first_array(raw, &["Producers", "producers"])
            .map(|items| items.iter().map(producer_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
        "type": first_int(raw, &["Type", "type"]),
    })
}

/// 制作人回值（参考 `GetProducerResponse`）：分组在 `Lst` 上，文案在 `ReinforceMsg`。
fn producer_response_payload(data: &Value) -> Value {
    json!({
        "data": first_array(data, &["Lst", "lst"])
            .map(|items| items.iter().map(producer_group_payload).collect::<Vec<_>>())
            .unwrap_or_default(),
        "reinforceMsg": first_text(data, &["ReinforceMsg", "reinforceMsg"]).unwrap_or_default(),
    })
}

/// 收藏数回值（参考 `GetFavNumResponse`）。
fn fav_num_payload(data: &Value) -> Value {
    json!({
        "numbers": data.get("m_numbers").cloned().unwrap_or(Value::Null),
        "show": data.get("m_show").cloned().unwrap_or(Value::Null),
    })
}

// MARK: - 请求参数（照参考逐字拼）

/// 每请求现生成的 `guid`：参考的 `get_guid()` 就是 `uuid4().hex`——
/// 32 位小写十六进制。CDN 调度与取流都把它当设备号。
fn guid() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 批量查询参数（参考 `query_song`）。
///
/// 每一项必须给 `id` 或 `mid` 之一且不能同给（参考的 `ValueError`）；`types` 与
/// `modify_stamp` 等长，`ids`/`mids` 只在有值时出现——照参考。
fn query_params(params: &Value) -> Result<Value, UpstreamError> {
    let items = first_array(
        params,
        &[
            "songs",
            "songInfo",
            "song_info",
            "items",
            "queries",
            "tracks",
        ],
    )
    .ok_or_else(|| UpstreamError::Upstream("缺少 song_info（歌曲查询信息列表）".into()))?;
    if items.is_empty() {
        return Err(UpstreamError::Upstream("song_info 不能为空".into()));
    }

    let mut ids: Vec<i64> = Vec::new();
    let mut mids: Vec<String> = Vec::new();
    let mut types: Vec<i64> = Vec::new();
    for item in items {
        let id = first_int(item, &["id", "songId", "songid"]);
        let mid = first_text(item, &["mid", "songMid", "songmid"]);
        // 参考的判据：`(id is None) == (mid is None)`——两个都给或都不给都拒。
        match (id, mid) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(UpstreamError::Upstream(
                    "SongQueryInfo 必须提供 id 或 mid 且不能同时提供".into(),
                ))
            }
            (Some(id), None) => ids.push(id),
            (None, Some(mid)) => mids.push(mid),
        }
        // 参考的 `item.song_type or 0`：缺省与 0 都发 0。
        types.push(first_int(item, &["songType", "song_type"]).unwrap_or(0));
    }

    let count = types.len();
    let mut param = json!({
        "ctx": 0,
        "client": 1,
        "types": types,
        "modify_stamp": vec![0i64; count],
    });
    if !ids.is_empty() {
        param["ids"] = json!(ids);
    }
    if !mids.is_empty() {
        param["mids"] = json!(mids);
    }
    Ok(param)
}

/// CDN 调度参数（参考 `get_cdn_dispatch`）：`uid` 是字面量 `"0"`。
fn cdn_dispatch_params() -> Value {
    json!({
        "guid": guid(),
        "uid": "0",
        "use_new_domain": 1,
        "use_ipv6": 1,
    })
}

/// 参考按**顶层** `file_type` 是否属于 `EncryptedSongFileType` 选端点
/// （`isinstance(file_type, EncryptedSongFileType)`）。
///
/// 注意：项里各自指定的加密类型**不参与**这个判断，参考就是如此——顶层给普通档
/// 时，即使某项要的是 `.mflac`，也仍打 `GetVkey`。
fn url_endpoint(encrypted: bool) -> (&'static str, &'static str) {
    if encrypted {
        ("music.vkey.GetEVkey", "CgiGetEVkey")
    } else {
        ("music.vkey.GetVkey", "UrlGetVkey")
    }
}

/// 方法级的文件类型：不给就按参考的缺省 `MP3_128`。
fn top_file_type(params: &Value) -> Result<FileTypeSpec, UpstreamError> {
    match first_text(params, &["fileType", "file_type"])
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    {
        Some(name) => file_type_spec(&name),
        None => Ok(default_file_type()),
    }
}

/// 请求里的单项取流条件（参考 `SongFileInfo` 的字段）。
#[derive(Debug)]
struct RequestedSongFile {
    mid: String,
    file_type: Option<String>,
    song_type: i64,
    media_mid: Option<String>,
}

/// 解析 `resolve_song_urls` 的请求项列表。
fn requested_files(params: &Value) -> Result<Vec<RequestedSongFile>, UpstreamError> {
    let items = first_array(
        params,
        &["fileInfo", "file_info", "files", "items", "songs"],
    )
    .ok_or_else(|| UpstreamError::Upstream("缺少 fileInfo（歌曲文件信息列表）".into()))?;
    let mut files = Vec::with_capacity(items.len());
    for item in items {
        let mid = first_text(item, &["mid", "songMid", "songmid"])
            .filter(|mid| !mid.trim().is_empty())
            .ok_or_else(|| UpstreamError::Upstream("SongFileInfo 缺少 mid".into()))?;
        files.push(RequestedSongFile {
            mid,
            file_type: first_text(item, &["fileType", "file_type"]),
            song_type: first_int(item, &["songType", "song_type"]).unwrap_or(0),
            media_mid: first_text(item, &["mediaMid", "media_mid"]),
        });
    }
    Ok(files)
}

/// 参考的上限检查：超限时上游回错误且无结果，所以提前拒掉。
fn check_mid_limit(count: usize) -> Result<(), UpstreamError> {
    if count > GET_SONG_URLS_MAX_MID {
        Err(UpstreamError::Upstream(format!(
            "mid 数量不能超过 {GET_SONG_URLS_MAX_MID}，当前为 {count}"
        )))
    } else {
        Ok(())
    }
}

/// 取流参数（参考 `get_song_urls`）：`uin` 是凭据里的字符串 music id，
/// `songmid`/`filename`/`songtype` 三个数组一一对应。
fn song_url_params(
    files: &[RequestedSongFile],
    top: FileTypeSpec,
    uin: &str,
) -> Result<Value, UpstreamError> {
    let mut songmid: Vec<&str> = Vec::with_capacity(files.len());
    let mut filename: Vec<String> = Vec::with_capacity(files.len());
    let mut songtype: Vec<i64> = Vec::with_capacity(files.len());
    for item in files {
        let spec = match item
            .file_type
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            Some(name) => file_type_spec(name)?,
            None => top,
        };
        songmid.push(item.mid.as_str());
        filename.push(file_name(spec, &item.mid, item.media_mid.as_deref()));
        songtype.push(item.song_type);
    }
    Ok(json!({
        "uin": uin,
        "filename": filename,
        "guid": guid(),
        "songmid": songmid,
        "songtype": songtype,
        "ctx": 0,
    }))
}

/// 取流用的档案：调用方显式给了 `platform` 就用它，否则 android。
///
/// 取流依赖设备会话（机器指纹 + `GetSession` 换来的 `uid`/`sid`），那套在 android
/// 档案的 `comm` 里；与既有 `resolve_song_url` 同款处理。
fn url_platform(params: &Value, platform: Platform) -> Platform {
    if params.get("platform").is_some() {
        platform
    } else {
        Platform::Android
    }
}

/// 参考的 `value: int | str`：十进制字符串当歌曲 ID，其余当 MID。
///
/// 这是给 `fetch_other_versions` / `fetch_song_producer` 的 `value` 用的；
/// 显式的 `songId` / `songMid` 键按调用方的意思走，不再二次判断。
fn identifier_params(params: &Value) -> Result<Value, UpstreamError> {
    if let Some(id) = first_int(params, &["songId", "songid", "id"]) {
        return Ok(json!({ "songid": id }));
    }
    if let Some(mid) =
        first_text(params, &["songMid", "songmid", "mid"]).filter(|mid| !mid.trim().is_empty())
    {
        return Ok(json!({ "songmid": mid }));
    }
    let value = first_text(params, &["value"])
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| UpstreamError::Upstream("缺少歌曲 ID 或 MID".into()))?;
    if is_decimal(&value) {
        let id: i64 = value
            .parse()
            .map_err(|_| UpstreamError::Upstream(format!("歌曲 ID 超出范围：{value}")))?;
        Ok(json!({ "songid": id }))
    } else {
        Ok(json!({ "songmid": value }))
    }
}

/// 参考的 `str.isdecimal()`：全为十进制数字（没有符号、没有空白）。
fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

/// 一组整数：上游 / 调用方可能给数字或数字字符串，非整数要报错（参考的
/// `list[int]` 会拒绝）。
fn required_int_list(raw: &Value, keys: &[&str], what: &str) -> Result<Vec<i64>, UpstreamError> {
    let items =
        first_array(raw, keys).ok_or_else(|| UpstreamError::Upstream(format!("缺少{what}")))?;
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let number = coerce_int(item)
            .ok_or_else(|| UpstreamError::Upstream(format!("{what}里有非整数：{item}")))?;
        values.push(number);
    }
    Ok(values)
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
        "query_songs" => query_song(upstream, credential, platform, params),
        "fetch_cdn_dispatch" => cdn_dispatch(upstream, credential, platform),
        "resolve_song_urls" => song_urls(upstream, credential, platform, params),
        "fetch_other_versions" => other_versions(upstream, credential, platform, params),
        "fetch_song_producer" => song_producer(upstream, credential, platform, params),
        "fetch_song_fav_count" => song_fav_count(upstream, credential, platform, params),
        _ => return None,
    };
    Some(result)
}

/// 批量歌曲信息（`music.trackInfo.UniformRuleCtrl / CgiGetTrackInfo`）。
///
/// 空 `tracks` 是答案：这批 id/mid 上游没有曲目，照常返回。
fn query_song(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = query_params(params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.trackInfo.UniformRuleCtrl",
            method: "CgiGetTrackInfo",
            param,
        },
    )?;
    Ok(query_song_payload(&data))
}

/// 音频 CDN 调度（`music.audioCdnDispatch.cdnDispatch / GetCdnDispatch`）。
fn cdn_dispatch(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
) -> Result<Value, UpstreamError> {
    let data = upstream.call_with(
        credential,
        Class::Playback,
        platform,
        Call {
            module: "music.audioCdnDispatch.cdnDispatch",
            method: "GetCdnDispatch",
            param: cdn_dispatch_params(),
        },
    )?;
    Ok(cdn_dispatch_payload(&data))
}

/// 批量取流（`music.vkey.GetVkey / UrlGetVkey`，加密档走 `GetEVkey`）。
///
/// 每一项的 `result` 是答案：无权限 / 设备受限都会照常回一个结果码。
/// `purl` 是相对路径，CDN 根地址来自 `fetch_cdn_dispatch` 的 `sip`。
fn song_urls(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let files = requested_files(params)?;
    check_mid_limit(files.len())?;
    let top = top_file_type(params)?;
    let param = song_url_params(&files, top, &credential.music_id)?;
    let (module, method) = url_endpoint(top.encrypted);
    let data = upstream.call_with(
        credential,
        Class::Playback,
        url_platform(params, platform),
        Call {
            module,
            method,
            param,
        },
    )?;
    Ok(song_urls_payload(&data))
}

/// 歌曲其他版本（`music.musichallSong.OtherVersionServer / GetOtherVersionSongs`）。
///
/// 空列表是答案：这首歌本来就没有别的版本。
fn other_versions(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = identifier_params(params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.musichallSong.OtherVersionServer",
            method: "GetOtherVersionSongs",
            param,
        },
    )?;
    Ok(other_version_payload(&data))
}

/// 歌曲制作人（`music.sociality.KolWorksTag / SongProducer`）。
///
/// 空列表是答案：上游对这首歌没有登记制作人信息。
fn song_producer(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let param = identifier_params(params)?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.sociality.KolWorksTag",
            method: "SongProducer",
            param,
        },
    )?;
    Ok(producer_response_payload(&data))
}

/// 歌曲收藏数（`music.musicasset.SongFavRead / GetSongFansNumberById`）。
///
/// 空映射是答案：这批 id 上游没有计数。
fn song_fav_count(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    let ids = required_int_list(
        params,
        &["songIds", "song_ids", "v_songId", "ids"],
        "songIds（歌曲 ID 列表）",
    )?;
    let data = upstream.call_with(
        credential,
        Class::Read,
        platform,
        Call {
            module: "music.musicasset.SongFavRead",
            method: "GetSongFansNumberById",
            param: json!({ "v_songId": ids }),
        },
    )?;
    Ok(fav_num_payload(&data))
}

// MARK: - 宿主包装

/// 批量获取歌曲信息；每项给 `id` 或 `mid` 之一，`songType` 缺省 0。
#[export]
pub fn query_songs(songs: Vec<SongQueryInfo>) -> Result<QuerySongResponse, crate::HelperError> {
    crate::port::call("query_songs", json!({ "songs": songs }))
}

/// 取音频 CDN 调度信息；`sip` 里的根地址用来拼取流回值的 `purl`。
#[export]
pub fn fetch_cdn_dispatch() -> Result<GetCdnDispatchResponse, crate::HelperError> {
    crate::port::call("fetch_cdn_dispatch", json!({}))
}

/// 批量取流；`file_type` 是参考枚举的成员名或 4 位前缀，缺省 `MP3_128`。
/// 一次最多 100 个 mid，超了在发请求前报错。
#[export]
pub fn resolve_song_urls(
    file_info: Vec<SongFileInfo>,
    file_type: Option<String>,
) -> Result<GetSongUrlsResponse, crate::HelperError> {
    let mut params = json!({ "fileInfo": file_info });
    if let Some(file_type) = file_type {
        params["fileType"] = json!(file_type);
    }
    crate::port::call("resolve_song_urls", params)
}

/// 歌曲其他版本；`value` 全为数字时按歌曲 ID 走，否则按 MID。
#[export]
pub fn fetch_other_versions(value: String) -> Result<GetOtherVersionResponse, crate::HelperError> {
    crate::port::call("fetch_other_versions", json!({ "value": value }))
}

/// 歌曲制作人；`value` 全为数字时按歌曲 ID 走，否则按 MID。
#[export]
pub fn fetch_song_producer(value: String) -> Result<GetProducerResponse, crate::HelperError> {
    crate::port::call("fetch_song_producer", json!({ "value": value }))
}

/// 歌曲收藏数；回值的 `numbers` 以歌曲 ID 为键。
#[export]
pub fn fetch_song_fav_count(song_ids: Vec<i64>) -> Result<GetFavNumResponse, crate::HelperError> {
    crate::port::call("fetch_song_fav_count", json!({ "songIds": song_ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份批量查询回值：键名照上游自己的拼写。
    fn raw_tracks() -> Value {
        json!({
            "tracks": [{
                "id": 2314161,
                "mid": "003w2xz20QlUZt",
                "type": 1,
                "name": "歌名",
                "singer": [{ "id": 1, "mid": "0025NhlN2yWrP4", "name": "甲" }],
                "album": { "id": 42, "mid": "0041WVfh2vtlJE", "name": "专辑" },
                "interval": 240,
                "pay": { "pay_play": 1, "pay_down": 0 }
            }]
        })
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 6);
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "resolve_song_url",
            &json!({})
        )
        .is_none());
    }

    #[test]
    fn a_query_list_needs_one_identifier_per_item() {
        let param = query_params(&json!({
            "songs": [
                { "id": 2314161 },
                { "mid": "003w2xz20QlUZt", "songType": 1 },
                { "mid": "001Qu4I30eVFYb", "song_type": 113 }
            ]
        }))
        .expect("三种写法都收");
        assert_eq!(param["ctx"], 0);
        assert_eq!(param["client"], 1);
        assert_eq!(param["ids"], json!([2314161]));
        assert_eq!(param["mids"], json!(["003w2xz20QlUZt", "001Qu4I30eVFYb"]));
        assert_eq!(param["types"], json!([0, 1, 113]));
        assert_eq!(param["modify_stamp"], json!([0, 0, 0]));

        // 只给 id 时没有 mids，反之亦然——参考就是「有才带」。
        let param = query_params(&json!({ "songs": [{ "mid": "003w2xz20QlUZt" }] })).unwrap();
        assert!(param.get("ids").is_none());
        assert_eq!(param["mids"], json!(["003w2xz20QlUZt"]));

        let error = query_params(&json!({ "songs": [] })).expect_err("空列表要报错");
        assert!(error.to_string().contains("不能为空"), "{error}");
        let error = query_params(&json!({ "songs": [{ "id": 1, "mid": "003w" }] }))
            .expect_err("同给要报错");
        assert!(error.to_string().contains("不能同时提供"), "{error}");
        let error = query_params(&json!({ "songs": [{}] })).expect_err("都不给要报错");
        assert!(error.to_string().contains("不能同时提供"), "{error}");
    }

    #[test]
    fn a_query_response_decodes_the_tracks_it_was_asked_for() {
        let payload = query_song_payload(&raw_tracks());
        let response: QuerySongResponse = serde_json::from_value(payload.clone()).expect("解析");
        let tracks = response.tracks.expect("有曲目");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].song_mid, "003w2xz20QlUZt");
        assert_eq!(tracks[0].song_id, Some(2314161));
        assert_eq!(tracks[0].title, "歌名");
        assert_eq!(tracks[0].artist, "甲");
        assert_eq!(tracks[0].album_id, Some(42));
        assert_eq!(tracks[0].album_mid.as_deref(), Some("0041WVfh2vtlJE"));
        assert_eq!(tracks[0].duration, Some(240));
        assert_eq!(tracks[0].pay_play, Some(1));
        assert_eq!(
            payload["tracks"][0]["singerMid"], "0025NhlN2yWrP4",
            "宿主按组件既有的曲目字段读"
        );

        // 上游没有曲目时是空列表，不是错误。
        let response: QuerySongResponse =
            serde_json::from_value(query_song_payload(&json!({}))).expect("解析空回值");
        assert!(response.tracks.expect("字段仍在").is_empty());
    }

    #[test]
    fn the_cdn_dispatch_request_and_response_are_the_reference_ones() {
        let param = cdn_dispatch_params();
        assert_eq!(param["uid"], "0");
        assert_eq!(param["use_new_domain"], 1);
        assert_eq!(param["use_ipv6"], 1);
        let guid = param["guid"].as_str().expect("guid 是字符串");
        assert_eq!(guid.len(), 32, "参考的 get_guid 是 uuid4().hex");
        assert!(guid.chars().all(|ch| ch.is_ascii_hexdigit()));

        let payload = cdn_dispatch_payload(&json!({
            "retcode": 0,
            "sip": ["https://isure.stream.qqmusic.qq.com/"],
            "sipinfo": [{
                "cdn": "isure.stream.qqmusic.qq.com",
                "quic": 0,
                "ipstack": 1,
                "quichost": "",
                "plaintextquic": 1,
                "encryptquic": 0
            }],
            "keepalivefile": "https://isure.stream.qqmusic.qq.com/keepalive",
            "expiration": 3600,
            "refreshTime": 600,
            "cacheTime": 120
        }));
        let response: GetCdnDispatchResponse = serde_json::from_value(payload).expect("解析");
        assert_eq!(response.retcode, Some(0));
        assert_eq!(
            response.sip.as_deref(),
            Some(&["https://isure.stream.qqmusic.qq.com/".to_string()][..])
        );
        assert_eq!(
            response.test_file.as_deref(),
            Some("https://isure.stream.qqmusic.qq.com/keepalive")
        );
        assert_eq!(response.expiration, Some(3600));
        assert_eq!(response.refresh_time, Some(600));
        assert_eq!(response.cache_time, Some(120));
        let sipinfo = response.sipinfo.expect("有节点");
        assert_eq!(sipinfo[0].ipstack, Some(1));
        assert_eq!(sipinfo[0].plaintext_quic, Some(1));
        assert_eq!(sipinfo[0].encrypt_quic, Some(0));
    }

    #[test]
    fn every_reference_file_type_is_in_the_table() {
        assert_eq!(FILE_TYPES.len(), 48, "17 普通 + 13 加密 + 15 特殊 + 3 彩铃");
        for spec in FILE_TYPES {
            assert!(!spec.name.is_empty());
            assert_eq!(spec.prefix.len(), 4, "{}", spec.name);
            assert!(spec.extension.starts_with('.'), "{}", spec.name);
        }
        let default = default_file_type();
        assert_eq!(
            (
                default.name,
                default.prefix,
                default.extension,
                default.encrypted
            ),
            ("MP3_128", "M500", ".mp3", false)
        );
    }

    #[test]
    fn a_file_type_is_found_by_name_class_or_prefix() {
        assert_eq!(file_type_spec("MP3_128").unwrap().prefix, "M500");
        assert_eq!(
            file_type_spec("flac").unwrap().prefix,
            "F000",
            "大小写不敏感"
        );
        assert_eq!(
            file_type_spec("EncryptedSongFileType.FLAC").unwrap(),
            FileTypeSpec {
                name: "FLAC",
                class: "EncryptedSongFileType",
                prefix: "F0M0",
                extension: ".mflac",
                encrypted: true,
            }
        );
        assert_eq!(file_type_spec("F0M0").unwrap().prefix, "F0M0", "前缀也行");
        assert!(file_type_spec("F0M0").unwrap().encrypted);
        assert_eq!(
            file_type_spec("RingSongFileType.RING_96")
                .unwrap()
                .extension,
            ".m4a"
        );
        assert_eq!(file_type_spec("O802").unwrap().name, "TRY_OGG_640");
        let error = file_type_spec("HIRES").expect_err("认不出要报错");
        assert!(error.to_string().contains("不支持的文件类型"), "{error}");
    }

    #[test]
    fn a_filename_is_the_prefix_plus_the_mid() {
        let flac = file_type_spec("FLAC").unwrap();
        assert_eq!(
            file_name(flac, "003w2xz20QlUZt", None),
            "F000003w2xz20QlUZt003w2xz20QlUZt.flac",
            "没有 mediaMid 时 mid 写两遍（参考如此）"
        );
        assert_eq!(
            file_name(flac, "003w2xz20QlUZt", Some("001Qu4I30eVFYb")),
            "F000001Qu4I30eVFYb.flac"
        );
        assert_eq!(
            file_name(flac, "003w2xz20QlUZt", Some("  ")),
            "F000003w2xz20QlUZt003w2xz20QlUZt.flac",
            "空白 media_mid 按没给（参考的 `not item.media_mid`）"
        );
    }

    #[test]
    fn the_song_urls_request_carries_the_reference_fields() {
        let param = song_url_params(
            &[
                RequestedSongFile {
                    mid: "003w2xz20QlUZt".into(),
                    file_type: None,
                    song_type: 0,
                    media_mid: None,
                },
                RequestedSongFile {
                    mid: "001Qu4I30eVFYb".into(),
                    file_type: Some("FLAC".into()),
                    song_type: 1,
                    media_mid: Some("001Qu4I30eVFYb".into()),
                },
            ],
            default_file_type(),
            "12345",
        )
        .expect("拼参数");
        assert_eq!(param["uin"], "12345");
        assert_eq!(param["ctx"], 0);
        assert_eq!(param["songmid"][0], "003w2xz20QlUZt");
        assert_eq!(param["songtype"], json!([0, 1]));
        assert_eq!(
            param["filename"][0], "M500003w2xz20QlUZt003w2xz20QlUZt.mp3",
            "缺省档是 MP3_128"
        );
        assert_eq!(param["filename"][1], "F000001Qu4I30eVFYb.flac");
        let guid = param["guid"].as_str().expect("guid 是字符串");
        assert_eq!(guid.len(), 32);
    }

    #[test]
    fn an_encrypted_top_level_file_type_switches_the_endpoint() {
        assert_eq!(url_endpoint(false), ("music.vkey.GetVkey", "UrlGetVkey"));
        assert_eq!(url_endpoint(true), ("music.vkey.GetEVkey", "CgiGetEVkey"));
        // 顶层给加密档才算加密；项里的加密类型不参与（参考只判断顶层）。
        let top = file_type_spec("EncryptedSongFileType.FLAC").unwrap();
        assert!(top.encrypted);
        assert_eq!(top_file_type(&json!({})).unwrap(), default_file_type());
        assert_eq!(
            top_file_type(&json!({ "fileType": "EncryptedSongFileType.FLAC" })).unwrap(),
            top
        );
    }

    #[test]
    fn more_than_a_hundred_mids_is_refused_before_any_request() {
        assert!(
            check_mid_limit(100).is_ok(),
            "恰好 100 个允许（参考比的是 >）"
        );
        let error = check_mid_limit(101).expect_err("超限要报错");
        assert!(error.to_string().contains("100"), "{error}");

        let files: Vec<Value> = (0..101)
            .map(|index| json!({ "mid": format!("mid-{index}") }))
            .collect();
        let upstream = Upstream::new();
        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "resolve_song_urls",
            &json!({ "fileInfo": files }),
        )
        .expect("是本层方法")
        .expect_err("超限在发请求前就要报错");
        assert!(error.to_string().contains("mid 数量不能超过"), "{error}");
    }

    #[test]
    fn a_missing_file_info_is_refused_before_any_request() {
        let upstream = Upstream::new();
        let error = dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "resolve_song_urls",
            &json!({}),
        )
        .expect("是本层方法")
        .expect_err("没有 fileInfo 要报错");
        assert!(error.to_string().contains("fileInfo"), "{error}");
    }

    #[test]
    fn an_urls_response_maps_every_midurlinfo_field() {
        let payload = song_urls_payload(&json!({
            "expiration": 7200,
            "midurlinfo": [{
                "songmid": "003w2xz20QlUZt",
                "filename": "M500003w2xz20QlUZt003w2xz20QlUZt.mp3",
                "purl": "M500003w2xz20QlUZt003w2xz20QlUZt.mp3?vkey=ABC",
                "vkey": "ABC",
                "ekey": "",
                "result": 0
            }]
        }));
        let response: GetSongUrlsResponse = serde_json::from_value(payload.clone()).expect("解析");
        assert_eq!(response.expiration, Some(7200));
        let items = response.data.expect("有结果");
        assert_eq!(items[0].mid.as_deref(), Some("003w2xz20QlUZt"));
        assert_eq!(
            items[0].filename.as_deref(),
            Some("M500003w2xz20QlUZt003w2xz20QlUZt.mp3")
        );
        assert_eq!(items[0].result, Some(0));
        assert_eq!(items[0].vkey.as_deref(), Some("ABC"));
        assert_eq!(payload["data"][0]["mid"], "003w2xz20QlUZt");

        // 无权限也是答案：结果码照常给，不是错误。
        let response: GetSongUrlsResponse = serde_json::from_value(song_urls_payload(&json!({
            "midurlinfo": [{ "songmid": "m", "result": 104003 }]
        })))
        .expect("解析被拒的回值");
        let items = response.data.expect("有结果");
        assert_eq!(items[0].result, Some(104003));
        assert!(items[0].purl.is_none());

        let response: GetSongUrlsResponse =
            serde_json::from_value(song_urls_payload(&json!({}))).expect("解析空回值");
        assert!(response.data.expect("字段仍在").is_empty());
        assert_eq!(response.expiration, None);
    }

    #[test]
    fn other_versions_come_out_of_version_list() {
        let payload = other_version_payload(&json!({ "versionList": raw_tracks()["tracks"] }));
        let response: GetOtherVersionResponse =
            serde_json::from_value(payload.clone()).expect("解析");
        let versions = response.data.expect("有版本");
        assert_eq!(versions[0].song_mid, "003w2xz20QlUZt");
        assert_eq!(versions[0].title, "歌名");
        assert_eq!(payload["data"][0]["songMid"], "003w2xz20QlUZt");

        // 没有其他版本时是空列表（参考的 default_factory），不是错误。
        let response: GetOtherVersionResponse =
            serde_json::from_value(other_version_payload(&json!({}))).expect("解析空回值");
        assert!(response.data.expect("字段仍在").is_empty());
    }

    #[test]
    fn producers_come_out_of_the_uppercase_lst_groups() {
        let payload = producer_response_payload(&json!({
            "Lst": [{
                "Title": "制作人",
                "Type": 1,
                "Producers": [{
                    "Type": 3,
                    "Name": "张三",
                    "Icon": "https://y.gtimg.cn/a.jpg",
                    "Scheme": "qqmusic://singer/0025NhlN2yWrP4",
                    "SingerMid": "0025NhlN2yWrP4",
                    "Follow": 1
                }]
            }],
            "ReinforceMsg": "以上信息由用户提供"
        }));
        let response: GetProducerResponse = serde_json::from_value(payload).expect("解析");
        assert_eq!(
            response.reinforce_msg.as_deref(),
            Some("以上信息由用户提供")
        );
        let groups = response.data.expect("有分组");
        assert_eq!(groups[0].title.as_deref(), Some("制作人"));
        assert_eq!(groups[0].kind, Some(1));
        let producers = groups[0].producers.as_ref().expect("有制作人");
        assert_eq!(producers[0].name.as_deref(), Some("张三"));
        assert_eq!(producers[0].kind, Some(3));
        assert_eq!(producers[0].singer_mid.as_deref(), Some("0025NhlN2yWrP4"));
        assert_eq!(producers[0].follow, Some(1));

        // 上游没有制作人时是空列表，文案给空串（参考的缺省）。
        let response: GetProducerResponse =
            serde_json::from_value(producer_response_payload(&json!({}))).expect("解析空回值");
        assert!(response.data.expect("字段仍在").is_empty());
        assert_eq!(response.reinforce_msg.as_deref(), Some(""));
    }

    #[test]
    fn favourite_counts_come_out_of_the_m_underscore_maps() {
        let payload = fav_num_payload(&json!({
            "m_numbers": { "100": 4321, "2314161": "1234" },
            "m_show": { "100": "4321人收藏", "2314161": 1234 }
        }));
        let response: GetFavNumResponse = serde_json::from_value(payload.clone()).expect("解析");
        let numbers = response.numbers.expect("有计数");
        assert_eq!(numbers.get("100"), Some(&4321));
        assert_eq!(numbers.get("2314161"), Some(&1234), "数字字符串也收");
        let show = response.show.expect("有文案");
        assert_eq!(show.get("100").map(String::as_str), Some("4321人收藏"));
        assert_eq!(show.get("2314161").map(String::as_str), Some("1234"));

        let response: GetFavNumResponse =
            serde_json::from_value(fav_num_payload(&json!({}))).expect("解析空回值");
        assert!(response.numbers.is_none());
        assert!(response.show.is_none());
    }

    #[test]
    fn the_generic_song_value_is_an_id_or_a_mid() {
        assert_eq!(
            identifier_params(&json!({ "value": "100" })).unwrap(),
            json!({ "songid": 100 })
        );
        assert_eq!(
            identifier_params(&json!({ "value": "003w2xz20QlUZt" })).unwrap(),
            json!({ "songmid": "003w2xz20QlUZt" })
        );
        assert_eq!(
            identifier_params(&json!({ "songMid": "001Qu4I30eVFYb" })).unwrap(),
            json!({ "songmid": "001Qu4I30eVFYb" })
        );
        assert_eq!(
            identifier_params(&json!({ "songId": 2314161 })).unwrap(),
            json!({ "songid": 2314161 })
        );
        // 参考的 isdecimal()：带符号的不算 ID。
        assert_eq!(
            identifier_params(&json!({ "value": "-5" })).unwrap(),
            json!({ "songmid": "-5" })
        );
        assert!(identifier_params(&json!({})).is_err(), "两者都缺要报错");
        assert!(is_decimal("100") && !is_decimal("") && !is_decimal("1a"));
    }

    #[test]
    fn the_favourite_request_sends_v_songid() {
        assert_eq!(
            required_int_list(
                &json!({ "songIds": [2314161, 100] }),
                &["songIds", "song_ids", "v_songId", "ids"],
                "songIds"
            )
            .unwrap(),
            vec![2314161, 100]
        );
        assert_eq!(
            required_int_list(&json!({ "songIds": ["3"] }), &["songIds"], "songIds").unwrap(),
            vec![3]
        );
        assert!(required_int_list(&json!({}), &["songIds"], "songIds").is_err());
        assert!(
            required_int_list(&json!({ "songIds": ["abc"] }), &["songIds"], "songIds").is_err()
        );
    }

    #[test]
    fn the_url_platform_defaults_to_android_unless_the_caller_chose() {
        assert_eq!(url_platform(&json!({}), Platform::Web), Platform::Android);
        assert_eq!(
            url_platform(&json!({ "platform": "web" }), Platform::Web),
            Platform::Web,
            "调用方显式给了就尊重"
        );
        assert_eq!(
            url_platform(&json!({ "platform": "android" }), Platform::Android),
            Platform::Android
        );
    }

    #[test]
    fn a_file_entry_without_a_mid_is_refused() {
        let error = requested_files(&json!({ "fileInfo": [{ "songType": 1 }] }))
            .expect_err("缺 mid 要报错");
        assert!(error.to_string().contains("mid"), "{error}");
        let files = requested_files(&json!({
            "fileInfo": [{ "mid": "003w2xz20QlUZt", "fileType": "FLAC", "songType": 2, "mediaMid": "m" }]
        }))
        .expect("解析");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].song_type, 2);
        assert_eq!(files[0].media_mid.as_deref(), Some("m"));
        assert_eq!(files[0].file_type.as_deref(), Some("FLAC"));
        // 一个空的请求项列表是允许的：不是 `song_info` 那种"整表读取"。
        assert!(requested_files(&json!({ "fileInfo": [] }))
            .unwrap()
            .is_empty());
    }
}

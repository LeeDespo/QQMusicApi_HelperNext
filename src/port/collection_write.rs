//! 集合写入：自建歌单的增删、歌单歌曲的增删，以及收藏专辑的加/取消。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/songlist.py`
//! （`create` / `delete` / `add_songs` / `del_songs`）与 `modules/album.py`
//! （`fav_album` / `del_fav_album`）；回值字段名照 `models/songlist.py` 的
//! `CreateDeleteSonglistResp` 与 `models/album.py` 的 `AlbumFavWriteResponse`
//! （camelCase）。
//!
//! 八个端点：
//!
//! * `create_playlist` —— `music.musicasset.PlaylistBaseWrite / AddPlaylist`；
//! * `delete_playlist` —— 同模块 / `DelPlaylist`（删除不存在的歌单时，上游回的
//!   `dirid` 是 0）；
//! * `add_playlist_songs` —— `music.musicasset.PlaylistDetailWrite / AddSonglist`；
//! * `remove_playlist_songs` —— 同模块 / `DelSonglist`；
//! * `fav_album` / `unfav_album` —— `music.musicasset.AlbumFavWrite / FavAlbum`·
//!   `CancelFavAlbum`。
//!
//! # 平台档案
//!
//! 参考的 `@cgi_endpoint` 八个都没标 `platform`，按本层约定默认 Web；
//! `dispatch` 收到调用方传入的 platform 时原样尊重。
//!
//! # 参数形状（照参考逐字拼）
//!
//! 歌单歌曲的读写请求是 [`songlist_oper_param`]：`dirId` / `tid` / `bFmtUtf8` /
//! `v_songInfo`（每项 `{songId, songType}`）。`bFmtUtf8` 有**两种形状**：参考的
//! `add_songs` 标了 `preserve_bool=True`（原样发 JSON `true`），`del_songs` 没标
//!（executor 的 `bool_to_int` 会把它转成 `1`）——这里也照这个差别发。
//!
//! # 空值判据
//!
//! 八个端点参考都标了 `require_login=True`，未登录时在发出任何请求之前报错。
//! 歌单歌曲列表**空是允许的**：参考对空 `song_info` 不报错，照发
//! `v_songInfo: []`（上游对它是无操作），专辑 ID 列表同理；但协议调用方**整条键
//! 都没给**时按参数错误处理——那多半是键名写错，静默发一份空请求更难查。
//!
//! # 80092
//!
//! 上游用业务码 80092 表示「这首歌已经在歌单里了 / 不在歌单里」。参考两个方法的
//! 文档串都写着「也返回 True」，但 `except` 分支的代码实际 `return False`——两者
//! 矛盾；工作单明确要求按**成功**处理，这里照工作单：`retCode` 或 slot code 是
//! 80092 时回 `success: true`（见 [`playlist_write_result`]）。
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
    "create_playlist",
    "delete_playlist",
    "add_playlist_songs",
    "remove_playlist_songs",
    "fav_album",
    "unfav_album",
    "fav_playlist",
    "unfav_playlist",
];

/// 上游「这首歌已经在歌单里 / 不在歌单里」的业务码（参考里两个方法都特判它）。
const ALREADY_IN_PLAYLIST: i64 = 80092;

/// 建 / 删歌单的模块。
const PLAYLIST_BASE_WRITE: &str = "music.musicasset.PlaylistBaseWrite";
/// 歌单歌曲增删的模块。
const PLAYLIST_DETAIL_WRITE: &str = "music.musicasset.PlaylistDetailWrite";
/// 收藏专辑的模块。
const ALBUM_FAV_WRITE: &str = "music.musicasset.AlbumFavWrite";
/// 收藏他人公开歌单的模块（user.fav_songlist / unfav_songlist）。
const PLAYLIST_FAV_WRITE: &str = "music.musicasset.PlaylistFavWrite";

// MARK: - 模型

/// 参考 `song_info` 的 `(song_id, song_type)` 对.
///
/// 宿主侧用具名结构而不是 `Vec<(i64, i64)>`：BoltFFI 渲染不了元组向量
/// （见 docs/ffi.md「生成器的限制」）。JSON 上就是 `{songId, songType}`。
///
/// 两个字段都是标量，`#[data]` 把它当 blittable，因此必须 `Copy`
/// （与 `models.rs` 的 `RateLimitUsage` 同款）。
#[data]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongInfoPair {
    /// 歌曲数字 ID.
    pub song_id: i64,
    /// 歌曲类型（1=普通歌曲，2=长音频，6=视频/直播……）.
    pub song_type: i64,
}

/// 参考 `CreateDeleteSonglistResp`：建 / 删歌单的响应体.
///
/// 参考模型里 `retCode` 是必填，这里给 `Option`：上游缺了就是 `null`，不假装
/// 有值（`id`/`dirid`/`name` 在参考里带 jsonpath，指向 `result` 块）。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeleteSonglistResp {
    /// 返回码（为 0 表示成功）.
    pub ret_code: Option<i64>,
    /// 创建成功的歌单 ID（上游 `$.result.tid`）.
    pub id: Option<i64>,
    /// 创建成功的歌单目录 ID（上游 `$.result.dirId`；删除不存在的歌单时是 0）.
    pub dirid: Option<i64>,
    /// 创建成功的歌单名称（上游 `$.result.dirName`）.
    pub name: Option<String>,
}

/// 参考 `AlbumFavWriteResponse`：收藏 / 取消收藏专辑的写操作响应.
///
/// 参考模型的 `result` 与 `failed_album_id` 都有缺省（0 / `[]`），`success` 是
/// 它的计算属性（`result == 0 and not failed_album_id`），这里一并给出来，
/// 宿主不必自己算。
#[data]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlbumFavWriteResponse {
    /// 操作结果码，0 表示成功.
    pub result: Option<i64>,
    /// 操作失败的专辑 ID 列表（上游 `v_failedAlbumId`），全部成功时为空.
    pub failed_album_id: Option<Vec<i64>>,
    /// 是否操作成功（`result` 为 0 且无失败项）.
    pub success: Option<bool>,
}

// MARK: - 回值搬运（上游形状 → 参考模型形状）

/// 建 / 删歌单回值（参考 `CreateDeleteSonglistResp`）。
///
/// `id`/`dirid`/`name` 照参考的 jsonpath 取自 `result` 块，`retCode` 在顶层；
/// `result` 缺失时退回顶层取（上游个别版本会平铺）。
fn create_delete_payload(data: &Value) -> Value {
    let result = first_object(data, &["result", "Result"]).unwrap_or(data);
    json!({
        "retCode": first_int(data, &["retCode", "ret_code"]),
        "id": first_int(result, &["tid", "id"]),
        "dirid": first_int(result, &["dirId", "dirid", "dir_id"]),
        "name": optional_text(result, &["dirName", "dirname", "name"]),
    })
}

/// 收藏歌单的布尔回值，与参考 user.fav_songlist / unfav_songlist 一致。
#[data]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct PlaylistFavWriteResponse {
    pub success: bool,
}

/// 写操作必须明确返回整数结果码；空对象或损坏字段不能被当成成功。
fn write_result_code(data: &Value, keys: &[&str]) -> Result<i64, UpstreamError> {
    keys.iter()
        .find_map(|key| data.get(*key))
        .and_then(int_value)
        .ok_or_else(|| UpstreamError::Upstream("写操作响应缺少有效的结果码".into()))
}

/// 失败列表允许缺省；存在时必须是完整的整数数组，不能丢掉损坏项。
fn failed_ids(data: &Value, keys: &[&str]) -> Result<Vec<i64>, UpstreamError> {
    match keys.iter().find_map(|key| data.get(*key)) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                int_value(item).ok_or_else(|| {
                    UpstreamError::Upstream("写操作响应的失败 ID 列表含非整数".into())
                })
            })
            .collect(),
        Some(_) => Err(UpstreamError::Upstream(
            "写操作响应的失败 ID 列表不是数组".into(),
        )),
    }
}

/// 收藏 / 取消收藏专辑回值。只有明确的 0 结果码且没有失败项才成功。
fn album_fav_payload(data: &Value) -> Result<Value, UpstreamError> {
    let failed = failed_ids(data, &["v_failedAlbumId", "v_failed_album_id"])?;
    let result = write_result_code(data, &["result", "Result"])?;
    Ok(json!({
        "result": result,
        "failedAlbumId": failed,
        "success": result == 0 && failed.is_empty(),
    }))
}

/// 参考判据是当前请求的 ID 不在失败列表中，而非整个列表必须为空。
fn playlist_fav_payload(data: &Value, playlist_id: i64) -> Result<Value, UpstreamError> {
    let result = write_result_code(data, &["result"])?;
    let failed = failed_ids(data, &["v_failedPlaylistId"])?;
    Ok(json!({ "success": result == 0 && !failed.contains(&playlist_id) }))
}

/// 歌单歌曲写操作的结果（参考 `add_songs` / `del_songs` 的 `bool` 回值）。
///
/// 参考的判据是 `data.get("retCode") == 0`；工作单要求 80092 也算成功
/// （「已经在里面了」/「不在里面」不是故障），所以这里多认一个码。
fn playlist_write_succeeded(data: &Value) -> bool {
    matches!(
        first_int(data, &["retCode", "ret_code"]),
        Some(0) | Some(ALREADY_IN_PLAYLIST)
    )
}

/// 把写操作的上抛收成 `{"success": bool}`。
///
/// slot code 是 80092 时上游不给 data，`call_with` 会把它拼成错误
/// （`上游返回错误（80092）：…`）——这条路同样按成功收下。其余错误照常上抛。
fn playlist_write_result(outcome: Result<Value, UpstreamError>) -> Result<Value, UpstreamError> {
    match outcome {
        Ok(data) => Ok(json!({ "success": playlist_write_succeeded(&data) })),
        Err(error) if upstream_error_code(&error) == Some(ALREADY_IN_PLAYLIST) => {
            Ok(json!({ "success": true }))
        }
        Err(error) => Err(error),
    }
}

/// 从 `call_with` 的错误信息里取出上游业务码。
///
/// 错误信息由 `upstream.rs` 拼成 `上游返回错误（{code}）：{msg}`，这里只认括号里
/// 那一段，免得 msg 里出现的同名数字被误伤（`song_related.rs` 的乐谱 10007
/// 用的是同一套办法）。
fn upstream_error_code(error: &UpstreamError) -> Option<i64> {
    let UpstreamError::Upstream(message) = error else {
        return None;
    };
    let (_, rest) = message.split_once('（')?;
    let (code, _) = rest.split_once('）')?;
    code.trim().parse().ok()
}

// MARK: - 请求参数（照参考逐字拼）

/// 建歌单参数（参考 `create`）。
fn create_params(dir_name: &str) -> Value {
    json!({ "dirName": dir_name })
}

/// 删歌单参数（参考 `delete`）。
fn delete_params(dirid: i64) -> Value {
    json!({ "dirId": dirid })
}

/// 参考 `_build_songlist_oper_param`：歌单写操作的最小参数.
///
/// `bFmtUtf8` 的两种形状见文件头：`add_songs` 的 `preserve_bool=True` 发 JSON
/// `true`，`del_songs` 发 `1`。逐字照参考的差别，不统一。
fn songlist_oper_param(dirid: i64, songs: &[(i64, i64)], tid: i64, preserve_bool: bool) -> Value {
    let v_song_info: Vec<Value> = songs
        .iter()
        .map(|(song_id, song_type)| json!({ "songId": song_id, "songType": song_type }))
        .collect();
    json!({
        "dirId": dirid,
        "tid": tid,
        "bFmtUtf8": if preserve_bool { json!(true) } else { json!(1) },
        "v_songInfo": v_song_info,
    })
}

/// 收藏 / 取消收藏专辑参数（参考 `fav_album` / `del_fav_album`）：
/// `album_id: int | list[int]` 一律归一成列表。
fn album_ids_param(ids: Vec<i64>) -> Value {
    json!({ "v_albumId": ids })
}

// MARK: - 协议参数解析

/// `tid`：参考缺省 0。
fn tid_of(params: &Value) -> i64 {
    first_int(params, &["tid"]).unwrap_or(0)
}

/// 歌单目录 ID（参考里 `dirid` 是必填的 `int`）。
fn dirid_of(params: &Value) -> Result<i64, UpstreamError> {
    first_int(params, &["dirId", "dirid", "dir_id"])
        .ok_or_else(|| UpstreamError::Upstream("缺少歌单目录 ID（dirId）".into()))
}

/// 歌曲信息列表（参考的 `song_info: list[tuple[int, int]]`）。
///
/// 协议层接受两种写法：对象 `{"songId": 1, "songType": 2}`（与回值同形）或二元
/// 数组 `[1, 2]`（参考的元组）。`songType` 缺省 0；整条 `songInfo` 键不给时报错，
/// 给了空列表则照发 `[]`（参考对空列表不报错）。
fn song_pairs(params: &Value) -> Result<Vec<(i64, i64)>, UpstreamError> {
    let items = first_array(params, &["songInfo", "song_info", "v_songInfo", "songs"])
        .ok_or_else(|| UpstreamError::Upstream("缺少 songInfo（歌曲信息列表）".into()))?;
    let mut pairs = Vec::with_capacity(items.len());
    for item in items {
        let pair = match item {
            Value::Array(values) => {
                let song_id = values.first().and_then(int_value).ok_or_else(|| {
                    UpstreamError::Upstream(format!("songInfo 里有非整数：{item}"))
                })?;
                (song_id, values.get(1).and_then(int_value).unwrap_or(0))
            }
            Value::Object(_) => {
                let song_id =
                    first_int(item, &["songId", "song_id", "songid", "id"]).ok_or_else(|| {
                        UpstreamError::Upstream(format!("songInfo 项缺少 songId：{item}"))
                    })?;
                let song_type = first_int(item, &["songType", "song_type", "type"]).unwrap_or(0);
                (song_id, song_type)
            }
            _ => {
                return Err(UpstreamError::Upstream(format!(
                    "songInfo 里有认不出的项：{item}"
                )))
            }
        };
        pairs.push(pair);
    }
    Ok(pairs)
}

/// 专辑 ID 列表（参考的 `album_id: int | list[int]` 归一成列表后就是 `v_albumId`）。
///
/// 参考收单个 int 也收列表；协议层两种都给——单个 `albumId: 7` 归一成 `[7]`，
/// 列表键（`albumIds` / `v_albumId`）按列表读。空列表允许（参考照发 `[]`），
/// 非整数是调用方的错误。
fn album_ids_of(params: &Value) -> Result<Vec<i64>, UpstreamError> {
    const KEYS: &[&str] = &["albumIds", "album_ids", "v_albumId", "albumId"];
    if let Some(items) = first_array(params, KEYS) {
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            let number = int_value(item)
                .ok_or_else(|| UpstreamError::Upstream(format!("albumIds 里有非整数：{item}")))?;
            ids.push(number);
        }
        return Ok(ids);
    }
    match KEYS
        .iter()
        .find_map(|key| params.get(*key).filter(|value| !value.is_null()))
    {
        Some(value) => {
            let single = int_value(value)
                .ok_or_else(|| UpstreamError::Upstream(format!("albumIds 里有非整数：{value}")))?;
            Ok(vec![single])
        }
        None => Err(UpstreamError::Upstream(
            "缺少专辑 ID 列表（albumIds）".into(),
        )),
    }
}

/// 数字或数字字符串 → 整数。
fn int_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 保留空串的字符串字段读取（`first_text` 会把空串当缺省）。
///
/// 参考模型里 `name` 的缺省是 `""`，删除不存在的歌单时 `dirName` 就是空的——
/// 读成 `None` 会与"上游没给"混在一起。
fn optional_text(raw: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| raw.get(*key))
        .and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
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
        "create_playlist" => add_playlist(upstream, credential, platform, params),
        "delete_playlist" => remove_playlist(upstream, credential, platform, params),
        "add_playlist_songs" => add_songs(upstream, credential, platform, params),
        "remove_playlist_songs" => remove_songs(upstream, credential, platform, params),
        "fav_album" => favorite_album(upstream, credential, platform, params),
        "unfav_album" => cancel_favorite_album(upstream, credential, platform, params),
        "fav_playlist" => write_playlist_favorite(upstream, credential, platform, params, false),
        "unfav_playlist" => write_playlist_favorite(upstream, credential, platform, params, true),
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

/// 建歌单（`music.musicasset.PlaylistBaseWrite / AddPlaylist`），需要登录。
///
/// 重名歌单并不会创建失败——上游会自动加时间戳（参考的 Note）。
fn add_playlist(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let dir_name = first_text(params, &["dirName", "dirname", "dir_name", "name"])
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| UpstreamError::Upstream("缺少歌单名称（dirName）".into()))?;
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: PLAYLIST_BASE_WRITE,
            method: "AddPlaylist",
            param: create_params(&dir_name),
        },
    )?;
    Ok(create_delete_payload(&data))
}

/// 删歌单（`music.musicasset.PlaylistBaseWrite / DelPlaylist`），需要登录。
///
/// 删除不存在的歌单不算故障：上游回 `dirid: 0`（参考的 Note）。
fn remove_playlist(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let dirid = dirid_of(params)?;
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: PLAYLIST_BASE_WRITE,
            method: "DelPlaylist",
            param: delete_params(dirid),
        },
    )?;
    Ok(create_delete_payload(&data))
}

/// 添加歌曲到歌单（`music.musicasset.PlaylistDetailWrite / AddSonglist`）。
///
/// `preserve_bool=True`（参考）——`bFmtUtf8` 发 JSON `true`。
/// 歌曲已在歌单里（80092）按成功处理，见 [`playlist_write_result`]。
fn add_songs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let dirid = dirid_of(params)?;
    let songs = song_pairs(params)?;
    playlist_write_result(upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: PLAYLIST_DETAIL_WRITE,
            method: "AddSonglist",
            param: songlist_oper_param(dirid, &songs, tid_of(params), true),
        },
    ))
}

/// 删除歌单中的歌曲（`music.musicasset.PlaylistDetailWrite / DelSonglist`）。
///
/// 参考的 `del_songs` 没标 `preserve_bool`，`bFmtUtf8` 被 executor 转成 `1`。
/// 歌曲不在歌单里（80092）按成功处理，见 [`playlist_write_result`]。
fn remove_songs(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let dirid = dirid_of(params)?;
    let songs = song_pairs(params)?;
    playlist_write_result(upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: PLAYLIST_DETAIL_WRITE,
            method: "DelSonglist",
            param: songlist_oper_param(dirid, &songs, tid_of(params), false),
        },
    ))
}

/// 收藏专辑（`music.musicasset.AlbumFavWrite / FavAlbum`），需要登录。
fn favorite_album(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let ids = album_ids_of(params)?;
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: ALBUM_FAV_WRITE,
            method: "FavAlbum",
            param: album_ids_param(ids),
        },
    )?;
    album_fav_payload(&data)
}

/// 取消收藏专辑（`music.musicasset.AlbumFavWrite / CancelFavAlbum`），需要登录。
fn cancel_favorite_album(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let ids = album_ids_of(params)?;
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: ALBUM_FAV_WRITE,
            method: "CancelFavAlbum",
            param: album_ids_param(ids),
        },
    )?;
    album_fav_payload(&data)
}

/// disstid/pid 是公开歌单 ID，不能以自建歌单的目录 ID 替代。
fn playlist_id_of(params: &Value) -> Result<i64, UpstreamError> {
    first_int(
        params,
        &[
            "playlistId",
            "playlist_id",
            "songlistId",
            "songlist_id",
            "disstid",
            "pid",
        ],
    )
    .ok_or_else(|| UpstreamError::Upstream("缺少歌单 ID（playlistId/disstid）".into()))
}

fn playlist_fav_params(encrypted_uin: &str, playlist_id: i64) -> Value {
    json!({ "uin": encrypted_uin, "v_playlistId": [playlist_id] })
}

/// 收藏他人的公开歌单，与 user.py 的两条写请求逐字段一致。
fn write_playlist_favorite(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    params: &Value,
    cancel: bool,
) -> Result<Value, UpstreamError> {
    require_login(credential)?;
    let playlist_id = playlist_id_of(params)?;
    let encrypted_uin = upstream.encrypted_uin(credential)?;
    let data = upstream.call_with(
        credential,
        Class::Write,
        platform,
        Call {
            module: PLAYLIST_FAV_WRITE,
            method: if cancel {
                "CancelFavPlaylist"
            } else {
                "FavPlaylist"
            },
            param: playlist_fav_params(&encrypted_uin, playlist_id),
        },
    )?;
    playlist_fav_payload(&data, playlist_id)
}

// MARK: - 宿主包装

/// 创建歌单。重名不会失败——上游自动加时间戳。
#[export]
pub fn create_playlist(dirname: String) -> Result<CreateDeleteSonglistResp, crate::HelperError> {
    crate::port::call("create_playlist", json!({ "dirName": dirname }))
}

/// 删除歌单；删除不存在的歌单时回值的 `dirid` 是 0。
#[export]
pub fn delete_playlist(dirid: i64) -> Result<CreateDeleteSonglistResp, crate::HelperError> {
    crate::port::call("delete_playlist", json!({ "dirId": dirid }))
}

/// 添加歌曲到歌单；`tid` 缺省 0。
///
/// 返回是否操作成功：歌曲已经在歌单里（80092）也算成功。
#[export]
pub fn add_playlist_songs(
    dirid: i64,
    song_info: Vec<SongInfoPair>,
    tid: Option<i64>,
) -> Result<bool, crate::HelperError> {
    let mut params = json!({ "dirId": dirid, "songInfo": song_info });
    if let Some(tid) = tid {
        params["tid"] = json!(tid);
    }
    let value: Value = crate::port::call("add_playlist_songs", params)?;
    Ok(value
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// 删除歌单中的歌曲；`tid` 缺省 0。
///
/// 返回是否操作成功：歌曲不在歌单里（80092）也算成功。
#[export]
pub fn remove_playlist_songs(
    dirid: i64,
    song_info: Vec<SongInfoPair>,
    tid: Option<i64>,
) -> Result<bool, crate::HelperError> {
    let mut params = json!({ "dirId": dirid, "songInfo": song_info });
    if let Some(tid) = tid {
        params["tid"] = json!(tid);
    }
    let value: Value = crate::port::call("remove_playlist_songs", params)?;
    Ok(value
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// 收藏专辑到当前登录用户；`album_ids` 是要收藏的专辑 ID 列表。
///
/// 回值的 `success` 已经算好：`result == 0` 且没有失败项。
#[export]
pub fn fav_album(album_ids: Vec<i64>) -> Result<AlbumFavWriteResponse, crate::HelperError> {
    crate::port::call("fav_album", json!({ "albumIds": album_ids }))
}

/// 取消收藏专辑；`album_ids` 是要取消的专辑 ID 列表。
#[export]
pub fn unfav_album(album_ids: Vec<i64>) -> Result<AlbumFavWriteResponse, crate::HelperError> {
    crate::port::call("unfav_album", json!({ "albumIds": album_ids }))
}

/// 收藏公开歌单（playlist_id 是 disstid/pid）。已收藏时也返回成功。
#[export]
pub fn fav_playlist(playlist_id: i64) -> Result<bool, crate::HelperError> {
    let response: PlaylistFavWriteResponse =
        crate::port::call("fav_playlist", json!({ "playlistId": playlist_id }))?;
    Ok(response.success)
}

/// 取消收藏公开歌单（playlist_id 是 disstid/pid）。未收藏时也返回成功。
#[export]
pub fn unfav_playlist(playlist_id: i64) -> Result<bool, crate::HelperError> {
    let response: PlaylistFavWriteResponse =
        crate::port::call("unfav_playlist", json!({ "playlistId": playlist_id }))?;
    Ok(response.success)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一组合法的歌曲对，用作参数拼装的输入。
    fn pairs() -> Vec<(i64, i64)> {
        vec![(2314161, 1), (102065756, 113)]
    }

    #[test]
    fn the_module_claims_its_own_methods_and_nothing_else() {
        assert_eq!(METHODS.len(), 8);
        let upstream = Upstream::new();
        assert!(dispatch(
            &upstream,
            &Credential::default(),
            Platform::Web,
            "set_liked",
            &json!({})
        )
        .is_none());
    }

    #[test]
    fn the_create_and_delete_requests_carry_the_reference_fields() {
        assert_eq!(create_params("夜曲"), json!({ "dirName": "夜曲" }));
        assert_eq!(delete_params(201), json!({ "dirId": 201 }));
    }

    #[test]
    fn a_songlist_write_carries_the_reference_param_shape() {
        let param = songlist_oper_param(201, &pairs(), 0, true);
        assert_eq!(param["dirId"], 201);
        assert_eq!(param["tid"], 0);
        assert_eq!(param["bFmtUtf8"], true, "add_songs 标了 preserve_bool");
        assert_eq!(
            param["v_songInfo"][0],
            json!({ "songId": 2314161, "songType": 1 })
        );
        assert_eq!(param["v_songInfo"][1]["songType"], 113);

        let param = songlist_oper_param(201, &pairs(), 7, false);
        assert_eq!(param["tid"], 7);
        assert_eq!(param["bFmtUtf8"], 1, "del_songs 没标，参考发 1");
        assert_eq!(
            param["v_songInfo"],
            json!([{ "songId": 2314161, "songType": 1 }, { "songId": 102065756, "songType": 113 }])
        );
    }

    #[test]
    fn a_song_info_list_takes_objects_or_pairs() {
        let songs = song_pairs(&json!({
            "songInfo": [
                { "songId": 1, "songType": 2 },
                ["2", "3"],
                { "songId": "4" },
                { "song_id": 5, "song_type": 6 }
            ]
        }))
        .expect("两种写法都收");
        assert_eq!(songs, vec![(1, 2), (2, 3), (4, 0), (5, 6)]);

        assert_eq!(
            song_pairs(&json!({ "songInfo": [] })).unwrap(),
            vec![],
            "空列表是允许的：参考照发 v_songInfo: []"
        );
        let error = song_pairs(&json!({})).expect_err("整条键不给要报错");
        assert!(error.to_string().contains("songInfo"), "{error}");
        assert!(song_pairs(&json!({ "songInfo": [{ "songType": 1 }] })).is_err());
    }

    /// 宿主把 `SongInfoPair` 交给包装时，线上就是参考的 `{songId, songType}`。
    #[test]
    fn a_song_info_pair_writes_the_reference_wire_shape() {
        let pairs = vec![
            SongInfoPair {
                song_id: 2314161,
                song_type: 1,
            },
            SongInfoPair {
                song_id: 102065756,
                song_type: 113,
            },
        ];
        assert_eq!(
            serde_json::to_value(&pairs).unwrap(),
            json!([{ "songId": 2314161, "songType": 1 }, { "songId": 102065756, "songType": 113 }])
        );
        // 宿主若走协议（JSON 参数）而不是类型化包装，两条路拼出同一份请求体。
        assert_eq!(
            song_pairs(&json!({ "songInfo": serde_json::to_value(&pairs).unwrap() })).unwrap(),
            pairs
                .iter()
                .map(|pair| (pair.song_id, pair.song_type))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_album_ids_are_normalised_to_v_album_id() {
        assert_eq!(
            album_ids_of(&json!({ "albumIds": [1, "2"] })).unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            album_ids_of(&json!({ "v_albumId": [7] })).unwrap(),
            vec![7],
            "协议中间的键名也认"
        );
        assert_eq!(
            album_ids_of(&json!({ "albumId": 7 })).unwrap(),
            vec![7],
            "参考收单个 int：归一成 [7]"
        );
        assert_eq!(
            album_ids_of(&json!({ "albumId": "7" })).unwrap(),
            vec![7],
            "数字字符串同理"
        );
        assert!(album_ids_of(&json!({})).is_err());
        assert!(album_ids_of(&json!({ "albumIds": ["x"] })).is_err());
        assert!(album_ids_of(&json!({ "albumId": "x" })).is_err());
        assert_eq!(album_ids_param(vec![1, 2]), json!({ "v_albumId": [1, 2] }));
    }

    #[test]
    fn a_created_playlist_reads_id_dirid_and_name_from_result() {
        let payload = create_delete_payload(&json!({
            "retCode": 0,
            "result": { "tid": 9578424174i64, "dirId": 201, "dirName": "夜曲" }
        }));
        let response: CreateDeleteSonglistResp =
            serde_json::from_value(payload.clone()).expect("解析建歌单回值");
        assert_eq!(response.ret_code, Some(0));
        assert_eq!(response.id, Some(9578424174));
        assert_eq!(response.dirid, Some(201));
        assert_eq!(response.name.as_deref(), Some("夜曲"));
        assert_eq!(payload["retCode"], 0, "宿主按参考字段名 retCode 读");

        // 删除不存在的歌单：上游回的 dirid 是 0，仍是一次正常应答。
        let response: CreateDeleteSonglistResp =
            serde_json::from_value(create_delete_payload(&json!({
                "retCode": 0,
                "result": { "tid": 0, "dirId": 0, "dirName": "" }
            })))
            .expect("解析删歌单回值");
        assert_eq!(response.dirid, Some(0));
        assert_eq!(response.name.as_deref(), Some(""), "空名字保留为空串");
    }

    #[test]
    fn a_favourite_album_write_reports_success_only_when_nothing_failed() {
        let response: AlbumFavWriteResponse = serde_json::from_value(
            album_fav_payload(&json!({
                "result": 0,
                "v_failedAlbumId": []
            }))
            .unwrap(),
        )
        .expect("解析收藏专辑回值");
        assert_eq!(response.result, Some(0));
        assert_eq!(response.success, Some(true));
        assert!(response.failed_album_id.expect("字段仍在").is_empty());

        let response: AlbumFavWriteResponse = serde_json::from_value(
            album_fav_payload(&json!({
                "result": 0,
                "v_failedAlbumId": [42]
            }))
            .unwrap(),
        )
        .expect("解析部分失败的回值");
        assert_eq!(response.success, Some(false));
        assert_eq!(response.failed_album_id, Some(vec![42]));

        assert!(album_fav_payload(&json!({})).is_err(), "空响应不能默认成功");
        assert!(album_fav_payload(&json!({"result": "bad"})).is_err());
        assert!(album_fav_payload(&json!({"result": 0, "v_failedAlbumId": ["bad"]})).is_err());
        assert!(album_fav_payload(&json!({"result": 0, "v_failedAlbumId": {}})).is_err());
        assert_eq!(
            album_fav_payload(&json!({"result": 0})).unwrap()["success"],
            true
        );
    }

    #[test]
    fn playlist_favorites_match_reference_parameters_and_target_validation() {
        assert_eq!(
            playlist_fav_params("encrypted-user", 123),
            json!({"uin": "encrypted-user", "v_playlistId": [123]})
        );
        for key in [
            "playlistId",
            "playlist_id",
            "songlistId",
            "songlist_id",
            "disstid",
            "pid",
        ] {
            assert_eq!(playlist_id_of(&json!({key: 123})).unwrap(), 123);
        }
        assert!(playlist_id_of(&json!({"dirId": 201})).is_err());
        for (data, success) in [
            (json!({"result": 0}), true),
            (json!({"result": 0, "v_failedPlaylistId": null}), true),
            (json!({"result": 0, "v_failedPlaylistId": [999]}), true),
            (json!({"result": 0, "v_failedPlaylistId": [123]}), false),
            (json!({"result": 1, "v_failedPlaylistId": []}), false),
        ] {
            let response: PlaylistFavWriteResponse =
                serde_json::from_value(playlist_fav_payload(&data, 123).unwrap()).unwrap();
            assert_eq!(response.success, success);
        }
        for data in [
            json!({}),
            json!({"result": null}),
            json!({"result": 0, "v_failedPlaylistId": ["broken"]}),
            json!({"result": 0, "v_failedPlaylistId": "broken"}),
        ] {
            assert!(playlist_fav_payload(&data, 123).is_err());
        }
    }

    #[test]
    fn the_80092_code_counts_as_success() {
        // retCode 是 80092（slot code 为 0）——参考代码里 `== 0` 会判 False，
        // 工作单要求按成功。
        assert!(playlist_write_succeeded(&json!({ "retCode": 0 })));
        assert!(playlist_write_succeeded(&json!({ "retCode": 80092 })));
        assert!(!playlist_write_succeeded(&json!({ "retCode": 1 })));
        assert!(!playlist_write_succeeded(&json!({})), "缺 retCode 不算成功");

        // slot code 是 80092 时 call_with 会报错（不给 data），这条路也按成功收下。
        let error = UpstreamError::Upstream("上游返回错误（80092）：歌曲已在歌单".into());
        let value = playlist_write_result(Err(error)).expect("80092 是成功");
        assert_eq!(value["success"], true);

        // 其他错误照常上抛。
        let error = UpstreamError::Upstream("上游返回错误（1000）：登录已过期".into());
        assert!(playlist_write_result(Err(error)).is_err());
        // 只认括号里的业务码，msg 里的同名数字不能误伤。
        let error = UpstreamError::Upstream("上游返回错误（1）：retCode=80092".into());
        assert!(playlist_write_result(Err(error)).is_err());
    }

    /// 八个写操作没有凭据时，在发出任何请求之前就应当被拒。
    #[test]
    fn writes_refuse_without_a_login_before_any_request() {
        let upstream = Upstream::new();
        for method in METHODS {
            let error = dispatch(
                &upstream,
                &Credential::default(),
                Platform::Web,
                method,
                &json!({}),
            )
            .expect("是本层的方法")
            .expect_err("未登录要报错");
            assert!(error.to_string().contains("登录"), "{method}: {error}");
        }
    }
}

//! 集合写入：自建歌单的增删改，以及收藏专辑的加/取消。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/songlist.py`
//! （`create` / `delete` / `add_songs` / `del_songs`）与 `modules/album.py`
//! （`fav_album` / `del_fav_album`）。
//! 本文件由移植工作流的一个子代理独占；不要在此文件之外修改任何内容。
use crate::credential::Credential;
use crate::upstream::{Platform, Upstream, UpstreamError};
use serde_json::Value;

/// 本模块对协议层暴露的方法名。
pub const METHODS: &[&str] = &[];

/// 协议分发：只认领 `METHODS` 里的名字，其余返回 `None`。
pub fn dispatch(
    _upstream: &Upstream,
    _credential: &Credential,
    _platform: Platform,
    _method: &str,
    _params: &Value,
) -> Option<Result<Value, UpstreamError>> {
    None
}

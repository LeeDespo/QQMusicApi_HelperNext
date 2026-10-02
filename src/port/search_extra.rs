//! 搜索扩展：热词、联想补全、快速搜索、综合搜索，以及既有四类之外的搜索类型。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/search.py`
//! （`get_hotkey` / `complete` / `quick_search` / `general_search` /
//! `search_by_type` 的全部类型）。既有的 `search_songs` 等四个方法不动。
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

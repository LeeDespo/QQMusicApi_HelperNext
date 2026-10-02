//! 推荐扩展：首页信息流、雷达推荐、推荐歌单。
//!
//! 参考：`dist/reference/QQMusicApi/qqmusic_api/modules/recommend.py`
//! （`get_home_feed` / `get_radar_recommend` / `get_recommend_songlist`；
//! `get_guess_recommend` 已由既有 `fetch_recommend_feed` 覆盖，
//! `get_recommend_newsong` 与既有 `fetch_new_songs` 同一端点，不重复移植）。
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

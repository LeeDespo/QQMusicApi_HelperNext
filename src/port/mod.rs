//! 参考实现（QQMusicApi）的移植层。
//!
//! 这一层与既有的 `methods.rs` / `catalog.rs` 分工明确：既有代码是**已上线契约**，
//! 不被改动；参考库里那些尚未移植的接口落在这里，一个领域一个文件，
//! 每个文件自带协议分发、模型与类型化包装（`#[export]`）。
//!
//! 当前公开契约见 `docs/endpoints.md`，解析与兼容规则见 `docs/parsing.md`。
//!
//! # 新增一个领域
//!
//! 1. `src/port/<domain>.rs`：`pub const METHODS`、`pub fn dispatch`、
//!    `#[data]` 模型、`#[export]` 包装、`#[cfg(test)]` 测试，全部在这一个文件里；
//! 2. 在下面的 `mod` 列表、`all_methods()`、`dispatch()` 三处登记；
//! 3. 不碰 `src/api.rs`、`src/models.rs`、`src/methods.rs` 等既有文件的既有内容。
//!
//! 本层自带两条确定性检查（见文件尾的测试）：每个协议方法都要有**同名**的
//! `#[export]` 包装（宿主与绑定都按这个名字调），且方法名在本层内不重复。

pub mod collection_write;
pub mod comment;
pub mod library_extra;
pub mod login_extra;
pub mod mv;
pub mod recommend_extra;
pub mod search_extra;
pub mod signed;
pub mod singer_extra;
pub mod song_asset;
pub mod song_related;
pub mod user_asset;
pub mod user_relation;

use crate::credential::Credential;
use crate::upstream::{Platform, Upstream, UpstreamError};
use serde_json::Value;

/// 本层全部协议方法名，按领域拼接。
pub fn all_methods() -> Vec<&'static str> {
    let mut list: Vec<&'static str> = Vec::new();
    for module in [
        collection_write::METHODS,
        comment::METHODS,
        library_extra::METHODS,
        login_extra::METHODS,
        mv::METHODS,
        recommend_extra::METHODS,
        search_extra::METHODS,
        singer_extra::METHODS,
        song_asset::METHODS,
        song_related::METHODS,
        user_asset::METHODS,
        user_relation::METHODS,
    ] {
        list.extend_from_slice(module);
    }
    list
}

/// 这个方法是不是本层的。
pub fn is_known(method: &str) -> bool {
    all_methods().contains(&method)
}

/// 按领域依次尝试。每个领域只认领自己 `METHODS` 里的名字。
pub fn dispatch(
    upstream: &Upstream,
    credential: &Credential,
    platform: Platform,
    method: &str,
    params: &Value,
) -> Option<Result<Value, UpstreamError>> {
    macro_rules! try_domain {
        ($module:ident) => {
            if let Some(result) = $module::dispatch(upstream, credential, platform, method, params)
            {
                return Some(result);
            }
        };
    }
    try_domain!(collection_write);
    try_domain!(comment);
    try_domain!(library_extra);
    try_domain!(login_extra);
    try_domain!(mv);
    try_domain!(recommend_extra);
    try_domain!(search_extra);
    try_domain!(singer_extra);
    try_domain!(song_asset);
    try_domain!(song_related);
    try_domain!(user_asset);
    try_domain!(user_relation);
    None
}

/// 走协议层再解析成模型，`#[export]` 包装的统一入口。
///
/// 与 `api.rs` 的同名私有函数走同一条路：凭据来自组件自己的存储，
/// HTTP agent、限流器与熔断器都取全进程共享的那一个。
pub fn call<T: serde::de::DeserializeOwned>(
    method: &str,
    params: Value,
) -> Result<T, crate::HelperError> {
    let upstream = crate::api::shared_upstream();
    let credential = crate::CredentialStore::for_directory(&crate::data_directory()).load();
    let value = crate::methods::dispatch(upstream, credential.as_ref(), method, &params)
        .map_err(crate::HelperError::from)?;
    serde_json::from_value(value).map_err(|error| crate::HelperError::Upstream(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本层的领域文件，一处登记、三处使用（mod / all_methods / dispatch 由人工保证，
    /// 这里检查的是每个方法都有自己的包装与分发位置）。
    const DOMAIN_FILES: &[(&str, &[&str], &str)] = &[
        (
            "library_extra",
            library_extra::METHODS,
            include_str!("library_extra.rs"),
        ),
        (
            "collection_write",
            collection_write::METHODS,
            include_str!("collection_write.rs"),
        ),
        ("comment", comment::METHODS, include_str!("comment.rs")),
        (
            "login_extra",
            login_extra::METHODS,
            include_str!("login_extra.rs"),
        ),
        ("mv", mv::METHODS, include_str!("mv.rs")),
        (
            "recommend_extra",
            recommend_extra::METHODS,
            include_str!("recommend_extra.rs"),
        ),
        (
            "search_extra",
            search_extra::METHODS,
            include_str!("search_extra.rs"),
        ),
        (
            "singer_extra",
            singer_extra::METHODS,
            include_str!("singer_extra.rs"),
        ),
        (
            "song_asset",
            song_asset::METHODS,
            include_str!("song_asset.rs"),
        ),
        (
            "song_related",
            song_related::METHODS,
            include_str!("song_related.rs"),
        ),
        (
            "user_asset",
            user_asset::METHODS,
            include_str!("user_asset.rs"),
        ),
        (
            "user_relation",
            user_relation::METHODS,
            include_str!("user_relation.rs"),
        ),
    ];

    #[test]
    fn every_port_method_has_its_wrapper_and_dispatch_arm() {
        for (domain, methods, source) in DOMAIN_FILES {
            for method in methods.iter() {
                assert!(
                    source.contains(&format!("pub fn {method}")),
                    "{domain}: {method} 缺少同名的 #[export] 包装"
                );
                let quoted = format!("\"{method}\"");
                assert!(
                    source.matches(quoted.as_str()).count() >= 2,
                    "{domain}: {method} 没有同时出现在 METHODS 与 dispatch 两处"
                );
            }
        }
    }

    #[test]
    fn port_method_names_are_unique_across_domains() {
        let mut seen: Vec<&str> = Vec::new();
        for method in all_methods() {
            assert!(!seen.contains(&method), "{method} 在多个领域重复登记");
            seen.push(method);
        }
    }
}

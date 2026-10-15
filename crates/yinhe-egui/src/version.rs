//! 应用版本号。
//!
//! CI 在构建前设置 `YINHE_VERSION`（快照如 `0.1.0-beta.3`，正式版如 `0.1.0`），
//! 编译期注入；本地开发没有该变量，回退到 Cargo.toml 的版本。

/// 版本号裸字符串（不带 `v` 前缀）。
pub(crate) fn version() -> &'static str {
    option_env!("YINHE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

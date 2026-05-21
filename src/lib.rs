//! `binance_tools` 公共库：可复用的业务与基础设施模块放在 `src/` 下按子模块组织。
//!
//! 仓库根目录下的 `examples/` 规划为**若干独立 Rust 子工程**（各含自有 `Cargo.toml`，说明见该目录内 `README.md`），与 Cargo 默认的 `examples/*.rs` 单文件示例不同。

pub mod ai;
pub mod app;
pub mod binance;
pub mod db;
pub mod square;

/// 默认应用名称。
pub fn greeting() -> &'static str {
    app::APP_NAME
}

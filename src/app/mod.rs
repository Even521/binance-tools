//! 应用级常量与跨入口共享状态。

/// 展示在 CLI 与桌面示例中的应用名称。
pub const APP_NAME: &str = "binance_tools";

/// 当前项目框架中的主要能力区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    BinanceSpot,
    DesktopUi,
}

/// 返回当前已接入的能力列表。
pub fn capabilities() -> &'static [Capability] {
    &[Capability::BinanceSpot, Capability::DesktopUi]
}

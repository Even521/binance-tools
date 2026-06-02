//! Binance 连接配置与 SDK 集成入口。
//!
//! 真实 REST/WebSocket 调用优先在本模块下继续拆分，例如 `market`、`account`、
//! `trade`。API key、secret 等敏感信息不应写入仓库。

/// 对外暴露官方 Binance SDK，便于业务模块在统一入口下引用。
pub use binance_sdk as sdk;

pub mod alpha;
pub mod market;
pub mod spot;
pub mod vision;

/// Binance 运行环境。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinanceEnvironment {
    Production,
    Testnet,
}

impl BinanceEnvironment {
    pub const fn rest_base_url(self) -> &'static str {
        match self {
            Self::Production => "https://api.binance.com",
            Self::Testnet => "https://testnet.binance.vision",
        }
    }

    pub const fn websocket_base_url(self) -> &'static str {
        match self {
            Self::Production => "wss://stream.binance.com:9443",
            Self::Testnet => "wss://stream.testnet.binance.vision",
        }
    }
}

impl core::fmt::Display for BinanceEnvironment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Production => f.write_str("production"),
            Self::Testnet => f.write_str("testnet"),
        }
    }
}

/// Binance SDK 与本项目业务模块共享的基础配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinanceSettings {
    environment: BinanceEnvironment,
}

impl BinanceSettings {
    pub const fn production() -> Self {
        Self {
            environment: BinanceEnvironment::Production,
        }
    }

    pub const fn testnet() -> Self {
        Self {
            environment: BinanceEnvironment::Testnet,
        }
    }

    pub const fn environment(&self) -> BinanceEnvironment {
        self.environment
    }

    pub const fn rest_base_url(&self) -> &'static str {
        self.environment.rest_base_url()
    }

    pub const fn websocket_base_url(&self) -> &'static str {
        self.environment.websocket_base_url()
    }
}

impl Default for BinanceSettings {
    fn default() -> Self {
        Self::testnet()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn testnet_uses_binance_spot_testnet_endpoint() {
        let settings = BinanceSettings::testnet();

        assert_eq!(settings.environment(), BinanceEnvironment::Testnet);
        assert_eq!(settings.rest_base_url(), "https://testnet.binance.vision");
    }
}

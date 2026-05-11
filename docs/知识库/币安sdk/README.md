# 币安官方 Rust SDK（连接器）

本目录用于整理与 **Binance Rust Connector** 相关的说明与链接，便于在本仓库内快速跳转。

## 仓库与 crate

| 资源 | 链接 |
| --- | --- |
| GitHub 仓库（官方） | [binance/binance-connector-rust](https://github.com/binance/binance-connector-rust) |
| crates.io / 文档 | [docs.rs/binance_sdk](https://docs.rs/binance-sdk) |
| 币安开放平台（接口权威说明） | [Binance API Documentation](https://developers.binance.com/docs/binance-spot-api-docs/README) |

说明：仓库 README 写明当前为 **OpenAPI 自动生成的模块化 SDK**，统一发布在 **`binance-sdk`** crate 中，通过 **feature** 按需启用各业务线连接器（例如现货 `spot`）。

## 环境与依赖要点

- **Rust**：README 要求 **1.86.0 及以上**（以仓库当前说明为准）。
- **TLS**：默认 **OpenSSL**；若需纯 Rust 栈，可 `default-features = false` 并启用 `rustls-tls`（仓库注明私钥签名等能力可能仍依赖 `openssl-tls`，请对照官方 README）。

## 安装示例（摘自官方思路）

仅启用现货模块时，在 `Cargo.toml` 中大致形如：

```toml
[dependencies]
binance-sdk = { version = "48.0.1", features = ["spot"] }
```

版本号请始终以 [crates.io 上的 `binance-sdk`](https://crates.io/crates/binance-sdk) 为准。需要多模块时继续追加 feature，或使用官方文档中的 `features = ["all"]` 等方式。

## 可用模块（feature 名）

以下为仓库 README 中列举的连接器 feature（按需启用；具体以仓库最新列表为准）：

`algo`、`alpha`、`c2c`、`convert`、`copy_trading`、`crypto_loan`、`derivatives_trading_coin_futures`、`derivatives_trading_options`、`derivatives_trading_portfolio_margin`、`derivatives_trading_portfolio_margin_pro`、`derivatives_trading_usds_futures`、`dual_investment`、`fiat`、`gift_card`、`margin_trading`、`mining`、`pay`、`rebate`、`simple_earn`、`spot`、`staking`、`sub_account`、`vip_loan`、`wallet` 等（NFT 相关 feature 在官方 README 中已标为废弃）。

## 迁移与贡献

- 从旧版连接器升级：见仓库内 [**Migration Guide**（`MIGRATION.md`）](https://github.com/binance/binance-connector-rust/blob/main/MIGRATION.md)；旧代码可在 `legacy` 分支查找。
- 贡献流程：以 **Issue / PR** 为主；生成代码若被工具覆盖，需遵循仓库中的代码生成工作流说明。

## 与本知识库其他目录的关系

- REST 行为与端点细节以 [`../币安api/spot-rest-api-CN/`](../币安api/spot-rest-api-CN/README.md) 及开放平台文档为准；SDK 为调用封装，**不替代**官方 API 说明。
- 总索引见 [`../币安现货API.md`](../币安现货API.md)。

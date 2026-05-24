# Binance Alpha API 文档

本目录保存 Binance Alpha 官方中文文档的本地知识库整理，来源为 Binance Developers Alpha 文档：

- https://developers.binance.com/docs/zh-CN/alpha/introduction
- https://developers.binance.com/docs/zh-CN/alpha/market-data/general-info
- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/token-list
- https://developers.binance.com/docs/zh-CN/alpha/market-data/websocket-market-data

## 阅读入口

- [REST-API-CN-分卷索引.md](./REST-API-CN-分卷索引.md)
- [00-概述与通用信息.md](./00-概述与通用信息.md)
- [01-市场数据REST.md](./01-市场数据REST.md)
- [02-WebSocket行情.md](./02-WebSocket行情.md)

## 完成状态

| 状态 | 项目 | 说明 |
| --- | --- | --- |
| ✓ | Alpha 知识库目录 | 已创建独立 `alpha` 目录，不混入现货文档 |
| ✓ | Alpha REST 文档整理 | 已覆盖 Token 列表、交易对信息、聚合交易、K 线、24hr ticker、完整深度 |
| ✓ | Alpha WebSocket 文档整理 | 已覆盖订阅协议和官方列出的 stream 名称 |
| ✓ | Alpha REST 客户端 | 已在 `src/binance/alpha.rs` 实现 6 个 REST 接口 |
| ✓ | Alpha WebSocket 辅助 | 已提供订阅/取消订阅消息和 stream 名称构造函数 |

## 代码入口

- `src/binance/alpha.rs`
- `src/binance/mod.rs`

Alpha 文档中的 symbol 不是普通现货 symbol。REST 示例使用类似 `ALPHA_175USDT` 的交易对，其中 `ALPHA_175` 来自 Token 列表接口的 `alphaId`。

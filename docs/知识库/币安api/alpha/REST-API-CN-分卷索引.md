# Binance Alpha 中文文档分卷索引

原始文档来自 Binance Developers Alpha 中文文档。此处按本项目使用方式拆成 3 个文件，方便查询、实现和后续维护。

| 分卷 | 文件 | 内容概要 |
| --- | --- | --- |
| 1/3 | [00-概述与通用信息.md](./00-概述与通用信息.md) | Alpha 定位、公开市场数据、Base URL、认证要求、symbol 规则 |
| 2/3 | [01-市场数据REST.md](./01-市场数据REST.md) | Token 列表、交易对信息、聚合交易、K 线、24hr ticker、完整深度 |
| 3/3 | [02-WebSocket行情.md](./02-WebSocket行情.md) | WebSocket base URL、订阅协议、ticker、trade、depth、kline streams |

## 本地实现状态

| 状态 | 模块 | 说明 |
| --- | --- | --- |
| [✓] 【完成】 | `src/binance/alpha.rs` | Alpha REST 和 WebSocket 辅助方法 |
| [✓] 【完成】 | `src/binance/mod.rs` | 已公开 `binance::alpha` 模块 |
| x | UI 页面 | 当前未新增独立 Alpha 页面，可由后续页面调用 Alpha 客户端 |

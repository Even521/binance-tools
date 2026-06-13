# 数据库表结构

运行时 SQLite 数据库默认位于：

```text
db/binance_tools.sqlite
```

建表和迁移逻辑集中在：

```text
src/db/mod.rs
```

业务读写逻辑集中在：

```text
src/db/
```

## 表索引

| 表 | 文档 | 说明 |
| --- | --- | --- |
| `spot_symbols` | [`spot_symbols.md`](./spot_symbols.md) | Binance Spot 交易对基础信息缓存 |
| `spot_klines` | [`spot_klines.md`](./spot_klines.md) | Spot K 线缓存 |
| `spot_klines_4h` | [`spot_klines_4h.md`](./spot_klines_4h.md) | Spot 回测 `4h` K 线缓存 |
| `spot_klines_1d` | [`spot_klines_1d.md`](./spot_klines_1d.md) | Spot 回测 `1d` K 线缓存 |
| `binance_market_products_cache` | [`binance_market_products_cache.md`](./binance_market_products_cache.md) | Binance Web product 市场榜单缓存 |
| `alpha_tokens` | [`alpha_tokens.md`](./alpha_tokens.md) | Binance Alpha Token 列表缓存 |
| `alpha_assets` | [`alpha_assets.md`](./alpha_assets.md) | Binance Alpha 资产列表缓存 |
| `alpha_symbols` | [`alpha_symbols.md`](./alpha_symbols.md) | Binance Alpha 交易对信息缓存 |
| `alpha_klines` | [`alpha_klines.md`](./alpha_klines.md) | Binance Alpha K 线缓存，用于日均线信号 |
| `binance_square_keys` | [`binance_square_keys.md`](./binance_square_keys.md) | 币安广场 API Key |
| `binance_square_tasks` | [`binance_square_tasks.md`](./binance_square_tasks.md) | 币安广场定时任务 |
| `binance_square_send_logs` | [`binance_square_send_logs.md`](./binance_square_send_logs.md) | 币安广场发送日志 |
| `binance_square_ai_settings` | [`binance_square_ai_settings.md`](./binance_square_ai_settings.md) | 币安广场 AI 分析开关 |
| `binance_square_ai_logs` | [`binance_square_ai_logs.md`](./binance_square_ai_logs.md) | AI 生成任务日志 |
| `ai_provider_keys` | [`ai_provider_keys.md`](./ai_provider_keys.md) | AI Provider 本地 API Key |
| `ai_chat_threads` | [`ai_chat_threads.md`](./ai_chat_threads.md) | AI Chat 历史会话，内容使用 zstd 压缩保存 |
| `ai_rules` | [`ai_rules.md`](./ai_rules.md) | 页面级 AI 分析规则，内容使用 zstd 压缩保存 |
| `tool_board_tasks` | [`tool_board_tasks.md`](./tool_board_tasks.md) | 工具模块任务看板本地提醒任务 |

## 维护约定

- 新增或修改表结构时，同步更新本目录对应文档。
- UI 层不直接拼 SQL；通过 `src/db/` 提供的函数读取数据。
- 运行时生成的 SQLite 文件不提交到版本控制。

## BTCC 补充

| 表 | 文档 | 说明 |
| --- | --- | --- |
| `btcc_wallets` | [`btcc_wallets.md`](./btcc_wallets.md) | BTCC 多钱包列表、本地地址元数据和余额缓存 |
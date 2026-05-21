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
| `binance_market_products_cache` | [`binance_market_products_cache.md`](./binance_market_products_cache.md) | Binance Web product 市场榜单缓存 |
| `binance_square_keys` | [`binance_square_keys.md`](./binance_square_keys.md) | 币安广场 API Key |
| `binance_square_tasks` | [`binance_square_tasks.md`](./binance_square_tasks.md) | 币安广场定时任务 |
| `binance_square_send_logs` | [`binance_square_send_logs.md`](./binance_square_send_logs.md) | 币安广场发送日志 |
| `binance_square_ai_settings` | [`binance_square_ai_settings.md`](./binance_square_ai_settings.md) | 币安广场 AI 分析开关 |
| `binance_square_ai_logs` | [`binance_square_ai_logs.md`](./binance_square_ai_logs.md) | AI 生成任务日志 |

## 维护约定

- 新增或修改表结构时，同步更新本目录对应文档。
- UI 层不直接拼 SQL；通过 `src/db/` 提供的函数读取数据。
- 运行时生成的 SQLite 文件不提交到版本控制。

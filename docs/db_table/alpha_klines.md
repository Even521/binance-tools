# alpha_klines

Binance Alpha K 线缓存表，用于 Alpha 日均线信号页面和 Alpha Token 列表进入的 120 日 K 线图。数据按 `symbol + interval + open_time` 去重写入，避免每次查询都请求 Binance Alpha。打开单个 K 线图时会校验最新日线是否覆盖当天，缓存停留在旧日期时会自动补齐。

| 字段 | 说明 |
| --- | --- |
| `symbol` | Alpha 交易对，例如 `ALPHA_175USDT` |
| `interval` | K 线周期，当前日均线使用 `1d` |
| `open_time` | K 线开盘时间，毫秒时间戳 |
| `open_price` | 开盘价 |
| `high_price` | 最高价 |
| `low_price` | 最低价 |
| `close_price` | 收盘价 |
| `volume` | 成交量 |
| `close_time` | K 线收盘时间，毫秒时间戳 |
| `updated_at` | 缓存写入或刷新时间 |

## 索引

- 主键：`symbol, interval, open_time`
- 普通索引：`idx_alpha_klines_symbol_interval_time`

## 使用位置

- 写入：`src/db/alpha.rs::upsert_alpha_daily_klines`
- 读取日均线信号：`src/db/alpha.rs::list_cached_alpha_usdt_daily_ma_signals`
- 读取或补齐单个交易对 120 日图表：`src/db/alpha.rs::load_or_fetch_alpha_daily_klines_blocking`
- UI：`examples/desktop-gpui/src/ui/alpha_ma_signal.rs`
- UI：`examples/desktop-gpui/src/ui/alpha.rs` 的 Alpha Token 列表 K 线按钮
- UI：`examples/desktop-gpui/src/ui/kline.rs` 的 Alpha 数据源

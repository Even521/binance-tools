# spot_klines 表结构

`spot_klines` 是旧版通用 Spot K 线缓存表，保留用于历史数据兼容和非 `4h` / `1d` 周期兜底。

现货回测的 `4h` 和 `1d` 数据已经拆到独立分表：

- [✓] 【完成】 `4h` 回测数据写入 [`spot_klines_4h`](./spot_klines_4h.md)。
- [✓] 【完成】 `1d` 回测数据写入 [`spot_klines_1d`](./spot_klines_1d.md)。
- [✓] 【完成】 现货日均线信号和现货日 K 图也已改为读取 [`spot_klines_1d`](./spot_klines_1d.md)。

本表继续保留，避免旧数据库升级后丢失历史缓存；程序启动迁移会把其中已有的 `4h` / `1d` 数据复制到对应分表。

数据库文件默认位置：

```text
db/binance_tools.sqlite
```

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS spot_klines (
    symbol TEXT NOT NULL,
    interval TEXT NOT NULL,
    open_time INTEGER NOT NULL,
    open_price REAL NOT NULL,
    high_price REAL NOT NULL,
    low_price REAL NOT NULL,
    close_price REAL NOT NULL,
    volume REAL NOT NULL,
    close_time INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (symbol, interval, open_time)
);

CREATE INDEX IF NOT EXISTS idx_spot_klines_symbol_interval_time
    ON spot_klines(symbol, interval, open_time);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `symbol` | `TEXT` | 交易对，例如 `BTCUSDT` |
| `interval` | `TEXT` | K 线周期，当前使用 `1d` |
| `open_time` | `INTEGER` | K 线开盘时间，Unix 毫秒 |
| `open_price` | `REAL` | 开盘价 |
| `high_price` | `REAL` | 最高价 |
| `low_price` | `REAL` | 最低价 |
| `close_price` | `REAL` | 收盘价 |
| `volume` | `REAL` | 成交量 |
| `close_time` | `INTEGER` | K 线收盘时间，Unix 毫秒 |
| `updated_at` | `TEXT` | 写入或刷新时间 |

## 缓存完整性判断

日均线信号按 UTC 日期窗口判断缓存是否满足：

- `end_time`：今天 00:00:00 UTC 的日 K `open_time`
- `start_time`：`end_time - (days - 1) * 86400000`
- 满足条件：`symbol + interval = 1d` 在 `[start_time, end_time]` 区间内的 K 线数量 `>= days`

## 日均线查询 SQL

```sql
WITH kline_window AS (
    SELECT
        symbol,
        COUNT(*) AS samples,
        AVG(close_price) AS average_price,
        MAX(open_time) AS latest_open_time
    FROM spot_klines
    WHERE interval = '1d'
        AND open_time BETWEEN ?1 AND ?2
    GROUP BY symbol
    HAVING samples >= ?3
)
SELECT
    s.symbol,
    s.base_asset,
    s.quote_asset,
    k.samples,
    k.average_price,
    latest.close_price
FROM kline_window k
JOIN spot_symbols s ON s.symbol = k.symbol
JOIN spot_klines latest
    ON latest.symbol = k.symbol
    AND latest.interval = '1d'
    AND latest.open_time = k.latest_open_time
WHERE s.quote_asset = 'USDT'
    AND s.status = 'TRADING'
    AND s.spot_trading_allowed = 1;
```

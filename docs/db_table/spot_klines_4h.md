# spot_klines_4h 表结构

`spot_klines_4h` 专门保存 Binance Spot 的 `4h` K 线数据，主要供现货回测页面使用。

这样做的原因是：`4h` 是回测默认周期，数据量比 `1m` 小很多，但又比 `1d` 更适合观察趋势。单独成表后，查询 `symbol + open_time` 范围更直接，也避免和其它周期混在一个大表里。

数据库文件默认位置：

```text
db/binance_tools.sqlite
```

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS spot_klines_4h (
    symbol TEXT NOT NULL,
    interval TEXT NOT NULL DEFAULT '4h',
    open_time INTEGER NOT NULL,
    open_price REAL NOT NULL,
    high_price REAL NOT NULL,
    low_price REAL NOT NULL,
    close_price REAL NOT NULL,
    volume REAL NOT NULL,
    close_time INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (symbol, open_time)
);

CREATE INDEX IF NOT EXISTS idx_spot_klines_4h_symbol_time
    ON spot_klines_4h(symbol, open_time);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `symbol` | `TEXT` | 交易对，例如 `BTCUSDT` |
| `interval` | `TEXT` | 固定为 `4h` |
| `open_time` | `INTEGER` | K 线开盘时间，Unix 毫秒 |
| `open_price` | `REAL` | 开盘价 |
| `high_price` | `REAL` | 最高价 |
| `low_price` | `REAL` | 最低价 |
| `close_price` | `REAL` | 收盘价 |
| `volume` | `REAL` | 成交量 |
| `close_time` | `INTEGER` | K 线收盘时间，Unix 毫秒 |
| `updated_at` | `TEXT` | 写入或刷新时间 |

## 写入规则

- [✓] 【完成】 Binance Vision 下载或本地 CSV 解析出的 `4h` 数据直接写入本表。
- [✓] 【完成】 主键为 `(symbol, open_time)`，重复写入同一根 K 线时更新价格、成交量和收盘时间。
- [✓] 【完成】 程序启动迁移时，会把旧 `spot_klines` 表里已有的 `interval = '4h'` 数据复制到本表。

## 回测读取规则

- [✓] 【完成】 回测周期为 `4h` 时，优先检查本表是否已有指定日期的数据。
- [✓] 【完成】 单日完整数据按 `6` 根判断，因为一天有 6 根 4 小时 K 线。
- [✓] 【完成】 本表数据不完整时，才读取本地 CSV 或从 Binance Vision 下载缺失日期。
- [✓] 【完成】 最终回测从本表按 `symbol + open_time BETWEEN start AND end` 查询。

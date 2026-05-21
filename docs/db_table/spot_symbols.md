# spot_symbols 表结构

`spot_symbols` 存储 Binance Spot `exchange_info` 返回的现货交易对基础信息，作为桌面 UI 查询币种页面的本地缓存。

数据库文件默认位置：

```text
db/binance_tools.sqlite
```

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS spot_symbols (
    symbol TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    base_asset TEXT NOT NULL,
    quote_asset TEXT NOT NULL,
    base_asset_precision INTEGER NOT NULL,
    quote_asset_precision INTEGER NOT NULL,
    order_types TEXT NOT NULL,
    spot_trading_allowed INTEGER NOT NULL,
    margin_trading_allowed INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_spot_symbols_base_asset
    ON spot_symbols(base_asset);

CREATE INDEX IF NOT EXISTS idx_spot_symbols_quote_asset
    ON spot_symbols(quote_asset);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `symbol` | `TEXT` | 交易对，主键，例如 `BTCUSDT` |
| `status` | `TEXT` | 交易状态 |
| `base_asset` | `TEXT` | 基础资产 |
| `quote_asset` | `TEXT` | 计价资产 |
| `base_asset_precision` | `INTEGER` | 基础资产精度 |
| `quote_asset_precision` | `INTEGER` | 计价资产精度 |
| `order_types` | `TEXT` | 订单类型列表，使用逗号连接 |
| `spot_trading_allowed` | `INTEGER` | 是否允许现货交易，`1` 是，`0` 否 |
| `margin_trading_allowed` | `INTEGER` | 是否允许杠杆交易，`1` 是，`0` 否 |
| `updated_at` | `TEXT` | 写入或刷新时间 |

## 统计 SQL

现货总数按 `base_asset` 去重统计：

```sql
SELECT COUNT(DISTINCT base_asset)
FROM spot_symbols
WHERE base_asset <> '';
```

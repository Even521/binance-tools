# binance_market_products_cache

`binance_market_products_cache` 缓存 Binance Web product 接口返回的市场榜单数据，用于 Dashboard 首页市场榜单、市场热力图和右侧 AI 分析。

数据来源：

```text
https://www.binance.com/bapi/asset/v2/public/asset-service/product/get-products
```

代码位置：

```text
src/binance/market.rs
src/db/market.rs
examples/desktop-gpui/src/ui/market.rs
examples/desktop-gpui/src/ui/heatmap.rs
```

## 缓存策略

- 缓存有效期为 5 分钟。
- 读取时先检查表内最新 `fetched_at`。
- 如果最新数据在 5 分钟内，直接读取 SQLite。
- 如果缓存为空或超过 5 分钟，重新请求 Binance Web product 接口。
- 写入新数据时，在同一个事务中先清空旧数据，再批量插入新数据。

## 热力图使用

- `price_change_percent` 用于决定方块颜色：上涨绿色，下跌红色，持平灰色。
- `market_cap` 和 `quote_volume` 用于热力图方块权重，页面可在市值和成交额之间切换。
- `quote_asset` 用于热力图计价资产筛选。
- 点击热力图方块会进入对应现货交易对的 1 日 K 线图。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS binance_market_products_cache (
    symbol TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    base_asset TEXT NOT NULL,
    quote_asset TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    quote_name TEXT NOT NULL,
    open_price REAL,
    high_price REAL,
    low_price REAL,
    last_price REAL,
    volume REAL,
    quote_volume REAL,
    circulating_supply REAL,
    market_cap REAL,
    price_change_percent REAL,
    partition TEXT NOT NULL,
    partition_name TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    is_etf INTEGER NOT NULL DEFAULT 0,
    is_trading INTEGER NOT NULL DEFAULT 0,
    fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_binance_market_products_quote_asset
    ON binance_market_products_cache(quote_asset);

CREATE INDEX IF NOT EXISTS idx_binance_market_products_fetched_at
    ON binance_market_products_cache(fetched_at);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `symbol` | `TEXT` | 交易对主键，例如 `AIUSDT` |
| `status` | `TEXT` | 交易状态 |
| `base_asset` | `TEXT` | 基础资产 |
| `quote_asset` | `TEXT` | 计价资产 |
| `asset_name` | `TEXT` | 基础资产展示名称 |
| `quote_name` | `TEXT` | 计价资产展示名称 |
| `open_price` | `REAL` | 24 小时开盘价 |
| `high_price` | `REAL` | 24 小时最高价 |
| `low_price` | `REAL` | 24 小时最低价 |
| `last_price` | `REAL` | 最新价 |
| `volume` | `REAL` | 24 小时基础资产成交量 |
| `quote_volume` | `REAL` | 24 小时计价成交额 |
| `circulating_supply` | `REAL` | 流通供应量 |
| `market_cap` | `REAL` | 市值，按 `last_price * circulating_supply` 计算 |
| `price_change_percent` | `REAL` | 24 小时涨跌幅，按 `(last_price - open_price) / open_price * 100` 计算 |
| `partition` | `TEXT` | 市场分区代码 |
| `partition_name` | `TEXT` | 市场分区名称 |
| `tags_json` | `TEXT` | 标签数组 JSON，例如 `["AI","Launchpool"]` |
| `is_etf` | `INTEGER` | 是否 ETF，`1` 是，`0` 否 |
| `is_trading` | `INTEGER` | 是否可交易，`1` 是，`0` 否 |
| `fetched_at` | `TEXT` | 本批缓存写入时间 |

## 使用场景

- `MarketProductsPage`：按 `quote_asset` 和 `is_trading` 筛选，默认按 `price_change_percent` 降序展示。
- `AI 分析`：读取当前表格排序后的前 50 条，构造精简 JSON prompt，发送到右侧 AI Chat 面板。

## 缓存有效性 SQL

```sql
SELECT CAST(strftime('%s', 'now') - strftime('%s', MAX(fetched_at)) AS INTEGER)
FROM binance_market_products_cache;
```

返回值在 `0..300` 秒内时，认为缓存可用。

# alpha_tokens

Binance Alpha Token 列表缓存表。页面优先读取本表，空表或手动刷新时请求 Binance。Alpha Token 列表页和 Alpha 热力图页共用本表。

| 字段 | 说明 |
| --- | --- |
| `alpha_id` | Alpha ID，主键，例如 `ALPHA_175` |
| `token_id` | Token ID |
| `chain_id` | 链 ID |
| `chain_name` | 链名称 |
| `contract_address` | 合约地址 |
| `name` | Token 名称 |
| `symbol` | Token 符号 |
| `price` | 价格，保留接口原始字符串 |
| `percent_change_24h` | 24h 涨跌幅，保留接口原始字符串 |
| `volume_24h` | 24h 成交量 |
| `market_cap` | 市值 |
| `liquidity` | 流动性 |
| `listing_cex` | 是否 CEX 上架 |
| `cex_coin_name` | CEX 币种名称 |
| `stock_state` | 证券代币相关状态 |
| `cex_off_display` | CEX 展示隐藏状态 |
| `hot_tag` | 热门标签 |
| `trade_decimal` | 交易精度 |
| `listing_time` | 上线时间 |
| `score` | 积分/评分 |
| `mul_point` | 积分倍率 |
| `extra_json` | 未显式建列字段的 JSON |
| `updated_at` | 更新时间 |

## 热力图使用

- `percent_change_24h` 用于决定方块颜色：上涨绿色，下跌红色，持平灰色。
- `market_cap` 用于 Alpha 热力图的 `市值` 模式。
- `volume_24h` 用于 Alpha 热力图的 `成交额` 模式。
- `chain_name` 和 `chain_id` 用于链筛选。
- `alpha_id` 用于构造 Alpha 交易对，例如 `ALPHA_175` 转为 `ALPHA_175USDT` 后打开 1 日 K 线图。

# alpha_symbols

Binance Alpha exchange info 交易对缓存表。`quote_asset` 用于页面分类筛选。

| 字段 | 说明 |
| --- | --- |
| `symbol` | 交易对，主键 |
| `status` | 状态 |
| `base_asset` | 基础资产 |
| `quote_asset` | 计价资产 |
| `price_precision` | 价格精度 |
| `quantity_precision` | 数量精度 |
| `base_asset_precision` | 基础资产精度 |
| `quote_precision` | 计价资产精度 |
| `filters_json` | 交易规则 JSON |
| `order_types_json` | 支持订单类型 JSON |
| `updated_at` | 更新时间 |

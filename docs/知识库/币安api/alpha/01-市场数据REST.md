# 01 - 市场数据 REST

来源：

- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/token-list
- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/get-exchange-info
- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/aggregated-trades
- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/klines
- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/24hr-ticker-price-change
- https://developers.binance.com/docs/zh-CN/alpha/market-data/rest-api/full-depth

## 接口清单

| 状态 | 名称 | HTTP | Path | 本地函数 |
| --- | --- | --- | --- | --- |
| [✓] 【完成】 | Token 列表 | GET | `/bapi/defi/v1/public/wallet-direct/buw/wallet/cex/alpha/all/token/list` | `AlphaClient::token_list` |
| [✓] 【完成】 | 交易对信息 | GET | `/bapi/defi/v1/public/alpha-trade/get-exchange-info` | `AlphaClient::exchange_info` |
| [✓] 【完成】 | 聚合交易 | GET | `/bapi/defi/v1/public/alpha-trade/agg-trades` | `AlphaClient::aggregate_trades` |
| [✓] 【完成】 | K 线 | GET | `/bapi/defi/v1/public/alpha-trade/klines` | `AlphaClient::klines` |
| [✓] 【完成】 | 24hr 价格变动 | GET | `/bapi/defi/v1/public/alpha-trade/ticker` | `AlphaClient::ticker_24hr` |
| [✓] 【完成】 | 完整深度 | GET | `/bapi/defi/v1/public/alpha-trade/fullDepth` | `AlphaClient::full_depth` |

## Token 列表

```text
GET /bapi/defi/v1/public/wallet-direct/buw/wallet/cex/alpha/all/token/list
```

参数：无。

主要返回字段：

| 字段 | 说明 |
| --- | --- |
| `tokenId` | Token 内部 ID |
| `alphaId` | Alpha 交易 ID，例如 `ALPHA_175` |
| `chainId` | 链 ID |
| `chainName` | 链名称 |
| `contractAddress` | 合约地址 |
| `name` | Token 名称 |
| `symbol` | Token 符号 |
| `price` | 当前价格 |
| `percentChange24h` | 24 小时涨跌幅 |
| `volume24h` | 24 小时成交量 |
| `marketCap` | 市值 |
| `liquidity` | 流动性 |
| `listingCex` | 是否已在 CEX 上线 |
| `cexCoinName` | CEX 币种名称，证券代币识别时可作为辅助字段 |
| `stockState` | 股票/证券相关状态，当前 UI 用作证券代币分类的优先判断 |
| `cexOffDisplay` | CEX 展示关闭/隐藏相关状态，官方示例包含但未进一步解释 |
| `hotTag` | 是否热度标签 |
| `score` | 积分/评分字段，官方示例包含但未进一步解释 |
| `mulPoint` | 积分倍率字段，例如 UI 可展示为 `x2`、`x4` |
| `listingTime` | 上线时间，毫秒 |

## 交易对信息

```text
GET /bapi/defi/v1/public/alpha-trade/get-exchange-info
```

参数：无。

主要返回字段：

| 字段 | 说明 |
| --- | --- |
| `timezone` | 交易数据时区 |
| `assets[].asset` | 资产符号 |
| `symbols[].symbol` | 交易对，例如 `ALPHA_105USDT` |
| `symbols[].status` | 交易状态，例如 `TRADING` |
| `symbols[].baseAsset` | 基础资产，例如 `ALPHA_105` |
| `symbols[].quoteAsset` | 计价资产，例如 `USDT` |
| `pricePrecision` | 价格精度 |
| `quantityPrecision` | 数量精度 |
| `filters[]` | 价格、数量、名义金额、百分比价格等过滤器 |
| `orderTypes[]` | 支持的订单类型 |

常见过滤器包括 `PRICE_FILTER`、`LOT_SIZE`、`MAX_NUM_ORDERS`、`MIN_NOTIONAL`、`MAX_NOTIONAL`、`NOTIONAL`、`PERCENT_PRICE`、`PERCENT_PRICE_BY_SIDE`。

## 聚合交易

```text
GET /bapi/defi/v1/public/alpha-trade/agg-trades
```

参数：

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `symbol` | 是 | 例如 `ALPHA_118USDC` |
| `fromId` | 否 | 起始成交 ID |
| `startTime` | 否 | 起始时间戳，毫秒 |
| `endTime` | 否 | 结束时间戳，毫秒 |
| `limit` | 否 | 默认 500，最大 1000 |

返回字段：`a` 聚合成交 ID、`p` 价格、`q` 数量、`f` 首笔成交 ID、`l` 末笔成交 ID、`T` 时间戳、`m` 买方是否做市商。

## K 线

```text
GET /bapi/defi/v1/public/alpha-trade/klines
```

参数：

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `symbol` | 是 | 例如 `ALPHA_175USDT` |
| `interval` | 是 | `1s`, `15s`, `1m`, `3m`, `5m`, `15m`, `30m`, `1h`, `2h`, `4h`, `6h`, `8h`, `12h`, `1d`, `3d`, `1w`, `1M` |
| `limit` | 否 | 默认 500，最大 1500 |
| `startTime` | 否 | 起始时间戳，毫秒 |
| `endTime` | 否 | 结束时间戳，毫秒 |

每根 K 线数组字段顺序：

1. 开盘时间
2. 开盘价
3. 最高价
4. 最低价
5. 收盘价
6. 成交量
7. 收盘时间
8. 计价资产成交量
9. 成交笔数
10. 主动买入基础资产成交量
11. 主动买入计价资产成交量
12. 忽略字段

## 24hr 价格变动

```text
GET /bapi/defi/v1/public/alpha-trade/ticker
```

参数：

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `symbol` | 是 | 例如 `ALPHA_175USDT` |

返回字段包括 `priceChange`、`priceChangePercent`、`weightedAvgPrice`、`lastPrice`、`lastQty`、`openPrice`、`highPrice`、`lowPrice`、`volume`、`quoteVolume`、`openTime`、`closeTime`、`firstId`、`lastId`、`count`。

## 完整深度

```text
GET /bapi/defi/v1/public/alpha-trade/fullDepth
```

参数：

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `symbol` | 是 | 例如 `ALPHA_175USDT` |
| `limit` | 否 | 默认 500，可选 `5`, `10`, `20`, `50`, `100`, `500`, `1000` |

返回字段：

| 字段 | 说明 |
| --- | --- |
| `lastUpdateId` | 订单簿最后更新 ID |
| `symbol` | 交易对 |
| `bids` | 买盘数组，元素为 `[price, quantity]` |
| `asks` | 卖盘数组，元素为 `[price, quantity]` |
| `E` | 事件时间，毫秒 |
| `T` | 交易时间，毫秒 |

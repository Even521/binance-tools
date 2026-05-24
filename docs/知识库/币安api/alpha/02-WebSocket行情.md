# 02 - WebSocket 行情

来源：

- https://developers.binance.com/docs/zh-CN/alpha/market-data/websocket-market-data

## 基本信息

Alpha WebSocket base URL：

```text
wss://nbstream.binance.com/w3w/wsa/stream
```

订阅、取消订阅、查看已订阅 stream 都通过发送 JSON 消息完成。`id` 用于区分请求和响应，支持 64 位有符号整数、最大长度 36 的字母数字字符串或 `null`。响应中的 `result = null` 表示请求发送成功。

## 订阅协议

订阅：

```json
{
  "method": "SUBSCRIBE",
  "params": ["came@allTokens@ticker24"],
  "id": 1
}
```

取消订阅：

```json
{
  "method": "UNSUBSCRIBE",
  "params": ["came@allTokens@ticker24"],
  "id": 1
}
```

查看已订阅：

```json
{
  "method": "LIST_SUBSCRIPTION",
  "id": 3
}
```

## Stream 清单

| 状态 | Stream | 说明 | 本地辅助 |
| --- | --- | --- | --- |
| ✓ | `came@allTokens@ticker24` | 全部 Alpha Token 24h ticker 列表 | `alpha_all_tokens_ticker_24hr_stream` |
| ✓ | `<symbol>@aggTrade` | 聚合交易 | `alpha_symbol_stream(symbol, "aggTrade")` |
| ✓ | `<symbol>@fulldepth@<interval>` | 完整深度，包含 UI 和 API 订单 | `alpha_full_depth_stream` |
| ✓ | `came@contractAddress@chainId@kline_<interval>` | 合约地址维度 K 线 | `alpha_contract_kline_stream` |
| ✓ | `<symbol>@bookTicker` | 最优买卖价 | `alpha_symbol_stream(symbol, "bookTicker")` |
| ✓ | `!bookTicker` | 全市场最优买卖价 |
| ✓ | `<symbol>@miniTicker` | 单交易对 24h mini ticker | `alpha_symbol_stream(symbol, "miniTicker")` |
| ✓ | `!miniTicker@arr` | 全市场 mini ticker |
| ✓ | `<symbol>@ticker` | 单交易对 24h ticker | `alpha_symbol_stream(symbol, "ticker")` |
| ✓ | `!ticker@arr` | 全市场 24h ticker |
| ✓ | `<symbol>@trade` | 原始成交 | `alpha_symbol_stream(symbol, "trade")` |
| ✓ | `<symbol>@depth<levels>@<interval>` | 部分深度，仅 UI 订单 | `alpha_partial_depth_stream` |
| ✓ | `<symbol>@kline_<interval>` | 交易对 K 线 | `alpha_symbol_kline_stream` |

## interval 规则

完整深度：

```text
0ms, 100ms, 500ms
```

合约地址 K 线：

```text
1s, 1m, 5m, 15m, 1h, 4h, 1d
```

交易对 K 线：

```text
1m, 3m, 5m, 15m, 30m, 1h, 2h, 4h, 6h, 8h, 12h, 1d, 3d, 1w, 1M
```

部分深度：

```text
levels: 5, 10, 20
interval: 0ms, 100ms, 500ms
```

## 本地实现

| 状态 | 项目 | 说明 |
| --- | --- | --- |
| ✓ | 订阅消息 | `alpha_ws_subscribe_message` |
| ✓ | 取消订阅消息 | `alpha_ws_unsubscribe_message` |
| ✓ | 已订阅查询消息 | `alpha_ws_list_subscriptions_message` |
| ✓ | stream 名称构造 | symbol 会统一转小写，匹配官方示例 |

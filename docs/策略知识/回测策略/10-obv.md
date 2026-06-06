# OBV 能量潮策略

## 创始人 / 来源

OBV 由 Joseph Granville 在 1963 年出版的 *Granville's New Key to Stock Market Profits* 中推广。

## 核心原理

OBV 将成交量按照价格涨跌方向累计。价格上涨时成交量加到 OBV，价格下跌时成交量从 OBV 中减去。它试图观察资金流向是否支持价格趋势。

## 计算公式

```text
如果 Close_today > Close_yesterday：
OBV_today = OBV_yesterday + Volume_today

如果 Close_today < Close_yesterday：
OBV_today = OBV_yesterday - Volume_today

如果 Close_today = Close_yesterday：
OBV_today = OBV_yesterday
```

## 当前回测规则

- 参数：OBV 均线、价格均线、单次仓位%、止盈%、止损%。
- 买入：OBV 上穿 OBV 均线，且价格高于价格均线。
- 卖出：OBV 下穿 OBV 均线，或触发止盈/止损。

## 适合行情

- 量价同步上涨。
- 资金流入明显。
- 趋势确认。

## 不适合行情

- 异常成交量频繁。
- 单一交易所成交量代表性不足。
- 横盘刷量。

## 历史故事

Granville 认为成交量往往先于价格变化，因为资金进出会留下痕迹。OBV 的故事就是“聪明钱先行动，价格后反应”。在数字资产里，这个思想仍有用，但需要注意交易所成交量口径。


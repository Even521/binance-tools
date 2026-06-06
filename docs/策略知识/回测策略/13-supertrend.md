# SuperTrend 策略

## 创始人 / 来源

SuperTrend 常被归功于 Olivier Seban。相比 RSI、MACD、布林带等经典指标，它的历史资料集中度较低，但在现代图表软件和交易社区中非常常见。

## 核心原理

SuperTrend 使用 ATR 构造动态趋势线。价格突破波动带后趋势翻多，跌破波动带后趋势翻空。它把趋势判断和移动止损合在一起。

## 计算公式

```text
ATR = 平均真实波幅
Middle = (High + Low) / 2
Upper Band = Middle + Multiplier * ATR
Lower Band = Middle - Multiplier * ATR

趋势翻多：Close 突破上方波动带
趋势翻空：Close 跌破下方波动带
```

## 当前回测规则

- 参数：ATR 周期、ATR 倍数、单次仓位%、止盈%、止损%。
- 买入：SuperTrend 趋势从空转多，且当前收盘价高于前一根收盘价。
- 卖出：趋势翻空，或触发止盈/止损。

## 适合行情

- 顺势行情。
- 趋势持续性较好的交易对。
- 需要波动率止损的系统。

## 不适合行情

- 横盘震荡。
- 上下插针频繁。
- ATR 倍数设置过小。

## 历史故事

SuperTrend 受到欢迎，是因为它在图表上很直观：线在价格下方时偏多，线在价格上方时偏空。它不像 MACD 那样放在副图，而是直接贴着价格走势，方便交易者把趋势和止损位置一起观察。


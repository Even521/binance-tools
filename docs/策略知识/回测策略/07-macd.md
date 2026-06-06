# MACD 策略

## 创始人 / 来源

MACD 通常归功于 Gerald Appel，他在 1970 年代后期推广了该指标。

## 核心原理

MACD 用快 EMA 和慢 EMA 的差值表示趋势动量，再用信号线观察动量变化。金叉表示动量转强，死叉表示动量转弱。

## 计算公式

```text
MACD Line = EMA(fast) - EMA(slow)
Signal Line = EMA(MACD Line, signal)
Histogram = MACD Line - Signal Line

买入：MACD Line 上穿 Signal Line，且 MACD Line > 0
卖出：MACD Line 下穿 Signal Line
```

## 当前回测规则

- 参数：快线 EMA、慢线 EMA、信号线、单次仓位%、止损%。
- 买入：MACD Line 上穿 Signal Line，且位于零轴上方。
- 卖出：MACD Line 下穿 Signal Line，或触发止损。

## 适合行情

- 趋势启动。
- 趋势延续。
- 需要动量确认的行情。

## 不适合行情

- 低波动横盘。
- 快速插针。
- 价格已经大幅拉升后的追涨阶段。

## 历史故事

MACD 的流行来自它同时表达趋势和动量。很多交易者不满足于只看均线交叉，于是使用 MACD 判断“趋势是否真的有力量”。它后来成为图表软件里最常见的指标之一。


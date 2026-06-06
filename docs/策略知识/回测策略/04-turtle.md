# Turtle 海龟策略

## 创始人 / 来源

海龟交易法由 Richard Dennis 和 William Eckhardt 在 1980 年代实验和推广。

## 核心原理

用突破确认趋势启动，用 ATR 衡量波动并设置止损。价格突破过去一段时间高点时买入，跌破较短周期低点或触发 ATR 止损时卖出。

## 计算公式

```text
入场高点 = 最近 entry_window 根 K 线最高价
退出低点 = 最近 exit_window 根 K 线最低价

TR = max(High - Low, abs(High - PreviousClose), abs(Low - PreviousClose))
ATR = TR 的均值

买入：Close > 入场高点
卖出：Close < 退出低点 或 Close <= 入场价 - ATR * 止损倍数
```

## 当前回测规则

- 参数：突破周期、退出周期、单次仓位%、ATR 周期、止损 ATR。
- 买入：收盘价突破过去突破周期最高价。
- 卖出：收盘价跌破退出周期最低价，或触发 ATR 止损。

## 适合行情

- 强趋势。
- 趋势持续时间长。
- 波动大但方向明确。

## 不适合行情

- 横盘震荡。
- 假突破频繁。
- 流动性差导致插针多的币种。

## 历史故事

Richard Dennis 和 William Eckhardt 争论交易能力是天生还是能训练出来，于是招募普通人教授规则。这批学员被称为“Turtles”。海龟交易法的核心不是神秘信号，而是严格规则、仓位管理和接受连续小亏以等待大趋势。


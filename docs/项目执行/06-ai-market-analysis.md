# 06 市场榜单 AI 分析

本文记录 Dashboard 首页“市场榜单”的 AI 分析功能。

## 功能目标

市场榜单页面提供 `AI 分析` 按钮。用户点击后，程序把当前页面已经筛选、排序后的市场数据整理为精简 JSON，发送给右侧 AI Chat 面板，由当前选中模型输出 1 个适合继续关注的币种短文案。

核心原则：

- 行情数据来自本地 SQLite 缓存，不让模型自行猜测或抓取网页。
- 只分析当前计价市场，例如 `USDT`、`FDUSD`。
- 只传当前表格排序后的前 50 条精简数据。
- 右侧 Chat UI 只展示摘要，不展示完整 JSON prompt，避免撑乱界面。
- 输出面向币安广场发布场景，避免交易诱导、收益承诺和容易触发风控的敏感词。

## 数据来源

市场榜单数据来自：

```text
src/db/market.rs
src/binance/market.rs
```

底层接口：

```text
https://www.binance.com/bapi/asset/v2/public/asset-service/product/get-products
```

缓存表：

```text
binance_market_products_cache
```

缓存规则：表中最新 `fetched_at` 在 5 分钟内时直接读 SQLite；超过 5 分钟时重新请求 Binance Web product 接口，并在事务中清空旧数据后批量写入新数据。

## 交互流程

1. 用户在 Dashboard 首页选择计价市场，例如 `USDT`。
2. 市场榜单表格展示该计价市场中 `is_trading = 1` 的交易对。
3. 表格默认按 `price_change_percent` 从高到低排序。
4. 用户点击顶部 `AI 分析`。
5. `MarketProductsPage` 读取当前表格中的前 50 条数据。
6. 页面调用 `build_market_analysis_prompt` 构造完整 prompt。
7. 页面通过 `MarketProductsEvent::AnalyzeWithAi` 把 prompt 发送给 Dashboard。
8. Dashboard 打开右侧 AI Chat 面板，调用 `AiChatPanel::submit_external_prompt`。
9. AI Chat 以流式方式展示模型回复。

## 发送给 AI 的数据

当前限制：

```text
AI_ANALYSIS_LIMIT = 50
```

每条数据包含：

| 字段 | 说明 |
| --- | --- |
| `symbol` | 交易对，例如 `AIUSDT` |
| `base_asset` | 基础资产，例如 `AI` |
| `asset_name` | 币种展示名称 |
| `price` | 最新价 |
| `change_24h_percent` | 24 小时涨跌幅 |
| `high_24h` | 24 小时最高价 |
| `low_24h` | 24 小时最低价 |
| `quote_volume` | 24 小时计价成交额 |
| `market_cap` | 流通市值 |
| `circulating_supply` | 流通量 |
| `tags` | Binance 返回的标签数组 |

示例结构：

```json
{
  "quote_asset": "USDT",
  "limit": 50,
  "sample_count": 50,
  "products": [
    {
      "symbol": "AIUSDT",
      "base_asset": "AI",
      "asset_name": "Sleepless AI",
      "price": 0.0321,
      "change_24h_percent": 7.36,
      "high_24h": 0.0334,
      "low_24h": 0.028,
      "quote_volume": 4810000,
      "market_cap": 8390000,
      "circulating_supply": 261250000,
      "tags": ["AI", "Seed", "Launchpool"]
    }
  ]
}
```

## Prompt 约束

Prompt 要求模型：

- 只基于 JSON 数据筛选。
- 不编造外部行情、新闻或实时价格。
- 只选出最值得关注的 1 个币种。
- 输出必须严格为一行：`$币种 极短理由`。
- 中文汉字数量必须不少于 60 个且不超过 90 个；`$币种代码`、空格、标点和英文字母不计入这 60 到 90 个汉字。
- 不要标题、Markdown 表格、风险提示或数据来源解释。
- 禁止使用“买入、梭哈、暴涨、翻倍、稳赚、必涨、带单、喊单、内幕、财富自由、保证收益、合约、杠杆、冲、无脑买、抄底、逃顶、目标价、止盈止损”等敏感或交易诱导词。
- 不承诺收益，不诱导交易，不写价格预测。
- 不输出具体价格、涨跌幅数值、目标位、百分比或任何预测性数字。
- 理由围绕 2 到 3 个 JSON 重点指标写，例如成交额相对位置、流通市值形成的盘面厚度、标签/赛道辨识度、流通供给、流动性改善、市场讨论焦点。
- 不要只写“热度、成交活跃、板块关注度、流动性”这些泛词，要说明该币相对同批标的的具体观察点。
- 避免反复套用“板块关注度持续提升、成交活跃、流动性良好、市场热度持续走高”等固定模板。
- 避免把“值得跟踪/值得追踪/值得关注”等短语作为固定结尾。

示例输出：

```text
$AI 成交承接在榜单里更主动，赛道标签辨识度清晰，流通市值给盘面留出一定厚度，市场讨论焦点没有明显分散，后续反馈可以多看
```

Dashboard 右侧 Chat 的市场分析用于交互式阅读；币安广场任务页的 AI 草稿生成也复用相同的市场数据和风控方向，但会把结果写入 `binance_square_tasks`，默认状态为 `draft`。

## UI 展示

右侧聊天面板中用户消息只显示摘要：

```text
AI 分析 USDT 市场榜单（前 50 条精简数据）
```

实际发送给模型的是完整 prompt 和 JSON 数据。聊天历史再次发送给模型时使用真实 prompt，界面渲染时使用摘要。

- [✓] 【完成】 页面触发 AI 分析时会先加载最后一条历史会话，避免新结果写入空白临时窗口。
- [✓] 【完成】 当前页面规则从 `ai_rules` 表读取后追加到真实 prompt。
- [✓] 【完成】 当次使用的规则会作为快照写入聊天历史，后续修改规则不会影响历史消息。

## Token 控制

50 条精简数据通常在 `4k - 8k tokens` 左右，取决于标签数量和币种名称长度。输出由 prompt 强约束为 60 到 90 个中文汉字，`$币种代码`、空格、标点和英文字母不计入限制。

后续如果需要更强分析能力，可新增两种模式：

- `AI 分析当前市场`：当前模式，传当前市场前 50 条。
- `AI 分析全市场`：先在 Rust 中聚合 top gainers、top volume、tag stats，再传汇总 JSON，不传全量原始记录。

## 相关代码

| 功能 | 代码 |
| --- | --- |
| 市场榜单 UI | `examples/desktop-gpui/src/ui/market.rs` |
| Dashboard 事件接入 | `examples/desktop-gpui/src/ui/dashboard.rs` |
| AI Chat 外部 prompt | `examples/desktop-gpui/src/ui/ai/chat.rs` |
| Binance Web product 请求 | `src/binance/market.rs` |
| 市场缓存读写 | `src/db/market.rs` |

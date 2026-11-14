# spot-ai-analysis

现货 AI 研究分析独立 Rust 领域包，不依赖 GPUI、桌面窗口或 Binance SDK。

- `model`：行情输入、日线均线输入、分析结果 DTO。
- `scoring`：USDT 候选筛选、六维评分、排序。
- `prompt`：AI 研究报告提示词。
- 桌面端负责 API/SQLite 数据获取、DTO 转换和 UI 展示。
- 缺失的多周期指标、新闻情绪、盘口深度必须明确标记；本包不执行交易。

验证：`cargo test -p spot-ai-analysis`

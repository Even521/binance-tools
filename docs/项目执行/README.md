# 项目执行文档

本目录记录当前项目功能设计、实现范围和维护约定。文档以当前代码为准，主要覆盖根 crate `src/` 和桌面应用 `examples/desktop-gpui/`。

## 文档索引

| 文件 | 说明 |
| --- | --- |
| [`01-项目框架.md`](./01-项目框架.md) | 仓库目标、技术栈、目录分层和运行命令 |
| [`02-项目UI.md`](./02-项目UI.md) | `desktop-gpui` 当前页面、主题、布局和 UI/业务边界 |
| [`03-币安现货.md`](./03-币安现货.md) | 现货币种、日均线信号和 K 线页面 |
| [`04-币安广场.md`](./04-币安广场.md) | 币安广场 Key、任务、发送日志和调度器 |
| [`05-ai.md`](./05-ai.md) | Zed 风格 AI 模块、Provider 配置、Chat 面板和流式输出 |
| [`06-ai-market-analysis.md`](./06-ai-market-analysis.md) | 市场榜单 AI 分析按钮、精简 JSON 和 prompt 约束 |
| [`07-binance-alpha.md`](./07-binance-alpha.md) | Binance Alpha 文档入库、REST 客户端和 WebSocket 辅助 |
| [`08-工具.md`](./08-工具.md) | 工具菜单和文档转换 |

## 当前主功能

- ✓ Dashboard 首页展示 Binance Web product 市场榜单，数据缓存到 SQLite 5 分钟。
- ✓ 现货菜单包含市场榜单、币种列表和日均线信号；日均线信号可进入 K 线图。
- ✓ 币安广场菜单包含 Key 设置、任务页面和发送消息日志。
- ✓ 右侧 AI Agent 面板支持 OpenAI-compatible provider 配置、模型选择、流式输出、重试、继续和复制。
- ✓ 右侧 AI Agent 面板支持 Zed 风格历史会话列表和会话恢复。
- ✓ 市场榜单可点击 `AI 分析`，把当前筛选市场前 50 条精简数据发送到右侧 AI 面板分析。
- ✓ 页面级 AI 规则保存到 SQLite `ai_rules` 表，按页面独立读取并压缩存储。
- ✓ 右侧 AI Agent 的 `Rules` 弹窗支持新增、搜索、编辑、Markdown/Text 格式选择、保存、复制、刷新和启用/禁用规则。
- ✓ AI 历史会话保存规则快照，后续修改规则不会覆盖旧分析记录。
- ✓ 币安广场 AI 生成每日最多 100 条，失败最多重试 3 次且间隔 5 分钟。
- ✓ Binance Alpha 官方文档已入库，REST 市场数据接口、WebSocket stream 辅助和顶部 Alpha 菜单已实现。
- ✓ 工具菜单包含文档转换，支持 Markdown / HTML 本地互转、复制和保存。

## 维护约定

- 新增页面或跨模块功能时，在本目录增加或更新对应文档。
- 数据库表结构请同步更新 [`../db_table`](../db_table/README.md)。
- 文档中的路径尽量写到具体模块，避免只描述 UI 截图。
- 已经完成并落地到代码的功能项统一用 `✓` 标记；尚未实现或只保留占位的功能用 `x` 标记。

# ai_rules

AI 页面规则表。每个页面或功能使用独立 `context_key`，用于保存该页面专属的 AI 分析规则。

规则内容可能很长，因此 `data` 使用 BLOB 存储，并通过 `data_type` 标记压缩格式。

| 字段 | 类型 | 说明 |
|---|---|---|
| `context_key` | `TEXT PRIMARY KEY` | 规则上下文，例如 `market_products`、`spot_symbols`、`square_tasks` |
| `label` | `TEXT` | UI 展示名称，例如 `市场榜单` |
| `format` | `TEXT` | 规则文本格式，`text` 或 `markdown` |
| `data_type` | `TEXT` | `zstd` 或兼容旧数据的 `text` |
| `data` | `BLOB` | 规则内容。新写入统一使用 zstd 压缩后的 UTF-8 文本 |
| `enabled` | `INTEGER` | 是否启用，`1` 启用，`0` 禁用 |
| `created_at` | `TEXT` | 创建时间 |
| `updated_at` | `TEXT` | 更新时间 |

## 使用方式

- ✓ 页面触发 AI 分析时传入自己的 `context_key`。
- ✓ 发送 prompt 前读取 `ai_rules.context_key` 对应规则。
- ✓ 如果规则存在、启用且内容非空，则解压后追加到 prompt。
- ✓ 不同页面可以拥有不同规则，避免市场榜单、现货、币安广场任务混用同一套分析条件。
- ✓ 默认规则只在缺失时写入数据库，用户后续修改不会被启动迁移覆盖。
- ✓ 新规则内容统一使用 `zstd` 压缩保存，读取时兼容旧 `text` 数据。
- ✓ 外部 AI 分析会把当次规则内容作为快照写入 `ai_chat_threads` 历史会话。
- ✓ 桌面端 `Rules` 页面可新增、搜索、编辑、保存、复制、刷新和启用/禁用规则。
- ✓ 桌面端 `Rules` 页面可选择规则格式为 `Text` 或 `Markdown`。
- ✓ Markdown 规则支持点击眼睛图标打开预览弹窗；预览只影响 UI 展示，数据库仍保存原始规则正文。

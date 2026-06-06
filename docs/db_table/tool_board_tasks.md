# tool_board_tasks

工具模块任务看板表。用于保存本地提醒任务，不和币安广场发送任务混用。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `INTEGER PRIMARY KEY AUTOINCREMENT` | 任务 ID |
| `title` | `TEXT` | 任务标题 |
| `note` | `TEXT` | 任务备注，默认为空 |
| `due_at` | `TEXT` | 提醒时间，格式 `YYYY-MM-DD HH:MM:SS` |
| `completed` | `INTEGER` | 是否完成，`1` 完成，`0` 未完成 |
| `created_at` | `TEXT` | 创建时间 |
| `updated_at` | `TEXT` | 更新时间 |

## 索引

- [✓] 【完成】 `idx_tool_board_tasks_completed_due`：按完成状态和提醒时间排序。
- [✓] 【完成】 `idx_tool_board_tasks_updated_at`：按更新时间查询。

## 使用方式

- [✓] 【完成】 桌面端 `工具 -> 任务看板` 读取和写入该表。
- [✓] 【完成】 到达提醒时间且未完成的任务会在看板顶部和卡片上高亮。
- [✓] 【完成】 页面每 30 秒刷新一次提醒状态。
- [✓] 【完成】 支持新增、修改、完成/恢复和删除任务。

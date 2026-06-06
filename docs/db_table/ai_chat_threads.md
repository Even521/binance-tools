# ai_chat_threads 表结构

`ai_chat_threads` 保存右侧 AI Chat 面板的历史会话。会话正文以 JSON 结构序列化后写入 `data`，当前默认使用 `zstd` 压缩，读取时兼容旧的 `json` 明文数据。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS ai_chat_threads (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    selected_model_json TEXT NOT NULL,
    data_type TEXT NOT NULL
        CHECK (data_type IN ('json', 'zstd')),
    data BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_ai_chat_threads_updated_at
    ON ai_chat_threads(updated_at DESC);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `TEXT` | 会话 ID，由 UI 创建并作为主键 |
| `title` | `TEXT` | 会话标题，默认从第一条用户消息生成 |
| `selected_model_json` | `TEXT` | 当前会话选中的模型 JSON，例如 `{"provider":"deepseek","model":"deepseek-chat"}` |
| `data_type` | `TEXT` | `zstd` 表示压缩 JSON，`json` 为兼容旧数据 |
| `data` | `BLOB` | 会话数据，包含消息、反馈、失败状态和外部 AI 分析使用的规则快照 |
| `created_at` | `TEXT` | 创建时间 |
| `updated_at` | `TEXT` | 最近更新时间，用于历史列表倒序排序 |

## 功能清单

- [✓] 【完成】 保存当前会话消息和选中模型。
- [✓] 【完成】 历史列表按 `updated_at DESC` 排序。
- [✓] 【完成】 读取历史会话时恢复消息和模型。
- [✓] 【完成】 删除历史会话时从 SQLite 移除对应记录。
- [✓] 【完成】 新数据默认使用 `zstd` 压缩。
- [✓] 【完成】 兼容读取旧 `json` 数据。
- [✓] 【完成】 页面触发的 AI 分析会保存当次使用的规则快照，规则后续修改不会覆盖历史会话。

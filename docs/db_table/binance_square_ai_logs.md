# binance_square_ai_logs 表结构

`binance_square_ai_logs` 保存 AI 分析生成币安广场任务的结果，方便排查为什么没有生成任务。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS binance_square_ai_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL,
    title TEXT,
    message TEXT,
    error_message TEXT,
    created_task_id INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `INTEGER` | 自增日志 ID |
| `status` | `TEXT` | `success`、`skipped`、`failed` |
| `title` | `TEXT` | AI 输出标题，例如 `$RONIN` |
| `message` | `TEXT` | AI 输出正文 |
| `error_message` | `TEXT` | 跳过或失败原因 |
| `created_task_id` | `INTEGER` | 成功生成的 `binance_square_tasks.id` |
| `created_at` | `TEXT` | 日志创建时间 |

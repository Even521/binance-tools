# binance_square_ai_settings 表结构

`binance_square_ai_settings` 保存币安广场任务页面的 AI 分析开关和下次执行时间。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS binance_square_ai_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 0,
    next_run_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `INTEGER` | 固定为 `1`，保证单行配置 |
| `enabled` | `INTEGER` | AI 分析开关，`1` 开启，`0` 关闭 |
| `next_run_at` | `TEXT` | 下次 AI 分析时间，按本地时间判断 |
| `updated_at` | `TEXT` | 更新时间 |

## 执行规则

- 开启后，页面调度器每 30 分钟检查一次。
- 如果 `enabled = 1` 且 `next_run_at <= 当前本地时间`，执行一次 AI 市场分析。
- 成功、跳过或失败后，`next_run_at` 都会推进到当前时间后一小时。

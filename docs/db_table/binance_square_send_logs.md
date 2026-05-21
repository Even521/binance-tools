# binance_square_send_logs 表结构

`binance_square_send_logs` 保存币安广场每次发送尝试的结果。消息发送成功、跳过、失败都会写入本表，用于审计、排查和避免重复发送。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS binance_square_send_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER,
    status TEXT NOT NULL,
    response_code TEXT,
    message_digest TEXT NOT NULL,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    sent_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_binance_square_send_logs_task_id
    ON binance_square_send_logs(task_id);

CREATE INDEX IF NOT EXISTS idx_binance_square_send_logs_status
    ON binance_square_send_logs(status);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `INTEGER` | 自增日志 ID |
| `task_id` | `INTEGER` | 关联的 `binance_square_tasks.id`；手动发送时可为空 |
| `status` | `TEXT` | 发送结果状态，例如 `success`、`skipped`、`failed`、`daily_limit`、`key_expired` |
| `response_code` | `TEXT` | Square API 返回码，例如 `20002`、`220009`、`220004` |
| `message_digest` | `TEXT` | 消息摘要或截断后的正文，用于排查和去重 |
| `error_message` | `TEXT` | 错误信息，成功时为空 |
| `retry_count` | `INTEGER` | 本次发送已重试次数 |
| `sent_at` | `TEXT` | 发送完成时间 |
| `created_at` | `TEXT` | 日志创建时间 |

## 状态约定

| 状态 | 说明 |
| --- | --- |
| `success` | 消息发送成功 |
| `skipped` | 敏感词、重复发布等跳过场景 |
| `failed` | 网络错误超过最大重试次数，或其他不可恢复错误 |
| `daily_limit` | 返回 `220009`，达到每日发帖上限 |
| `key_expired` | 返回 `220004`，Key 已过期 |

## 写入要求

- 发送成功必须写入 `success` 日志。
- 敏感词 `20002`、`20022` 写入 `skipped`。
- 每日限额 `220009` 写入 `daily_limit`。
- Key 过期 `220004` 写入 `key_expired`。
- 网络错误重试耗尽后写入 `failed`，并记录 `retry_count`。

## 使用场景

- 任务页面执行“立即发送”或调度器发送后都会产生日志。
- 选中任务后执行“立即发送”会把 `task_id` 写入日志；未选中任务时 `task_id` 为空。
- 发送消息日志页面读取最近 1000 条记录。
- UI 支持复制选中行、选中列或单元格内容，便于排查失败原因。
- UI 支持删除选中日志，底层执行 `DELETE FROM binance_square_send_logs WHERE id = ?` 真删除，删除前需要二次确认。

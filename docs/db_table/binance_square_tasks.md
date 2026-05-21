# binance_square_tasks 表结构

`binance_square_tasks` 保存币安广场待发送、已发送或失败的消息任务。当前任务不再按固定间隔循环发送，而是按 `scheduled_at` 预计发送时间执行一次。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS binance_square_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT,
    name TEXT NOT NULL,
    message TEXT NOT NULL,
    interval_minutes INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_sent_at TEXT,
    scheduled_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    send_status TEXT NOT NULL DEFAULT 'pending',
    source_type TEXT NOT NULL DEFAULT 'manual',
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_binance_square_tasks_scheduled_status
    ON binance_square_tasks(send_status, scheduled_at);

CREATE INDEX IF NOT EXISTS idx_binance_square_tasks_source_title
    ON binance_square_tasks(source_type, title);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `INTEGER` | 自增任务 ID |
| `title` | `TEXT` | 任务标题；AI 市场分析任务使用 `$AAA` 币种代码 |
| `name` | `TEXT` | 任务名称 |
| `message` | `TEXT` | 发送正文 |
| `interval_minutes` | `INTEGER` | 旧字段，兼容历史数据；新调度不再使用 |
| `enabled` | `INTEGER` | 是否启用，`1` 是，`0` 否 |
| `last_sent_at` | `TEXT` | 旧字段，兼容历史数据；新调度只在发送成功或跳过时更新 |
| `scheduled_at` | `TEXT` | 预计发送时间，按本地时间和调度器比较 |
| `send_status` | `TEXT` | `draft`、`pending`、`sending`、`sent`、`failed`、`skipped` |
| `source_type` | `TEXT` | `manual` 或 `ai_market_analysis` |
| `updated_at` | `TEXT` | 更新时间 |

## 到期任务查询

发送执行器每 30 分钟检查一次到期任务：

```sql
SELECT *
FROM binance_square_tasks
WHERE enabled = 1
    AND send_status = 'pending'
    AND datetime(scheduled_at) <= datetime('now', 'localtime')
ORDER BY scheduled_at ASC, id ASC;
```

执行器发送前会先把命中的 `pending` 任务标记为 `sending`，再读取这些 `sending` 任务执行发送，避免页面按钮或调度器重复触发导致同一任务发送两次。发送后按结果更新 `send_status`：

| 状态 | 含义 |
| --- | --- |
| `draft` | AI 分析生成后的草稿任务，只进入任务表，不会被发送调度器直接发送 |
| `pending` | 等待发送 |
| `sending` | 已被执行器领取，正在发送；用于防重复发送 |
| `sent` | 已发送成功 |
| `skipped` | 敏感词、重复发布等跳过 |
| `failed` | 网络错误、Key 过期、每日限额或其他失败 |

## AI 去重

AI 市场分析任务会把输出的 `$AAA` 写入 `title`。生成新任务前会查询当天所有 `source_type = 'ai_market_analysis'` 的标题，并从本次 50 条市场数据里排除这些币种，避免同一天重复发布同一个币种。

AI 生成的任务默认写入 `send_status = 'draft'`，只作为数据入口和人工确认内容，不会被发送执行器直接发布到币安广场。

任务页面支持选中任务后修改 `title`、`name`、`message` 和 `scheduled_at`。点击 `确认发送` 会把任务状态改为 `pending`，之后才会被到期任务查询选中。

选中任务后点击 `立即发送` 会直接发送该任务正文，并按服务端结果把任务状态更新为 `sent`、`skipped` 或 `failed`。删除任务使用 `DELETE FROM binance_square_tasks WHERE id = ?` 真删除，删除后不会保留软删除标记。

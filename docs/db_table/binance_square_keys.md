# binance_square_keys 表结构

`binance_square_keys` 保存币安广场发送消息所需的 API Key。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS binance_square_keys (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    api_key TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `INTEGER` | 固定为 `1`，保证单行配置 |
| `api_key` | `TEXT` | 币安广场 API Key |
| `updated_at` | `TEXT` | 更新时间 |

当前为本地明文保存。后续如果进入真实生产环境，应增加密钥加密，或改为系统凭据存储。

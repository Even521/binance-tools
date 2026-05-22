# ai_provider_keys 表结构

`ai_provider_keys` 保存 AI Provider 的本地密钥。运行时 SQLite 数据库不提交到版本控制，因此真实 API Key 不再写入 `config/ai.json`。

## 建表 SQL

```sql
CREATE TABLE IF NOT EXISTS ai_provider_keys (
    provider_id TEXT PRIMARY KEY,
    provider_name TEXT NOT NULL,
    key_source TEXT NOT NULL DEFAULT 'db'
        CHECK (key_source IN ('db', 'env', 'none')),
    api_key TEXT,
    api_key_env TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (
        (key_source = 'db' AND api_key IS NOT NULL AND length(trim(api_key)) > 0)
        OR
        (key_source = 'env' AND api_key_env IS NOT NULL AND length(trim(api_key_env)) > 0)
        OR
        (key_source = 'none' AND api_key IS NULL AND api_key_env IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_ai_provider_keys_enabled
    ON ai_provider_keys(enabled);
```

## 字段说明

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `provider_id` | `TEXT` | Provider 标识，对应 AI 配置里的 provider key |
| `provider_name` | `TEXT` | Provider 显示名 |
| `key_source` | `TEXT` | `db` 使用本表 `api_key`，`env` 使用环境变量，`none` 表示无需密钥 |
| `api_key` | `TEXT` | 本地保存的真实 API Key |
| `api_key_env` | `TEXT` | 环境变量名 |
| `enabled` | `INTEGER` | 是否启用该密钥记录 |
| `last_used_at` | `TEXT` | 最近一次通过 DB key 发起请求的时间 |
| `created_at` | `TEXT` | 创建时间 |
| `updated_at` | `TEXT` | 更新时间 |

## 功能清单

- [x] 新增或编辑 Provider 时，真实 API Key 保存到本表。
- [x] `config/ai.json` 不再保存真实 API Key。
- [x] 读取 Provider 时优先合并本表保存的 DB key。
- [x] 使用 DB key 发起请求后更新 `last_used_at`。
- [x] `Clear Key` 将记录更新为 `key_source = 'none'`，保留 Provider 配置。
- [x] `Delete` 删除 Provider 配置时，同步删除本表对应 key 记录。
- [x] Provider 列表只有 DB key 存在且非空时显示绿色勾。

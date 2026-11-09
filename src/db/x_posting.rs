use anyhow::{Context, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Persisted switches for the X assistant task scheduler.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XAssistantTaskSettings {
    pub auto_reply_enabled: bool,
    pub auto_post_enabled: bool,
    pub auto_comment_enabled: bool,
    pub own_post_reply_enabled: bool,
    pub posts_per_hour: u8,
    /// Maximum confirmed X replies per Beijing calendar day.
    pub daily_reply_limit: u16,
    /// crypto | entertainment | all
    pub auto_post_news_type: String,
}

impl Default for XAssistantTaskSettings {
    fn default() -> Self {
        Self {
            auto_reply_enabled: false,
            auto_post_enabled: false,
            auto_comment_enabled: false,
            own_post_reply_enabled: false,
            posts_per_hour: 3,
            daily_reply_limit: 1000,
            auto_post_news_type: "all".to_string(),
        }
    }
}

/// Local record for one X post. The record is created before submission and
/// only becomes Published after the UI flow has verified the result.
#[derive(Clone, Debug)]
pub struct XPublishedPost {
    pub id: i64,
    pub news_id: Option<String>,
    pub original_hash: String,
    pub content_hash: String,
    pub content: String,
    pub status: String,
    pub x_post_url: Option<String>,
    pub published_at: Option<String>,
    pub last_error: Option<String>,
    pub comment_status: String,
    pub comment_text: Option<String>,
    pub commented_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct XAssistantTaskRecord {
    pub task_id: String,
    pub task_type: String,
    pub stage: String,
    pub payload_json: String,
    pub retry_count: u32,
    pub last_error: Option<String>,
    pub updated_at: String,
}


/// Stable FNV-1a 64-bit fingerprint for local deduplication.
/// This is a duplicate-detection key, not a cryptographic integrity hash.
pub fn content_fingerprint(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn ensure_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS x_assistant_task_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            auto_reply_enabled INTEGER NOT NULL DEFAULT 0,
            auto_post_enabled INTEGER NOT NULL DEFAULT 0,
            auto_comment_enabled INTEGER NOT NULL DEFAULT 0,
            own_post_reply_enabled INTEGER NOT NULL DEFAULT 0,
            posts_per_hour INTEGER NOT NULL DEFAULT 3 CHECK (posts_per_hour BETWEEN 1 AND 10),
            auto_post_news_type TEXT NOT NULL DEFAULT 'all',
            daily_reply_limit INTEGER NOT NULL DEFAULT 1000,
            updated_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
        );

        INSERT OR IGNORE INTO x_assistant_task_settings (id) VALUES (1);

        CREATE TABLE IF NOT EXISTS x_published_posts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            news_id TEXT,
            source_url TEXT,
            original_hash TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            content TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'submitting', 'published', 'unknown', 'failed')),
            x_post_url TEXT,
            published_at TEXT,
            last_error TEXT,
            comment_status TEXT NOT NULL DEFAULT 'pending'
                CHECK (comment_status IN ('pending', 'commenting', 'completed', 'unknown', 'failed')),
            comment_text TEXT,
            commented_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
        );

        CREATE INDEX IF NOT EXISTS idx_x_published_posts_status_time
            ON x_published_posts(status, published_at DESC);

        CREATE INDEX IF NOT EXISTS idx_x_published_posts_news_id
            ON x_published_posts(news_id);

        CREATE INDEX IF NOT EXISTS idx_x_published_posts_original_hash
            ON x_published_posts(original_hash);

        CREATE INDEX IF NOT EXISTS idx_x_published_posts_content_hash
            ON x_published_posts(content_hash);

        CREATE TABLE IF NOT EXISTS x_assistant_tasks (
            task_id TEXT PRIMARY KEY,
            task_type TEXT NOT NULL,
            stage TEXT NOT NULL,
            payload_json TEXT NOT NULL DEFAULT '{}',
            retry_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
        );

        CREATE INDEX IF NOT EXISTS idx_x_assistant_tasks_stage_updated
            ON x_assistant_tasks(stage, updated_at);
        "#,
    )
    .context("initialize X assistant task persistence schema")?;

    // Upgrade databases created before the news-type preference existed.
    let has_news_type = {
        let mut statement = connection.prepare("PRAGMA table_info(x_assistant_task_settings)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        columns.collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "auto_post_news_type")
    };
    if !has_news_type {
        connection.execute(
            "ALTER TABLE x_assistant_task_settings
             ADD COLUMN auto_post_news_type TEXT NOT NULL DEFAULT 'all'",
            [],
        )?;
    }
    let has_daily_reply_limit = {
        let mut statement = connection.prepare("PRAGMA table_info(x_assistant_task_settings)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        columns.collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "daily_reply_limit")
    };
    if !has_daily_reply_limit {
        connection.execute(
            "ALTER TABLE x_assistant_task_settings
             ADD COLUMN daily_reply_limit INTEGER NOT NULL DEFAULT 1000",
            [],
        )?;
    }

    // The previous application default was 50. Upgrade untouched defaults to 1000.
    let reply_limit_default_migration_applied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_schema_migrations WHERE version = 'x_daily_reply_limit_default_1000_v1')",
        [],
        |row| row.get(0),
    )?;
    if !reply_limit_default_migration_applied {
        connection.execute(
            "UPDATE x_assistant_task_settings SET daily_reply_limit = 1000 WHERE id = 1 AND daily_reply_limit = 50",
            [],
        )?;
        connection.execute(
            "INSERT INTO app_schema_migrations (version) VALUES ('x_daily_reply_limit_default_1000_v1')",
            [],
        )?;
    }

    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS app_schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
        );
        "#
    )?;
    let migration_applied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_schema_migrations WHERE version = 'x_posting_timestamps_beijing_v1')",
        [],
        |row| row.get(0),
    )?;
    if !migration_applied {
        connection.execute_batch(
            r#"
            UPDATE x_assistant_task_settings
            SET updated_at = datetime(updated_at, '+8 hours')
            WHERE updated_at IS NOT NULL;

            UPDATE x_published_posts
            SET published_at = datetime(published_at, '+8 hours')
            WHERE published_at IS NOT NULL;
            UPDATE x_published_posts
            SET commented_at = datetime(commented_at, '+8 hours')
            WHERE commented_at IS NOT NULL;
            UPDATE x_published_posts
            SET created_at = datetime(created_at, '+8 hours'),
                updated_at = datetime(updated_at, '+8 hours');

            UPDATE x_assistant_tasks
            SET created_at = datetime(created_at, '+8 hours'),
                updated_at = datetime(updated_at, '+8 hours');

            INSERT INTO app_schema_migrations (version)
            VALUES ('x_posting_timestamps_beijing_v1');
            "#
        )?;
    }

    // 既有 SQLite 表的 DEFAULT 不会被 CREATE TABLE IF NOT EXISTS 更新。
    // 旧表可能仍以 UTC 自动生成 created_at，而后续 updated_at/published_at
    // 已由应用显式写入北京时间。两者相差约 8 小时的记录可安全修正。
    let timestamp_fix_applied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_schema_migrations WHERE version = 'x_posting_timestamps_beijing_v2')",
        [],
        |row| row.get(0),
    )?;
    if !timestamp_fix_applied {
        connection.execute_batch(
            r#"
            UPDATE x_published_posts
            SET created_at = datetime(created_at, '+8 hours')
            WHERE created_at IS NOT NULL
              AND (
                    (updated_at IS NOT NULL
                     AND julianday(updated_at) - julianday(created_at)
                         BETWEEN 7.0 / 24.0 AND 9.0 / 24.0)
                    OR
                    (published_at IS NOT NULL
                     AND julianday(published_at) - julianday(created_at)
                         BETWEEN 7.0 / 24.0 AND 9.0 / 24.0)
                    OR
                    (commented_at IS NOT NULL
                     AND julianday(commented_at) - julianday(created_at)
                         BETWEEN 7.0 / 24.0 AND 9.0 / 24.0)
              );

            INSERT INTO app_schema_migrations (version)
            VALUES ('x_posting_timestamps_beijing_v2');
            "#
        )?;
    }
    Ok(())
}

pub fn load_task_settings_blocking() -> anyhow::Result<XAssistantTaskSettings> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    connection
        .query_row(
            "SELECT auto_reply_enabled, auto_post_enabled, auto_comment_enabled,
                    own_post_reply_enabled, posts_per_hour, auto_post_news_type,
                    daily_reply_limit
             FROM x_assistant_task_settings WHERE id = 1",
            [],
            |row| {
                Ok(XAssistantTaskSettings {
                    auto_reply_enabled: row.get::<_, i64>(0)? != 0,
                    auto_post_enabled: row.get::<_, i64>(1)? != 0,
                    auto_comment_enabled: row.get::<_, i64>(2)? != 0,
                    own_post_reply_enabled: row.get::<_, i64>(3)? != 0,
                    posts_per_hour: row.get::<_, i64>(4)?.clamp(1, 10) as u8,
                    auto_post_news_type: match row.get::<_, String>(5)?.as_str() {
                        "crypto" => "crypto".to_string(),
                        "entertainment" => "entertainment".to_string(),
                        _ => "all".to_string(),
                    },
                    daily_reply_limit: row.get::<_, i64>(6)?.clamp(1, 10_000) as u16,
                })
            },
        )
        .map_err(Into::into)
}

pub fn save_task_settings_blocking(settings: &XAssistantTaskSettings) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let posts_per_hour = settings.posts_per_hour.clamp(1, 10);
    let daily_reply_limit = settings.daily_reply_limit.clamp(1, 10_000);
    connection.execute(
        "UPDATE x_assistant_task_settings
         SET auto_reply_enabled = ?1,
             auto_post_enabled = ?2,
             auto_comment_enabled = ?3,
             own_post_reply_enabled = ?4,
             posts_per_hour = ?5,
             auto_post_news_type = ?6,
             daily_reply_limit = ?7,
             updated_at = datetime('now', '+8 hours')
         WHERE id = 1",
        params![
            settings.auto_reply_enabled,
            settings.auto_post_enabled,
            settings.auto_comment_enabled,
            settings.own_post_reply_enabled,
            posts_per_hour,
            match settings.auto_post_news_type.as_str() {
                "crypto" => "crypto",
                "entertainment" => "entertainment",
                _ => "all",
            },
            daily_reply_limit,
        ],
    )?;
    Ok(())
}

/// Checks both the source-news fingerprint and the final post-body fingerprint.
/// Unknown/submitting/published records are treated as duplicates; a failed
/// record may be safely prepared again after the caller verifies no post exists.
pub fn is_post_duplicate_blocking(
    original_hash: &str,
    content_hash: &str,
) -> anyhow::Result<bool> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    Ok(connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM x_published_posts
            WHERE (original_hash = ?1 OR content_hash = ?2)
              AND status IN ('pending', 'submitting', 'published', 'unknown')
        )",
        params![original_hash, content_hash],
        |row| row.get(0),
    )?)
}

/// Creates a durable Pending record before the browser is touched.
/// Reuses an earlier Failed row with the same fingerprint.
pub fn reserve_post_blocking(
    news_id: Option<&str>,
    source_url: Option<&str>,
    original_hash: &str,
    content_hash: &str,
    content: &str,
) -> anyhow::Result<i64> {
    ensure!(!original_hash.trim().is_empty(), "原始新闻指纹不能为空");
    ensure!(!content_hash.trim().is_empty(), "发布正文指纹不能为空");
    ensure!(!content.trim().is_empty(), "发布正文不能为空");

    let mut connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let tx = connection.transaction()?;

    let active_duplicate: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM x_published_posts
            WHERE (original_hash = ?1 OR content_hash = ?2)
              AND status IN ('pending', 'submitting', 'published', 'unknown')
        )",
        params![original_hash, content_hash],
        |row| row.get(0),
    )?;
    ensure!(!active_duplicate, "该新闻或正文已有待处理、发布中或已发布记录，为避免重复发布已拦截");

    let existing: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, status FROM x_published_posts
             WHERE original_hash = ?1 OR content_hash = ?2
             ORDER BY id DESC LIMIT 1",
            params![original_hash, content_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let id = if let Some((id, status)) = existing {
        ensure!(
            status == "failed",
            "该新闻或正文已有本地发布记录（状态：{status}），为避免重复发布已拦截"
        );
        tx.execute(
            "UPDATE x_published_posts
             SET news_id = ?2, source_url = ?3, original_hash = ?4,
                 content_hash = ?5, content = ?6, status = 'pending',
                 x_post_url = NULL, published_at = NULL, last_error = NULL,
                 comment_status = 'pending', comment_text = NULL, commented_at = NULL,
                 updated_at = datetime('now', '+8 hours')
             WHERE id = ?1",
            params![id, news_id, source_url, original_hash, content_hash, content],
        )?;
        id
    } else {
        tx.execute(
            "INSERT INTO x_published_posts (
                news_id, source_url, original_hash, content_hash, content, status,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending',
                       datetime('now', '+8 hours'), datetime('now', '+8 hours'))",
            params![news_id, source_url, original_hash, content_hash, content],
        )?;
        tx.last_insert_rowid()
    };

    tx.commit()?;
    Ok(id)
}

pub fn mark_post_submitting_blocking(id: i64) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let changed = connection.execute(
        "UPDATE x_published_posts
         SET status = 'submitting', updated_at = datetime('now', '+8 hours')
         WHERE id = ?1 AND status = 'pending'",
        [id],
    )?;
    ensure!(changed == 1, "发帖记录 {id} 不是待发布状态，拒绝重复提交");
    Ok(())
}

pub fn mark_post_published_blocking(id: i64, x_post_url: Option<&str>) -> anyhow::Result<()> {
    update_post_status(id, "published", x_post_url, None)
}

pub fn mark_post_unknown_blocking(id: i64, reason: &str) -> anyhow::Result<()> {
    update_post_status(id, "unknown", None, Some(reason))
}

pub fn mark_post_failed_blocking(id: i64, reason: &str) -> anyhow::Result<()> {
    update_post_status(id, "failed", None, Some(reason))
}

fn update_post_status(
    id: i64,
    status: &str,
    x_post_url: Option<&str>,
    last_error: Option<&str>,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let published = status == "published";
    let allowed_previous = match status {
        "published" | "unknown" => "('submitting', 'unknown')",
        "failed" => "('pending', 'submitting', 'unknown')",
        _ => anyhow::bail!("不支持的发帖状态转换：{status}"),
    };
    let sql = format!(
        "UPDATE x_published_posts
         SET status = ?2,
             x_post_url = COALESCE(?3, x_post_url),
             published_at = CASE WHEN ?4 THEN datetime('now', '+8 hours') ELSE published_at END,
             last_error = ?5,
             updated_at = datetime('now', '+8 hours')
         WHERE id = ?1 AND status IN {allowed_previous}"
    );
    let changed = connection.execute(
        &sql,
        params![id, status, x_post_url, published, last_error],
    )?;
    ensure!(changed == 1, "未找到本地 X 发帖记录：{id}");
    Ok(())
}

pub fn list_published_posts_blocking(limit: u32) -> anyhow::Result<Vec<XPublishedPost>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let mut statement = connection.prepare(
        "SELECT id, news_id, original_hash, content_hash, content, status,
                x_post_url, published_at, last_error, comment_status, comment_text,
                commented_at, created_at, updated_at
         FROM x_published_posts
         ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit.clamp(1, 500) as i64], |row| {
        Ok(XPublishedPost {
            id: row.get(0)?,
            news_id: row.get(1)?,
            original_hash: row.get(2)?,
            content_hash: row.get(3)?,
            content: row.get(4)?,
            status: row.get(5)?,
            x_post_url: row.get(6)?,
            published_at: row.get(7)?,
            last_error: row.get(8)?,
            comment_status: row.get(9)?,
            comment_text: row.get(10)?,
            commented_at: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn mark_post_commenting_blocking(id: i64, comment_text: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let changed = connection.execute(
        "UPDATE x_published_posts
         SET comment_status = 'commenting', comment_text = ?2,
             updated_at = datetime('now', '+8 hours')
         WHERE id = ?1 AND status = 'published'
           AND comment_status IN ('pending', 'failed')",
        params![id, comment_text],
    )?;
    ensure!(changed == 1, "发帖记录 {id} 尚未确认发布，或评论任务不能启动");
    Ok(())
}

pub fn mark_post_comment_completed_blocking(id: i64) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let changed = connection.execute(
        "UPDATE x_published_posts
         SET comment_status = 'completed', commented_at = datetime('now', '+8 hours'),
             updated_at = datetime('now', '+8 hours')
         WHERE id = ?1 AND status = 'published' AND comment_status = 'commenting'",
        [id],
    )?;
    ensure!(changed == 1, "发帖记录 {id} 不存在或评论任务未处于执行状态");
    Ok(())
}

pub fn mark_post_comment_failed_blocking(id: i64, reason: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let changed = connection.execute(
        "UPDATE x_published_posts
         SET comment_status = 'failed', last_error = ?2,
             updated_at = datetime('now', '+8 hours')
         WHERE id = ?1 AND status = 'published'
           AND comment_status IN ('commenting', 'failed')",
        params![id, reason],
    )?;
    ensure!(changed == 1, "未找到可更新的已发布帖子评论记录：{id}");
    Ok(())
}

pub fn list_recoverable_posts_blocking() -> anyhow::Result<Vec<XPublishedPost>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let mut statement = connection.prepare(
        "SELECT id, news_id, original_hash, content_hash, content, status,
                x_post_url, published_at, last_error, comment_status, comment_text,
                commented_at, created_at, updated_at
         FROM x_published_posts
         WHERE status IN ('pending', 'submitting', 'unknown')
            OR (status = 'published' AND comment_status IN ('pending', 'commenting', 'unknown'))
         ORDER BY id ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(XPublishedPost {
            id: row.get(0)?,
            news_id: row.get(1)?,
            original_hash: row.get(2)?,
            content_hash: row.get(3)?,
            content: row.get(4)?,
            status: row.get(5)?,
            x_post_url: row.get(6)?,
            published_at: row.get(7)?,
            last_error: row.get(8)?,
            comment_status: row.get(9)?,
            comment_text: row.get(10)?,
            commented_at: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn mark_post_comment_unknown_blocking(id: i64, reason: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let changed = connection.execute(
        "UPDATE x_published_posts
         SET comment_status = 'unknown', last_error = ?2,
             updated_at = datetime('now', '+8 hours')
         WHERE id = ?1 AND status = 'published'
           AND comment_status IN ('commenting', 'unknown')",
        params![id, reason],
    )?;
    ensure!(changed == 1, "未找到已发布的 X 帖子记录：{id}");
    Ok(())
}

pub fn count_posts_last_hour_blocking() -> anyhow::Result<u64> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM x_published_posts
         WHERE status = 'published'
           AND published_at >= datetime('now', '+7 hours')",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

/// Age in seconds of the most recently confirmed published post.
pub fn seconds_since_last_post_blocking() -> anyhow::Result<Option<u64>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let age: Option<i64> = connection.query_row(
        "SELECT CAST(strftime('%s', 'now', '+8 hours') - strftime('%s', MAX(published_at)) AS INTEGER)
         FROM x_published_posts
         WHERE status = 'published' AND published_at IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(age.map(|value| value.max(0) as u64))
}

/// Upserts a resumable task checkpoint. payload_json should contain only
/// task data, never credentials or browser session secrets.
pub fn save_task_checkpoint_blocking(
    task_id: &str,
    task_type: &str,
    stage: &str,
    payload_json: &str,
    retry_count: u32,
    last_error: Option<&str>,
) -> anyhow::Result<()> {
    ensure!(!task_id.trim().is_empty(), "任务 ID 不能为空");
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    connection.execute(
        "INSERT INTO x_assistant_tasks (
            task_id, task_type, stage, payload_json, retry_count, last_error
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(task_id) DO UPDATE SET
            task_type = excluded.task_type,
            stage = excluded.stage,
            payload_json = excluded.payload_json,
            retry_count = excluded.retry_count,
            last_error = excluded.last_error,
            updated_at = datetime('now', '+8 hours')",
        params![task_id, task_type, stage, payload_json, retry_count, last_error],
    )?;
    Ok(())
}

pub fn list_incomplete_tasks_blocking() -> anyhow::Result<Vec<XAssistantTaskRecord>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let mut statement = connection.prepare(
        "SELECT task_id, task_type, stage, payload_json, retry_count, last_error, updated_at
         FROM x_assistant_tasks
         WHERE stage NOT IN ('completed', 'cancelled')
         ORDER BY created_at ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(XAssistantTaskRecord {
            task_id: row.get(0)?,
            task_type: row.get(1)?,
            stage: row.get(2)?,
            payload_json: row.get(3)?,
            retry_count: row.get::<_, i64>(4)?.max(0) as u32,
            last_error: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{ensure_schema, XAssistantTaskSettings};

    #[test]
    fn fingerprints_are_stable_and_whitespace_normalized() {
        assert_eq!(super::content_fingerprint("a  b"), super::content_fingerprint("a b"));
        assert_ne!(super::content_fingerprint("a b"), super::content_fingerprint("a c"));
    }

    #[test]
    fn migrates_existing_settings_table_with_news_type_default() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE x_assistant_task_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                auto_reply_enabled INTEGER NOT NULL DEFAULT 0,
                auto_post_enabled INTEGER NOT NULL DEFAULT 0,
                auto_comment_enabled INTEGER NOT NULL DEFAULT 0,
                own_post_reply_enabled INTEGER NOT NULL DEFAULT 0,
                posts_per_hour INTEGER NOT NULL DEFAULT 3,
                updated_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
            );
            INSERT INTO x_assistant_task_settings (id) VALUES (1);",
        ).unwrap();

        ensure_schema(&connection).unwrap();

        let news_type: String = connection.query_row(
            "SELECT auto_post_news_type FROM x_assistant_task_settings WHERE id = 1",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(news_type, "all");
    }

    #[test]
    fn creates_settings_history_and_recovery_tables() {
        let connection = Connection::open_in_memory().unwrap();
        ensure_schema(&connection).unwrap();

        let settings: (i64, i64) = connection
            .query_row(
                "SELECT auto_post_enabled, posts_per_hour
                 FROM x_assistant_task_settings WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(settings, (0, 3));

        let defaults = XAssistantTaskSettings::default();
        assert!(!defaults.auto_post_enabled);
        assert_eq!(defaults.posts_per_hour, 3);
        assert_eq!(defaults.auto_post_news_type, "all");
    }

    #[test]
    fn stores_post_before_submit_and_prevents_duplicate_reservation() {
        let connection = Connection::open_in_memory().unwrap();
        ensure_schema(&connection).unwrap();
        connection.execute(
            "INSERT INTO x_published_posts (
                original_hash, content_hash, content, status
             ) VALUES ('source-1', 'body-1', '测试正文', 'pending')",
            [],
        ).unwrap();

        let duplicate: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM x_published_posts
                WHERE (original_hash = 'source-1' OR content_hash = 'body-1')
                  AND status IN ('pending', 'submitting', 'published', 'unknown')
             )",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(duplicate);
    }
}

//! Per-platform delivery tasks for prepared news posts.

use anyhow::{ensure, Context};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

pub const PLATFORM_X: &str = "x";
pub const PLATFORM_BINANCE: &str = "binance_square";

pub fn target_mode_includes(mode: &str, platform: &str) -> bool {
    matches!(mode, "all")
        || (mode == "x" && platform == PLATFORM_X)
        || (mode == "binance" && platform == PLATFORM_BINANCE)
}

#[derive(Clone, Debug)]
pub struct NewsSendTask {
    pub id: i64,
    pub post_id: i64,
    pub news_id: String,
    pub news_title: String,
    pub platform: String,
    pub status: String,
    pub content: String,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

static READY_SCHEMAS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn ensure_schema(connection: &Connection) -> anyhow::Result<()> {
    let Some(database_path) = connection.path().filter(|path| *path != ":memory:") else {
        return initialize_schema_with_retry(connection);
    };
    let database_path = PathBuf::from(database_path);
    let database_path = fs::canonicalize(&database_path).unwrap_or(database_path);
    let ready_schemas = READY_SCHEMAS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut ready_schemas = ready_schemas
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if ready_schemas.contains(&database_path) {
        return Ok(());
    }

    initialize_schema_with_retry(connection)?;
    ready_schemas.insert(database_path);
    Ok(())
}

fn initialize_schema_with_retry(connection: &Connection) -> anyhow::Result<()> {
    const RETRIES: usize = 3;
    for attempt in 0..=RETRIES {
        match initialize_schema(connection) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < RETRIES && is_sqlite_busy(&error) => {
                let delay = Duration::from_millis(100 * (1 << attempt));
                app_log::warn!(
                    "[SQLite][news_send_tasks] schema 初始化遇到短暂写锁，第 {}/{} 次重试，等待 {}ms：{:#}",
                    attempt + 1,
                    RETRIES,
                    delay.as_millis(),
                    error
                );
                thread::sleep(delay);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("schema retry loop always returns")
}

fn initialize_schema(connection: &Connection) -> anyhow::Result<()> {
    connection
        .execute_batch(
            r#"
        CREATE TABLE IF NOT EXISTS news_send_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            post_id INTEGER NOT NULL,
            news_id TEXT NOT NULL,
            news_title TEXT NOT NULL,
            platform TEXT NOT NULL CHECK (platform IN ('x', 'binance_square')),
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'sending', 'sent', 'failed', 'skipped', 'unknown')),
            last_error TEXT,
            sent_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours')),
            UNIQUE(post_id, platform)
        );
        CREATE INDEX IF NOT EXISTS idx_news_send_tasks_platform_status
            ON news_send_tasks(platform, status, id);
        CREATE INDEX IF NOT EXISTS idx_news_send_tasks_created
            ON news_send_tasks(created_at DESC);
        CREATE TABLE IF NOT EXISTS news_send_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            target_mode TEXT NOT NULL DEFAULT 'all'
                CHECK (target_mode IN ('all', 'x', 'binance')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
        );
        INSERT OR IGNORE INTO news_send_settings (id, target_mode) VALUES (1, 'all');
        "#,
        )
        .context("initialize per-platform news send task table")?;
    let has_sent_at = {
        let mut statement = connection.prepare("PRAGMA table_info(news_send_tasks)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        columns.iter().any(|column| column == "sent_at")
    };
    if !has_sent_at {
        connection.execute("ALTER TABLE news_send_tasks ADD COLUMN sent_at TEXT", [])?;
    }
    Ok(())
}

fn is_sqlite_busy(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<rusqlite::Error>()
            .is_some_and(|error| match error {
                rusqlite::Error::SqliteFailure(code, _) => matches!(
                    code.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ),
                _ => false,
            })
    })
}

pub fn load_target_mode_blocking() -> anyhow::Result<String> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    Ok(connection.query_row(
        "SELECT target_mode FROM news_send_settings WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

pub fn save_target_mode_blocking(mode: &str) -> anyhow::Result<()> {
    ensure!(
        matches!(mode, "all" | "x" | "binance"),
        "invalid news send target mode: {mode}"
    );
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let transaction_trace = super::TransactionTrace::start("news_send_tasks.save_target_mode");
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE news_send_settings SET target_mode = ?1,
             updated_at = datetime('now', '+8 hours') WHERE id = 1",
        [mode],
    )?;
    transaction.execute(
        "UPDATE news_send_tasks SET status = 'skipped',
             last_error = '发送平台未启用', updated_at = datetime('now', '+8 hours')
         WHERE status = 'pending'
           AND ((?1 = 'x' AND platform = 'binance_square')
             OR (?1 = 'binance' AND platform = 'x'))",
        [mode],
    )?;
    transaction.execute(
        "UPDATE news_send_tasks SET status = 'pending', last_error = NULL,
             updated_at = datetime('now', '+8 hours')
         WHERE status = 'skipped' AND last_error = '发送平台未启用'
           AND ((?1 IN ('all', 'x') AND platform = 'x')
             OR (?1 IN ('all', 'binance') AND platform = 'binance_square'))",
        [mode],
    )?;
    transaction.commit()?;
    transaction_trace.commit();
    Ok(())
}

fn news_title(connection: &Connection, news_id: &str) -> anyhow::Result<String> {
    let Some((table, raw_id)) = news_id.split_once(':') else {
        return Ok(news_id.to_string());
    };
    let Ok(id) = raw_id.parse::<i64>() else {
        return Ok(news_id.to_string());
    };
    let title = match table {
        "crypto_news" => connection
            .query_row(
                "SELECT COALESCE(NULLIF(chinese_title, ''), title) FROM crypto_news WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?,
        "entertainment_news" => connection
            .query_row(
                "SELECT COALESCE(NULLIF(chinese_title, ''), title) FROM entertainment_news WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?,
        _ => None,
    };
    Ok(title.unwrap_or_else(|| news_id.to_string()))
}

fn enqueue_for_post(
    connection: &Connection,
    post_id: i64,
    target_mode: &str,
    binance_key_configured: bool,
) -> anyhow::Result<()> {
    let post_status: Option<String> = connection
        .query_row(
            "SELECT status FROM x_published_posts WHERE id = ?1",
            [post_id],
            |row| row.get(0),
        )
        .optional()?;
    ensure!(post_status.is_some(), "X 发帖记录不存在：{post_id}");
    let news_id: Option<String> = connection
        .query_row(
            "SELECT news_id FROM x_published_posts WHERE id = ?1",
            [post_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let news_id = news_id.unwrap_or_else(|| format!("post:{post_id}"));
    let title = news_title(connection, &news_id)?;
    if post_status.as_deref() == Some("pending") {
        connection.execute(
            "UPDATE news_send_tasks SET status = 'pending', last_error = NULL,
                 updated_at = datetime('now', '+8 hours')
             WHERE post_id = ?1 AND status = 'failed'",
            [post_id],
        )?;
    }
    for platform in [PLATFORM_X, PLATFORM_BINANCE] {
        connection.execute(
            "INSERT OR IGNORE INTO news_send_tasks (post_id, news_id, news_title, platform)
             VALUES (?1, ?2, ?3, ?4)",
            params![post_id, news_id, title, platform],
        )?;
        if target_mode_includes(target_mode, platform)
            && (platform != PLATFORM_BINANCE || binance_key_configured)
        {
            connection.execute(
                "UPDATE news_send_tasks SET status = 'pending', last_error = NULL,
                     updated_at = datetime('now', '+8 hours')
                 WHERE post_id = ?1 AND platform = ?2 AND status = 'skipped'
                   AND last_error IN ('未配置币安广场 API Key，未发送', '发送平台未启用')",
                params![post_id, platform],
            )?;
        } else if !target_mode_includes(target_mode, platform) {
            connection.execute(
                "UPDATE news_send_tasks SET status = 'skipped',
                     last_error = '发送平台未启用', updated_at = datetime('now', '+8 hours')
                 WHERE post_id = ?1 AND platform = ?2 AND status = 'pending'",
                params![post_id, platform],
            )?;
        } else if platform == PLATFORM_BINANCE && !binance_key_configured {
            connection.execute(
                "UPDATE news_send_tasks SET status = 'skipped',
                     last_error = '未配置币安广场 API Key，未发送',
                     updated_at = datetime('now', '+8 hours')
                 WHERE post_id = ?1 AND platform = ?2 AND status = 'pending'",
                params![post_id, platform],
            )?;
        }
    }
    Ok(())
}

/// Recover interrupted sends conservatively. Binance requests with a lost
/// response become unknown and are never retried automatically.
pub fn recover_interrupted_send_tasks_blocking() -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    connection.execute_batch(
        "UPDATE news_send_tasks SET status = 'sent', last_error = NULL,
             sent_at = COALESCE(sent_at, datetime('now', '+8 hours')),
             updated_at = datetime('now', '+8 hours')
         WHERE platform = 'x' AND status = 'sending'
           AND EXISTS (SELECT 1 FROM x_published_posts p
                       WHERE p.id = news_send_tasks.post_id AND p.status = 'published');
         UPDATE news_send_tasks SET status = 'unknown',
             last_error = COALESCE(last_error, '进程退出时 X 提交结果未确认'),
             updated_at = datetime('now', '+8 hours')
         WHERE platform = 'x' AND status = 'sending'
           AND EXISTS (SELECT 1 FROM x_published_posts p
                       WHERE p.id = news_send_tasks.post_id
                         AND p.status IN ('submitting', 'unknown'));
         UPDATE news_send_tasks SET status = 'pending', last_error = NULL,
             updated_at = datetime('now', '+8 hours')
         WHERE platform = 'x' AND status = 'sending'
           AND EXISTS (SELECT 1 FROM x_published_posts p
                       WHERE p.id = news_send_tasks.post_id AND p.status = 'pending');
         UPDATE news_send_tasks SET status = 'unknown',
             last_error = COALESCE(last_error, '进程退出时币安接口结果未确认，避免自动重复发送'),
             updated_at = datetime('now', '+8 hours')
         WHERE platform = 'binance_square' AND status = 'sending';",
    )?;
    Ok(())
}

pub fn ensure_tasks_for_post_blocking(
    post_id: i64,
    target_mode: &str,
    binance_key_configured: bool,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    enqueue_for_post(&connection, post_id, target_mode, binance_key_configured)
}

pub fn ensure_tasks_for_pending_posts_blocking(
    target_mode: &str,
    binance_key_configured: bool,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let mut statement = connection
        .prepare("SELECT id FROM x_published_posts WHERE status = 'pending' ORDER BY id ASC")?;
    let post_ids = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for post_id in post_ids {
        enqueue_for_post(&connection, post_id, target_mode, binance_key_configured)?;
    }
    Ok(())
}

pub fn load_next_pending_post_blocking(mode: &str) -> anyhow::Result<Option<(i64, String)>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    Ok(connection
        .query_row(
            "SELECT p.id, p.content FROM x_published_posts p
             JOIN news_send_tasks t ON t.post_id = p.id
             WHERE t.status = 'pending'
               AND (?1 = 'all'
                 OR (?1 = 'x' AND t.platform = 'x')
                 OR (?1 = 'binance' AND t.platform = 'binance_square'))
             ORDER BY t.id ASC LIMIT 1",
            [mode],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn load_next_pending_by_filter_blocking(
    mode: &str,
    filter: &str,
) -> anyhow::Result<Option<(i64, String)>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    Ok(connection
        .query_row(
            "SELECT p.id, p.content FROM x_published_posts p
         JOIN news_send_tasks t ON t.post_id = p.id
         WHERE p.status = 'pending' AND t.status = 'pending'
           AND (?1 = 'all'
             OR (?1 = 'x' AND t.platform = 'x')
             OR (?1 = 'binance' AND t.platform = 'binance_square'))
           AND (?2 = 'all'
             OR (?2 = 'crypto' AND t.news_id LIKE 'crypto_news:%')
             OR (?2 = 'entertainment' AND t.news_id LIKE 'entertainment_news:%'))
         ORDER BY t.id ASC LIMIT 1",
            params![mode, filter],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}
pub fn count_ready_posts_blocking(mode: &str) -> anyhow::Result<usize> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(DISTINCT p.id) FROM x_published_posts p
         JOIN news_send_tasks t ON t.post_id = p.id
         WHERE p.status IN ('pending', 'published', 'unknown', 'submitting')
           AND ((t.status = 'pending'
                 AND (?1 = 'all'
                   OR (?1 = 'x' AND t.platform = 'x')
                   OR (?1 = 'binance' AND t.platform = 'binance_square')))
             OR (?1 = 'binance' AND t.platform = 'binance_square'
                 AND t.status = 'skipped'
                 AND t.last_error = '未配置币安广场 API Key，未发送'))",
        [mode],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub fn count_sent_posts_last_hour_blocking(mode: &str) -> anyhow::Result<u64> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(DISTINCT post_id) FROM news_send_tasks
         WHERE status IN ('sent', 'unknown', 'sending')
           AND COALESCE(sent_at, updated_at) >= datetime('now', '+7 hours')
           AND (?1 = 'all'
             OR (?1 = 'x' AND platform = 'x')
             OR (?1 = 'binance' AND platform = 'binance_square'))",
        [mode],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

pub fn seconds_since_last_send_blocking(mode: &str) -> anyhow::Result<Option<u64>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let age: Option<i64> = connection.query_row(
        "SELECT CAST((julianday(datetime('now', '+8 hours')) -
                      julianday(MAX(COALESCE(sent_at, updated_at)))) * 86400 AS INTEGER)
         FROM news_send_tasks
         WHERE status IN ('sent', 'unknown', 'sending')
           AND (?1 = 'all'
             OR (?1 = 'x' AND platform = 'x')
             OR (?1 = 'binance' AND platform = 'binance_square'))",
        [mode],
        |row| row.get(0),
    )?;
    Ok(age.map(|value| value.max(0) as u64))
}

pub fn claim_task_blocking(post_id: i64, platform: &str) -> anyhow::Result<bool> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let changed = connection.execute(
        "UPDATE news_send_tasks SET status = 'sending', last_error = NULL,
             updated_at = datetime('now', '+8 hours')
         WHERE post_id = ?1 AND platform = ?2 AND status = 'pending'",
        params![post_id, platform],
    )?;
    Ok(changed == 1)
}

pub fn update_task_status_blocking(
    post_id: i64,
    platform: &str,
    status: &str,
    last_error: Option<&str>,
) -> anyhow::Result<()> {
    ensure!(
        matches!(
            status,
            "pending" | "sending" | "sent" | "failed" | "skipped" | "unknown"
        ),
        "invalid news send task status: {status}"
    );
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    connection.execute(
        "UPDATE news_send_tasks SET status = ?3, last_error = ?4,
             sent_at = CASE WHEN ?3 = 'sent' THEN datetime('now', '+8 hours') ELSE sent_at END,
             updated_at = datetime('now', '+8 hours')
         WHERE post_id = ?1 AND platform = ?2 AND status IN ('pending', 'sending')",
        params![post_id, platform, status, last_error],
    )?;
    Ok(())
}

pub fn list_news_send_tasks_blocking(limit: usize) -> anyhow::Result<Vec<NewsSendTask>> {
    let connection = super::open_default_connection()?;
    ensure_schema(&connection)?;
    let mut statement = connection.prepare(
        "SELECT t.id, t.post_id, t.news_id, t.news_title, t.platform, t.status,
                p.content, t.last_error, t.created_at, t.updated_at
         FROM news_send_tasks t
         JOIN x_published_posts p ON p.id = t.post_id
         ORDER BY t.id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit.clamp(1, 1000) as i64], |row| {
        Ok(NewsSendTask {
            id: row.get(0)?,
            post_id: row.get(1)?,
            news_id: row.get(2)?,
            news_title: row.get(3)?,
            platform: row.get(4)?,
            status: row.get(5)?,
            content: row.get(6)?,
            last_error: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

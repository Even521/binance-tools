//! 币安广场配置与任务 SQLite 持久化。

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};

pub const SQUARE_TASK_STATUS_PENDING: &str = "pending";
pub const SQUARE_TASK_STATUS_DRAFT: &str = "draft";
pub const SQUARE_TASK_STATUS_SENDING: &str = "sending";
pub const SQUARE_TASK_STATUS_SENT: &str = "sent";
pub const SQUARE_TASK_STATUS_FAILED: &str = "failed";
pub const SQUARE_TASK_STATUS_SKIPPED: &str = "skipped";
pub const SQUARE_TASK_SOURCE_MANUAL: &str = "manual";
pub const SQUARE_TASK_SOURCE_AI_MARKET_ANALYSIS: &str = "ai_market_analysis";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinanceSquareKey {
    pub api_key: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinanceSquareTask {
    pub id: i64,
    pub title: Option<String>,
    pub name: String,
    pub message: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub last_sent_at: Option<String>,
    pub scheduled_at: String,
    pub send_status: String,
    pub source_type: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinanceSquareAiSettings {
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBinanceSquareTask {
    pub title: Option<String>,
    pub name: String,
    pub message: String,
    pub scheduled_at: Option<String>,
    pub send_status: String,
    pub source_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinanceSquareSendLog {
    pub id: i64,
    pub task_id: Option<i64>,
    pub status: String,
    pub response_code: Option<String>,
    pub message_digest: String,
    pub error_message: Option<String>,
    pub retry_count: u32,
    pub sent_at: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBinanceSquareSendLog {
    pub task_id: Option<i64>,
    pub status: String,
    pub response_code: Option<String>,
    pub message_digest: String,
    pub error_message: Option<String>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBinanceSquareAiLog {
    pub status: String,
    pub title: Option<String>,
    pub message: Option<String>,
    pub error_message: Option<String>,
    pub created_task_id: Option<i64>,
}

pub fn save_square_api_key_blocking(api_key: String) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    save_square_api_key(&connection, &api_key)
}

pub fn load_square_api_key_blocking() -> anyhow::Result<Option<BinanceSquareKey>> {
    let connection = super::open_default_connection()?;
    load_square_api_key(&connection)
}

pub fn save_square_task_blocking(
    name: String,
    message: String,
    scheduled_at: Option<String>,
) -> anyhow::Result<i64> {
    let connection = super::open_default_connection()?;
    save_square_task(
        &connection,
        &NewBinanceSquareTask {
            title: extract_title(&message),
            name,
            message,
            scheduled_at,
            send_status: SQUARE_TASK_STATUS_PENDING.to_string(),
            source_type: SQUARE_TASK_SOURCE_MANUAL.to_string(),
        },
    )
}

pub fn save_ai_square_task_blocking(
    title: String,
    message: String,
    scheduled_at: Option<String>,
) -> anyhow::Result<i64> {
    let connection = super::open_default_connection()?;
    save_square_task(
        &connection,
        &NewBinanceSquareTask {
            title: Some(title),
            name: "AI 市场分析".to_string(),
            message,
            scheduled_at,
            send_status: SQUARE_TASK_STATUS_DRAFT.to_string(),
            source_type: SQUARE_TASK_SOURCE_AI_MARKET_ANALYSIS.to_string(),
        },
    )
}

pub fn list_square_tasks_blocking() -> anyhow::Result<Vec<BinanceSquareTask>> {
    let connection = super::open_default_connection()?;
    list_square_tasks(&connection)
}

pub fn update_square_task_blocking(
    task_id: i64,
    title: Option<String>,
    name: String,
    message: String,
    scheduled_at: Option<String>,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    update_square_task(
        &connection,
        task_id,
        title,
        &name,
        &message,
        scheduled_at.as_deref(),
    )
}

pub fn delete_square_task_blocking(task_id: i64) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    delete_square_task(&connection, task_id)
}

pub fn confirm_square_task_blocking(
    task_id: i64,
    scheduled_at: Option<String>,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    confirm_square_task(&connection, task_id, scheduled_at.as_deref())
}

pub fn claim_due_square_tasks_blocking() -> anyhow::Result<Vec<BinanceSquareTask>> {
    let mut connection = super::open_default_connection()?;
    claim_due_square_tasks(&mut connection)
}

pub fn record_square_send_log_blocking(log: NewBinanceSquareSendLog) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    record_square_send_log(&connection, &log)
}

pub fn list_square_send_logs_blocking(limit: usize) -> anyhow::Result<Vec<BinanceSquareSendLog>> {
    let connection = super::open_default_connection()?;
    list_square_send_logs(&connection, limit)
}

pub fn delete_square_send_log_blocking(log_id: i64) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    delete_square_send_log(&connection, log_id)
}

pub fn mark_square_task_status_blocking(task_id: i64, status: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    mark_square_task_status(&connection, task_id, status)
}

pub fn load_square_ai_settings_blocking() -> anyhow::Result<BinanceSquareAiSettings> {
    let connection = super::open_default_connection()?;
    load_square_ai_settings(&connection)
}

pub fn save_square_ai_settings_blocking(enabled: bool) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    save_square_ai_settings(&connection, enabled)
}

pub fn mark_square_ai_next_run_blocking(next_run_expression: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    mark_square_ai_next_run(&connection, next_run_expression)
}

pub fn square_ai_generation_due_blocking() -> anyhow::Result<bool> {
    let connection = super::open_default_connection()?;
    square_ai_generation_due(&connection)
}

pub fn list_today_ai_titles_blocking() -> anyhow::Result<Vec<String>> {
    let connection = super::open_default_connection()?;
    list_today_ai_titles(&connection)
}

pub fn ai_title_exists_today_blocking(title: &str) -> anyhow::Result<bool> {
    let connection = super::open_default_connection()?;
    ai_title_exists_today(&connection, title)
}

pub fn record_square_ai_log_blocking(log: NewBinanceSquareAiLog) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    record_square_ai_log(&connection, &log)
}

pub fn save_square_api_key(connection: &Connection, api_key: &str) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            INSERT INTO binance_square_keys (id, api_key, updated_at)
            VALUES (1, ?1, CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO UPDATE SET
                api_key = excluded.api_key,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![api_key],
        )
        .context("save Binance Square API key failed")?;

    Ok(())
}

pub fn load_square_api_key(connection: &Connection) -> anyhow::Result<Option<BinanceSquareKey>> {
    connection
        .query_row(
            "SELECT api_key, updated_at FROM binance_square_keys WHERE id = 1",
            [],
            |row| {
                Ok(BinanceSquareKey {
                    api_key: row.get(0)?,
                    updated_at: row.get(1)?,
                })
            },
        )
        .optional()
        .context("load Binance Square API key failed")
}

pub fn save_square_task(
    connection: &Connection,
    task: &NewBinanceSquareTask,
) -> anyhow::Result<i64> {
    connection
        .execute(
            r#"
            INSERT INTO binance_square_tasks (
                title,
                name,
                message,
                interval_minutes,
                enabled,
                scheduled_at,
                send_status,
                source_type,
                updated_at
            )
            VALUES (
                ?1, ?2, ?3, 1, 1,
                COALESCE(NULLIF(?4, ''), datetime('now', 'localtime')),
                ?5,
                ?6,
                CURRENT_TIMESTAMP
            )
            "#,
            params![
                task.title,
                task.name,
                task.message,
                task.scheduled_at.as_deref().unwrap_or(""),
                task.send_status,
                task.source_type,
            ],
        )
        .context("save Binance Square task failed")?;

    Ok(connection.last_insert_rowid())
}

pub fn list_square_tasks(connection: &Connection) -> anyhow::Result<Vec<BinanceSquareTask>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                id,
                title,
                name,
                message,
                interval_minutes,
                enabled,
                last_sent_at,
                scheduled_at,
                send_status,
                source_type,
                updated_at
            FROM binance_square_tasks
            ORDER BY id DESC
            "#,
        )
        .context("prepare list Binance Square tasks SQL failed")?;

    statement
        .query_map([], task_from_row)
        .context("query Binance Square tasks failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read Binance Square task rows failed")
}

pub fn update_square_task(
    connection: &Connection,
    task_id: i64,
    title: Option<String>,
    name: &str,
    message: &str,
    scheduled_at: Option<&str>,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            UPDATE binance_square_tasks
            SET title = ?2,
                name = ?3,
                message = ?4,
                scheduled_at = COALESCE(NULLIF(?5, ''), scheduled_at),
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![task_id, title, name, message, scheduled_at.unwrap_or("")],
        )
        .with_context(|| format!("update Binance Square task failed: {task_id}"))?;

    Ok(())
}

pub fn delete_square_task(connection: &Connection, task_id: i64) -> anyhow::Result<()> {
    connection
        .execute(
            "DELETE FROM binance_square_tasks WHERE id = ?1",
            params![task_id],
        )
        .with_context(|| format!("delete Binance Square task failed: {task_id}"))?;

    Ok(())
}

pub fn confirm_square_task(
    connection: &Connection,
    task_id: i64,
    scheduled_at: Option<&str>,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            UPDATE binance_square_tasks
            SET send_status = 'pending',
                scheduled_at = COALESCE(NULLIF(?2, ''), datetime('now', 'localtime')),
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![task_id, scheduled_at.unwrap_or("")],
        )
        .with_context(|| format!("confirm Binance Square task failed: {task_id}"))?;

    Ok(())
}

pub fn claim_due_square_tasks(
    connection: &mut Connection,
) -> anyhow::Result<Vec<BinanceSquareTask>> {
    let transaction = connection
        .transaction()
        .context("begin claim due Binance Square tasks transaction failed")?;
    let task_ids = {
        let mut statement = transaction
            .prepare(
                r#"
                SELECT id
                FROM binance_square_tasks
                WHERE enabled = 1
                    AND send_status = 'pending'
                    AND datetime(scheduled_at) <= datetime('now', 'localtime')
                ORDER BY scheduled_at ASC, id ASC
                "#,
            )
            .context("prepare claim due Binance Square task ids SQL failed")?;

        statement
            .query_map([], |row| row.get::<_, i64>(0))
            .context("query claim due Binance Square task ids failed")?
            .collect::<Result<Vec<_>, _>>()
            .context("read claim due Binance Square task ids failed")?
    };

    for task_id in &task_ids {
        transaction
            .execute(
                r#"
                UPDATE binance_square_tasks
                SET send_status = 'sending',
                    updated_at = datetime('now', 'localtime')
                WHERE id = ?1
                    AND send_status = 'pending'
                "#,
                params![task_id],
            )
            .with_context(|| format!("claim due Binance Square task failed: {task_id}"))?;
    }

    let tasks = if task_ids.is_empty() {
        Vec::new()
    } else {
        let placeholders = std::iter::repeat_n("?", task_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            r#"
            SELECT
                id,
                title,
                name,
                message,
                interval_minutes,
                enabled,
                last_sent_at,
                scheduled_at,
                send_status,
                source_type,
                updated_at
            FROM binance_square_tasks
            WHERE id IN ({placeholders})
                AND send_status = 'sending'
            ORDER BY scheduled_at ASC, id ASC
            "#
        );
        let mut statement = transaction
            .prepare(&sql)
            .context("prepare claimed Binance Square tasks SQL failed")?;
        statement
            .query_map(rusqlite::params_from_iter(task_ids.iter()), task_from_row)
            .context("query claimed Binance Square tasks failed")?
            .collect::<Result<Vec<_>, _>>()
            .context("read claimed Binance Square task rows failed")?
    };

    transaction
        .commit()
        .context("commit claim due Binance Square tasks transaction failed")?;

    Ok(tasks)
}

pub fn list_due_square_tasks(connection: &Connection) -> anyhow::Result<Vec<BinanceSquareTask>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                id,
                title,
                name,
                message,
                interval_minutes,
                enabled,
                last_sent_at,
                scheduled_at,
                send_status,
                source_type,
                updated_at
            FROM binance_square_tasks
            WHERE enabled = 1
                AND send_status = 'pending'
                AND datetime(scheduled_at) <= datetime('now', 'localtime')
            ORDER BY scheduled_at ASC, id ASC
            "#,
        )
        .context("prepare list due Binance Square tasks SQL failed")?;

    statement
        .query_map([], task_from_row)
        .context("query due Binance Square tasks failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read due Binance Square task rows failed")
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BinanceSquareTask> {
    Ok(BinanceSquareTask {
        id: row.get(0)?,
        title: row.get(1)?,
        name: row.get(2)?,
        message: row.get(3)?,
        interval_minutes: row.get::<_, i64>(4)?.max(1) as u32,
        enabled: row.get::<_, i64>(5)? != 0,
        last_sent_at: row.get(6)?,
        scheduled_at: row.get(7)?,
        send_status: row.get(8)?,
        source_type: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

pub fn record_square_send_log(
    connection: &Connection,
    log: &NewBinanceSquareSendLog,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            INSERT INTO binance_square_send_logs (
                task_id,
                status,
                response_code,
                message_digest,
                error_message,
                retry_count,
                sent_at,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            "#,
            params![
                log.task_id,
                log.status,
                log.response_code,
                log.message_digest,
                log.error_message,
                i64::from(log.retry_count),
            ],
        )
        .context("record Binance Square send log failed")?;

    Ok(())
}

pub fn list_square_send_logs(
    connection: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<BinanceSquareSendLog>> {
    let limit = i64::try_from(limit.clamp(1, 1000)).unwrap_or(1000);
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                id,
                task_id,
                status,
                response_code,
                message_digest,
                error_message,
                retry_count,
                sent_at,
                created_at
            FROM binance_square_send_logs
            ORDER BY id DESC
            LIMIT ?1
            "#,
        )
        .context("prepare list Binance Square send logs SQL failed")?;

    statement
        .query_map(params![limit], |row| {
            Ok(BinanceSquareSendLog {
                id: row.get(0)?,
                task_id: row.get(1)?,
                status: row.get(2)?,
                response_code: row.get(3)?,
                message_digest: row.get(4)?,
                error_message: row.get(5)?,
                retry_count: row.get::<_, i64>(6)?.max(0) as u32,
                sent_at: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .context("query Binance Square send logs failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read Binance Square send log rows failed")
}

pub fn delete_square_send_log(connection: &Connection, log_id: i64) -> anyhow::Result<()> {
    connection
        .execute(
            "DELETE FROM binance_square_send_logs WHERE id = ?1",
            params![log_id],
        )
        .with_context(|| format!("delete Binance Square send log failed: {log_id}"))?;

    Ok(())
}

pub fn mark_square_task_status(
    connection: &Connection,
    task_id: i64,
    status: &str,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            UPDATE binance_square_tasks
            SET send_status = ?2,
                last_sent_at = CASE
                    WHEN ?2 IN ('sent', 'skipped') THEN CURRENT_TIMESTAMP
                    ELSE last_sent_at
                END,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![task_id, status],
        )
        .with_context(|| format!("mark Binance Square task status failed: {task_id}"))?;

    Ok(())
}

pub fn load_square_ai_settings(connection: &Connection) -> anyhow::Result<BinanceSquareAiSettings> {
    connection
        .execute(
            r#"
            INSERT INTO binance_square_ai_settings (id, enabled, next_run_at, updated_at)
            VALUES (1, 0, datetime('now', 'localtime'), CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO NOTHING
            "#,
            [],
        )
        .context("initialize Binance Square AI settings failed")?;

    connection
        .query_row(
            r#"
            SELECT enabled, next_run_at, updated_at
            FROM binance_square_ai_settings
            WHERE id = 1
            "#,
            [],
            |row| {
                Ok(BinanceSquareAiSettings {
                    enabled: row.get::<_, i64>(0)? != 0,
                    next_run_at: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .context("load Binance Square AI settings failed")
}

pub fn save_square_ai_settings(connection: &Connection, enabled: bool) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            INSERT INTO binance_square_ai_settings (id, enabled, next_run_at, updated_at)
            VALUES (1, ?1, datetime('now', 'localtime'), CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO UPDATE SET
                enabled = excluded.enabled,
                next_run_at = CASE
                    WHEN excluded.enabled = 1 THEN datetime('now', 'localtime')
                    ELSE binance_square_ai_settings.next_run_at
                END,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![bool_to_i64(enabled)],
        )
        .context("save Binance Square AI settings failed")?;

    Ok(())
}

pub fn mark_square_ai_next_run(
    connection: &Connection,
    next_run_expression: &str,
) -> anyhow::Result<()> {
    let sql = format!(
        r#"
        INSERT INTO binance_square_ai_settings (id, enabled, next_run_at, updated_at)
        VALUES (1, 1, datetime('now', 'localtime', {next_run_expression}), CURRENT_TIMESTAMP)
        ON CONFLICT(id) DO UPDATE SET
            next_run_at = datetime('now', 'localtime', {next_run_expression}),
            updated_at = CURRENT_TIMESTAMP
        "#
    );
    connection
        .execute(&sql, [])
        .context("mark Binance Square AI next run failed")?;

    Ok(())
}

pub fn square_ai_generation_due(connection: &Connection) -> anyhow::Result<bool> {
    let settings = load_square_ai_settings(connection)?;
    if !settings.enabled {
        return Ok(false);
    }

    connection
        .query_row(
            r#"
            SELECT CASE
                WHEN next_run_at IS NULL THEN 1
                WHEN datetime(next_run_at) <= datetime('now', 'localtime') THEN 1
                ELSE 0
            END
            FROM binance_square_ai_settings
            WHERE id = 1
            "#,
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .context("check Binance Square AI due failed")
}

pub fn list_today_ai_titles(connection: &Connection) -> anyhow::Result<Vec<String>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT DISTINCT title
            FROM (
                SELECT title
                FROM binance_square_tasks
                WHERE source_type = 'ai_market_analysis'
                    AND title IS NOT NULL
                    AND title <> ''
                    AND date(COALESCE(scheduled_at, updated_at), 'localtime') = date('now', 'localtime')
                UNION ALL
                SELECT title
                FROM binance_square_ai_logs
                WHERE title IS NOT NULL
                    AND title <> ''
                    AND date(created_at, 'localtime') = date('now', 'localtime')
            )
            ORDER BY title
            "#,
        )
        .context("prepare list today AI titles SQL failed")?;

    statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("query today AI titles failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read today AI title rows failed")
}

pub fn ai_title_exists_today(connection: &Connection, title: &str) -> anyhow::Result<bool> {
    connection
        .query_row(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM binance_square_tasks
                WHERE source_type = 'ai_market_analysis'
                    AND title = ?1
                    AND date(COALESCE(scheduled_at, updated_at), 'localtime') = date('now', 'localtime')
            )
            "#,
            params![title],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .context("check today AI title failed")
}

pub fn record_square_ai_log(
    connection: &Connection,
    log: &NewBinanceSquareAiLog,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            INSERT INTO binance_square_ai_logs (
                status,
                title,
                message,
                error_message,
                created_task_id,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
            "#,
            params![
                log.status,
                log.title,
                log.message,
                log.error_message,
                log.created_task_id,
            ],
        )
        .context("record Binance Square AI log failed")?;

    Ok(())
}

fn extract_title(message: &str) -> Option<String> {
    message
        .split_whitespace()
        .next()
        .filter(|value| value.starts_with('$') && value.len() > 1)
        .map(str::to_string)
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_square_key_and_scheduled_task() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        save_square_api_key(&connection, "key-1").unwrap();
        save_square_api_key(&connection, "key-2").unwrap();
        assert_eq!(
            load_square_api_key(&connection).unwrap().unwrap().api_key,
            "key-2"
        );

        let task_id = save_square_task(
            &connection,
            &NewBinanceSquareTask {
                title: Some("$AI".to_string()),
                name: "任务".to_string(),
                message: "$AI 热度靠前".to_string(),
                scheduled_at: None,
                send_status: SQUARE_TASK_STATUS_PENDING.to_string(),
                source_type: SQUARE_TASK_SOURCE_AI_MARKET_ANALYSIS.to_string(),
            },
        )
        .unwrap();
        let tasks = list_square_tasks(&connection).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title.as_deref(), Some("$AI"));
        assert_eq!(tasks[0].send_status, SQUARE_TASK_STATUS_PENDING);

        record_square_send_log(
            &connection,
            &NewBinanceSquareSendLog {
                task_id: Some(task_id),
                status: "success".to_string(),
                response_code: Some("0".to_string()),
                message_digest: "消息".to_string(),
                error_message: None,
                retry_count: 0,
            },
        )
        .unwrap();
        let logs = list_square_send_logs(&connection, 10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, "success");

        assert_eq!(list_due_square_tasks(&connection).unwrap().len(), 1);
        mark_square_task_status(&connection, task_id, SQUARE_TASK_STATUS_SENT).unwrap();
        assert!(list_due_square_tasks(&connection).unwrap().is_empty());
        assert!(ai_title_exists_today(&connection, "$AI").unwrap());

        update_square_task(
            &connection,
            task_id,
            Some("$AI2".to_string()),
            "修改后",
            "$AI2 内容",
            None,
        )
        .unwrap();
        let task = list_square_tasks(&connection).unwrap().remove(0);
        assert_eq!(task.title.as_deref(), Some("$AI2"));
        assert_eq!(task.name, "修改后");
    }

    #[test]
    fn stores_ai_settings_and_logs() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        assert!(!load_square_ai_settings(&connection).unwrap().enabled);
        save_square_ai_settings(&connection, true).unwrap();
        assert!(load_square_ai_settings(&connection).unwrap().enabled);
        assert!(square_ai_generation_due(&connection).unwrap());

        record_square_ai_log(
            &connection,
            &NewBinanceSquareAiLog {
                status: "success".to_string(),
                title: Some("$AI".to_string()),
                message: Some("$AI 热度靠前".to_string()),
                error_message: None,
                created_task_id: Some(1),
            },
        )
        .unwrap();
    }
}

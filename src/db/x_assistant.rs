use anyhow::ensure;
use rusqlite::{Connection, params};

#[derive(Clone, Copy, Debug, Default)]
pub struct XAssistantReplyStats {
    pub last_hour: u64,
    pub today: u64,
}

#[derive(Clone, Debug)]
pub struct XAssistantReplyRecord {
    pub run_id: String,
    pub author_name: String,
    pub account_handle: String,
    pub posted_at_text: String,
    pub post_text: String,
    pub reply_text: String,
    pub completed_last_step: bool,
    pub created_at: String,
}

pub fn reply_stats_blocking() -> anyhow::Result<XAssistantReplyStats> {
    let connection = super::open_default_connection()?;
    connection
        .query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN completed_at >= datetime('now', '-1 hour')
                          AND completed_at <= datetime('now') THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN date(completed_at, 'localtime') = date('now', 'localtime')
                         THEN 1 ELSE 0 END), 0)
             FROM x_assistant_reply_records
             WHERE completed_last_step = 1 AND completed_at IS NOT NULL",
            [],
            |row| {
                Ok(XAssistantReplyStats {
                    last_hour: row.get::<_, i64>(0)?.max(0) as u64,
                    today: row.get::<_, i64>(1)?.max(0) as u64,
                })
            },
        )
        .map_err(Into::into)
}

pub fn list_reply_records_blocking() -> anyhow::Result<Vec<XAssistantReplyRecord>> {
    let connection = super::open_default_connection()?;
    let mut statement = connection.prepare(
        "SELECT run_id, author_name, account_handle, posted_at_text, post_text,
                reply_text, completed_last_step, created_at
         FROM x_assistant_reply_records
         ORDER BY id DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(XAssistantReplyRecord {
            run_id: row.get(0)?,
            author_name: row.get(1)?,
            account_handle: row.get(2)?,
            posted_at_text: row.get(3)?,
            post_text: row.get(4)?,
            reply_text: row.get(5)?,
            completed_last_step: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn replied_to_account_within_day_blocking(account_handle: &str) -> anyhow::Result<bool> {
    if account_handle.trim().is_empty() {
        return Ok(false);
    }
    let connection = super::open_default_connection()?;
    replied_to_account_within_day(&connection, account_handle.trim())
}

pub fn insert_reply_record_blocking(
    run_id: &str,
    author_name: &str,
    account_handle: &str,
    posted_at_text: &str,
    post_text: &str,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    insert_reply_record(
        &connection,
        run_id,
        author_name,
        account_handle,
        posted_at_text,
        post_text,
    )
}

pub fn save_reply_text_blocking(run_id: &str, reply_text: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    save_reply_text(&connection, run_id, reply_text)
}

pub fn mark_last_step_completed_blocking(run_id: &str) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    mark_completed(&connection, run_id)
}

pub fn seconds_since_last_completed_blocking() -> anyhow::Result<Option<u64>> {
    let connection = super::open_default_connection()?;
    let age: Option<i64> = connection.query_row(
        "SELECT CAST(strftime('%s', 'now') AS INTEGER) -
                CAST(strftime('%s', MAX(completed_at)) AS INTEGER)
         FROM x_assistant_reply_records
         WHERE completed_last_step = 1 AND completed_at IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(age.map(|seconds| seconds.max(0) as u64))
}

fn replied_to_account_within_day(
    connection: &Connection,
    account_handle: &str,
) -> anyhow::Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM x_assistant_reply_records
            WHERE lower(account_handle) = lower(?1)
              AND completed_last_step = 1
              AND completed_at >= datetime('now', '-1 day')
        )",
        [account_handle],
        |row| row.get(0),
    )?)
}

fn insert_reply_record(
    connection: &Connection,
    run_id: &str,
    author_name: &str,
    account_handle: &str,
    posted_at_text: &str,
    post_text: &str,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO x_assistant_reply_records (
            run_id, author_name, account_handle, posted_at_text, post_text
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            run_id,
            author_name,
            account_handle,
            posted_at_text,
            post_text
        ],
    )?;
    let stored_post_text: String = connection.query_row(
        "SELECT post_text FROM x_assistant_reply_records WHERE run_id = ?1",
        [run_id],
        |row| row.get(0),
    )?;
    ensure!(
        stored_post_text == post_text,
        "帖子正文写入 SQLite 后校验不一致：run_id={run_id}，传入 {} 字节/{} 字符，读回 {} 字节/{} 字符",
        post_text.len(),
        post_text.chars().count(),
        stored_post_text.len(),
        stored_post_text.chars().count()
    );
    Ok(())
}

fn save_reply_text(connection: &Connection, run_id: &str, reply_text: &str) -> anyhow::Result<()> {
    let changed = connection.execute(
        "UPDATE x_assistant_reply_records SET reply_text = ?2 WHERE run_id = ?1",
        params![run_id, reply_text],
    )?;
    ensure!(changed == 1, "未找到 X 回复流程记录 {run_id}");
    Ok(())
}

fn mark_completed(connection: &Connection, run_id: &str) -> anyhow::Result<()> {
    let changed = connection.execute(
        "UPDATE x_assistant_reply_records
         SET completed_last_step = 1, completed_at = CURRENT_TIMESTAMP
         WHERE run_id = ?1",
        [run_id],
    )?;
    ensure!(changed == 1, "未找到 X 回复流程记录 {run_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        insert_reply_record, mark_completed, replied_to_account_within_day, save_reply_text,
    };

    #[test]
    fn stores_post_metadata_and_tracks_final_step() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE x_assistant_reply_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL UNIQUE,
                author_name TEXT NOT NULL DEFAULT '',
                account_handle TEXT NOT NULL DEFAULT '',
                posted_at_text TEXT NOT NULL DEFAULT '',
                post_text TEXT NOT NULL,
                reply_text TEXT NOT NULL DEFAULT '',
                completed_last_step INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                completed_at TEXT
            );",
            )
            .unwrap();

        insert_reply_record(&connection, "run-1", "Alice", "@alice", "3分钟", "帖子正文").unwrap();
        save_reply_text(&connection, "run-1", "回复内容").unwrap();
        mark_completed(&connection, "run-1").unwrap();
        assert!(replied_to_account_within_day(&connection, "@alice").unwrap());
        assert!(replied_to_account_within_day(&connection, "@ALICE").unwrap());
        assert!(!replied_to_account_within_day(&connection, "@bob").unwrap());

        let row: (String, String, String, String, bool) = connection.query_row(
            "SELECT author_name, account_handle, posted_at_text, reply_text, completed_last_step
             FROM x_assistant_reply_records WHERE run_id = ?1",
            ["run-1"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).unwrap();
        assert_eq!(
            row,
            (
                "Alice".into(),
                "@alice".into(),
                "3分钟".into(),
                "回复内容".into(),
                true
            )
        );
    }
}

//! 本地工具任务看板持久化。

use anyhow::Context;
use rusqlite::{Connection, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolBoardTask {
    pub id: i64,
    pub title: String,
    pub note: String,
    pub due_at: String,
    pub completed: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub fn create_tool_board_task_blocking(
    title: String,
    note: String,
    due_at: String,
) -> anyhow::Result<i64> {
    let connection = super::open_default_connection()?;
    create_tool_board_task(&connection, &title, &note, &due_at)
}

pub fn list_tool_board_tasks_blocking() -> anyhow::Result<Vec<ToolBoardTask>> {
    let connection = super::open_default_connection()?;
    list_tool_board_tasks(&connection)
}

pub fn set_tool_board_task_completed_blocking(task_id: i64, completed: bool) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    set_tool_board_task_completed(&connection, task_id, completed)
}

pub fn update_tool_board_task_blocking(
    task_id: i64,
    title: String,
    note: String,
    due_at: String,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    update_tool_board_task(&connection, task_id, &title, &note, &due_at)
}

pub fn delete_tool_board_task_blocking(task_id: i64) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    delete_tool_board_task(&connection, task_id)
}

pub fn create_tool_board_task(
    connection: &Connection,
    title: &str,
    note: &str,
    due_at: &str,
) -> anyhow::Result<i64> {
    let title = title.trim();
    let note = note.trim();
    let due_at = due_at.trim();

    anyhow::ensure!(!title.is_empty(), "task title cannot be empty");
    anyhow::ensure!(!due_at.is_empty(), "task due_at cannot be empty");

    connection
        .execute(
            r#"
            INSERT INTO tool_board_tasks (title, note, due_at, completed, updated_at)
            VALUES (?1, ?2, ?3, 0, CURRENT_TIMESTAMP)
            "#,
            params![title, note, due_at],
        )
        .context("create tool board task failed")?;

    Ok(connection.last_insert_rowid())
}

pub fn list_tool_board_tasks(connection: &Connection) -> anyhow::Result<Vec<ToolBoardTask>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT id, title, note, due_at, completed, created_at, updated_at
            FROM tool_board_tasks
            ORDER BY completed ASC, due_at ASC, id DESC
            "#,
        )
        .context("prepare list tool board tasks failed")?;

    let tasks = statement
        .query_map([], |row| {
            Ok(ToolBoardTask {
                id: row.get(0)?,
                title: row.get(1)?,
                note: row.get(2)?,
                due_at: row.get(3)?,
                completed: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .context("query tool board tasks failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read tool board tasks failed")?;

    Ok(tasks)
}

pub fn set_tool_board_task_completed(
    connection: &Connection,
    task_id: i64,
    completed: bool,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            UPDATE tool_board_tasks
            SET completed = ?2,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![task_id, if completed { 1 } else { 0 }],
        )
        .with_context(|| format!("set tool board task completed failed: {task_id}"))?;

    Ok(())
}

pub fn update_tool_board_task(
    connection: &Connection,
    task_id: i64,
    title: &str,
    note: &str,
    due_at: &str,
) -> anyhow::Result<()> {
    let title = title.trim();
    let note = note.trim();
    let due_at = due_at.trim();

    anyhow::ensure!(!title.is_empty(), "task title cannot be empty");
    anyhow::ensure!(!due_at.is_empty(), "task due_at cannot be empty");

    connection
        .execute(
            r#"
            UPDATE tool_board_tasks
            SET title = ?2,
                note = ?3,
                due_at = ?4,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![task_id, title, note, due_at],
        )
        .with_context(|| format!("update tool board task failed: {task_id}"))?;

    Ok(())
}

pub fn delete_tool_board_task(connection: &Connection, task_id: i64) -> anyhow::Result<()> {
    connection
        .execute(
            "DELETE FROM tool_board_tasks WHERE id = ?1",
            params![task_id],
        )
        .with_context(|| format!("delete tool board task failed: {task_id}"))?;

    Ok(())
}

use anyhow::{Context, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

const DATA_TYPE_JSON: &str = "json";
const DATA_TYPE_ZSTD: &str = "zstd";
const COMPRESSION_LEVEL: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiChatThreadMetadata {
    pub id: String,
    pub title: String,
    pub selected_model_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiChatThread {
    pub metadata: AiChatThreadMetadata,
    pub data: Value,
}

pub fn save_ai_chat_thread_blocking(
    id: &str,
    title: &str,
    selected_model_json: &str,
    data: &Value,
) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    save_ai_chat_thread(&connection, id, title, selected_model_json, data)
}

pub fn save_ai_chat_thread(
    connection: &Connection,
    id: &str,
    title: &str,
    selected_model_json: &str,
    data: &Value,
) -> anyhow::Result<()> {
    let id = id.trim();
    let title = normalize_title(title);
    let selected_model_json = selected_model_json.trim();

    if id.is_empty() {
        return Err(anyhow!("AI chat thread id cannot be empty"));
    }
    if selected_model_json.is_empty() {
        return Err(anyhow!("AI chat thread selected model cannot be empty"));
    }

    let compressed = encode_thread_data(data)?;

    connection
        .execute(
            r#"
            INSERT INTO ai_chat_threads (
                id,
                title,
                selected_model_json,
                data_type,
                data,
                created_at,
                updated_at
            )
            VALUES (?1, ?2, ?3, 'zstd', ?4, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                selected_model_json = excluded.selected_model_json,
                data_type = excluded.data_type,
                data = excluded.data,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![id, title, selected_model_json, compressed],
        )
        .with_context(|| format!("save AI chat thread failed: {id}"))?;

    Ok(())
}

pub fn list_ai_chat_threads_blocking() -> anyhow::Result<Vec<AiChatThreadMetadata>> {
    let connection = crate::db::open_default_connection()?;
    list_ai_chat_threads(&connection)
}

pub fn list_ai_chat_threads(connection: &Connection) -> anyhow::Result<Vec<AiChatThreadMetadata>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT id, title, selected_model_json, created_at, updated_at
            FROM ai_chat_threads
            ORDER BY updated_at DESC, created_at DESC
            "#,
        )
        .context("prepare AI chat thread list query failed")?;

    statement
        .query_map([], |row| {
            Ok(AiChatThreadMetadata {
                id: row.get(0)?,
                title: row.get(1)?,
                selected_model_json: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .context("query AI chat thread list failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read AI chat thread list failed")
}

pub fn load_ai_chat_thread_blocking(id: &str) -> anyhow::Result<Option<AiChatThread>> {
    let connection = crate::db::open_default_connection()?;
    load_ai_chat_thread(&connection, id)
}

pub fn load_ai_chat_thread(
    connection: &Connection,
    id: &str,
) -> anyhow::Result<Option<AiChatThread>> {
    connection
        .query_row(
            r#"
            SELECT id, title, selected_model_json, created_at, updated_at, data_type, data
            FROM ai_chat_threads
            WHERE id = ?1
            "#,
            params![id],
            |row| {
                let data_type: String = row.get(5)?;
                let data: Vec<u8> = row.get(6)?;
                let data = decode_thread_data(&data_type, &data).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Blob,
                        err.into(),
                    )
                })?;

                Ok(AiChatThread {
                    metadata: AiChatThreadMetadata {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        selected_model_json: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    },
                    data,
                })
            },
        )
        .optional()
        .with_context(|| format!("load AI chat thread failed: {id}"))
}

pub fn delete_ai_chat_thread_blocking(id: &str) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    delete_ai_chat_thread(&connection, id)
}

pub fn delete_ai_chat_thread(connection: &Connection, id: &str) -> anyhow::Result<()> {
    connection
        .execute("DELETE FROM ai_chat_threads WHERE id = ?1", params![id])
        .with_context(|| format!("delete AI chat thread failed: {id}"))?;
    Ok(())
}

fn normalize_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "Untitled thread".to_string()
    } else {
        title.to_string()
    }
}

fn encode_thread_data(data: &Value) -> anyhow::Result<Vec<u8>> {
    let json = serde_json::to_vec(data).context("serialize AI chat thread data failed")?;
    zstd::encode_all(json.as_slice(), COMPRESSION_LEVEL)
        .context("compress AI chat thread data failed")
}

fn decode_thread_data(data_type: &str, data: &[u8]) -> anyhow::Result<Value> {
    let json = match data_type {
        DATA_TYPE_ZSTD => {
            zstd::decode_all(data).context("decompress AI chat thread data failed")?
        }
        DATA_TYPE_JSON => data.to_vec(),
        value => return Err(anyhow!("unknown AI chat thread data type: {value}")),
    };

    serde_json::from_slice(&json).context("parse AI chat thread data failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;

    #[test]
    fn stores_thread_data_as_zstd() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        let payload = json!({
            "messages": [
                { "role": "user", "content": "hello" },
                { "role": "assistant", "content": "world" }
            ]
        });

        save_ai_chat_thread(
            &connection,
            "thread-1",
            "First thread",
            r#"{"provider":"deepseek","model":"deepseek-chat"}"#,
            &payload,
        )
        .unwrap();

        let (data_type, data): (String, Vec<u8>) = connection
            .query_row(
                "SELECT data_type, data FROM ai_chat_threads WHERE id = ?1",
                params!["thread-1"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(data_type, DATA_TYPE_ZSTD);
        assert_ne!(data, serde_json::to_vec(&payload).unwrap());

        let loaded = load_ai_chat_thread(&connection, "thread-1")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.metadata.title, "First thread");
        assert_eq!(loaded.data, payload);
    }

    #[test]
    fn loads_legacy_json_thread_data() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();
        let payload = json!({ "messages": [{ "role": "user", "content": "legacy" }] });

        connection
            .execute(
                r#"
                INSERT INTO ai_chat_threads (
                    id, title, selected_model_json, data_type, data
                )
                VALUES (?1, ?2, ?3, 'json', ?4)
                "#,
                params![
                    "legacy-thread",
                    "Legacy thread",
                    r#"{"provider":"openai","model":"gpt"}"#,
                    serde_json::to_vec(&payload).unwrap()
                ],
            )
            .unwrap();

        let loaded = load_ai_chat_thread(&connection, "legacy-thread")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.data, payload);
    }
}

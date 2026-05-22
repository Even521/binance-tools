use anyhow::{Context, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProviderKeySource {
    Db,
    Env,
    None,
}

impl AiProviderKeySource {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "db" => Ok(Self::Db),
            "env" => Ok(Self::Env),
            "none" => Ok(Self::None),
            value => Err(anyhow!("unknown AI provider key source: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProviderKey {
    pub provider_id: String,
    pub provider_name: String,
    pub key_source: AiProviderKeySource,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub enabled: bool,
    pub last_used_at: Option<String>,
    pub updated_at: String,
}

pub fn save_ai_provider_api_key_blocking(
    provider_id: &str,
    provider_name: &str,
    api_key: &str,
) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    save_ai_provider_api_key(&connection, provider_id, provider_name, api_key)
}

pub fn save_ai_provider_api_key(
    connection: &Connection,
    provider_id: &str,
    provider_name: &str,
    api_key: &str,
) -> anyhow::Result<()> {
    let provider_id = provider_id.trim();
    let provider_name = provider_name.trim();
    let api_key = api_key.trim();

    if provider_id.is_empty() {
        return Err(anyhow!("AI provider id cannot be empty"));
    }
    if provider_name.is_empty() {
        return Err(anyhow!("AI provider name cannot be empty"));
    }
    if api_key.is_empty() {
        return Err(anyhow!("AI provider API key cannot be empty"));
    }

    connection
        .execute(
            r#"
            INSERT INTO ai_provider_keys (
                provider_id,
                provider_name,
                key_source,
                api_key,
                api_key_env,
                enabled,
                updated_at
            )
            VALUES (?1, ?2, 'db', ?3, NULL, 1, CURRENT_TIMESTAMP)
            ON CONFLICT(provider_id) DO UPDATE SET
                provider_name = excluded.provider_name,
                key_source = 'db',
                api_key = excluded.api_key,
                api_key_env = NULL,
                enabled = 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![provider_id, provider_name, api_key],
        )
        .with_context(|| format!("save AI provider API key failed: {provider_id}"))?;

    Ok(())
}

pub fn save_ai_provider_api_key_env(
    connection: &Connection,
    provider_id: &str,
    provider_name: &str,
    api_key_env: &str,
) -> anyhow::Result<()> {
    let provider_id = provider_id.trim();
    let provider_name = provider_name.trim();
    let api_key_env = api_key_env.trim();

    if provider_id.is_empty() {
        return Err(anyhow!("AI provider id cannot be empty"));
    }
    if provider_name.is_empty() {
        return Err(anyhow!("AI provider name cannot be empty"));
    }
    if api_key_env.is_empty() {
        return Err(anyhow!("AI provider API key env cannot be empty"));
    }

    connection
        .execute(
            r#"
            INSERT INTO ai_provider_keys (
                provider_id,
                provider_name,
                key_source,
                api_key,
                api_key_env,
                enabled,
                updated_at
            )
            VALUES (?1, ?2, 'env', NULL, ?3, 1, CURRENT_TIMESTAMP)
            ON CONFLICT(provider_id) DO UPDATE SET
                provider_name = excluded.provider_name,
                key_source = 'env',
                api_key = NULL,
                api_key_env = excluded.api_key_env,
                enabled = 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![provider_id, provider_name, api_key_env],
        )
        .with_context(|| format!("save AI provider API key env failed: {provider_id}"))?;

    Ok(())
}

pub fn save_ai_provider_key_none_blocking(
    provider_id: &str,
    provider_name: &str,
) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    save_ai_provider_key_none(&connection, provider_id, provider_name)
}

pub fn save_ai_provider_key_none(
    connection: &Connection,
    provider_id: &str,
    provider_name: &str,
) -> anyhow::Result<()> {
    let provider_id = provider_id.trim();
    let provider_name = provider_name.trim();

    if provider_id.is_empty() {
        return Err(anyhow!("AI provider id cannot be empty"));
    }
    if provider_name.is_empty() {
        return Err(anyhow!("AI provider name cannot be empty"));
    }

    connection
        .execute(
            r#"
            INSERT INTO ai_provider_keys (
                provider_id,
                provider_name,
                key_source,
                api_key,
                api_key_env,
                enabled,
                updated_at
            )
            VALUES (?1, ?2, 'none', NULL, NULL, 1, CURRENT_TIMESTAMP)
            ON CONFLICT(provider_id) DO UPDATE SET
                provider_name = excluded.provider_name,
                key_source = 'none',
                api_key = NULL,
                api_key_env = NULL,
                enabled = 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![provider_id, provider_name],
        )
        .with_context(|| format!("clear AI provider API key failed: {provider_id}"))?;

    Ok(())
}

pub fn load_ai_provider_key_blocking(provider_id: &str) -> anyhow::Result<Option<AiProviderKey>> {
    let connection = crate::db::open_default_connection()?;
    load_ai_provider_key(&connection, provider_id)
}

pub fn load_ai_provider_key(
    connection: &Connection,
    provider_id: &str,
) -> anyhow::Result<Option<AiProviderKey>> {
    connection
        .query_row(
            r#"
            SELECT
                provider_id,
                provider_name,
                key_source,
                api_key,
                api_key_env,
                enabled,
                last_used_at,
                updated_at
            FROM ai_provider_keys
            WHERE provider_id = ?1
            "#,
            params![provider_id],
            |row| {
                let key_source: String = row.get(2)?;
                Ok(AiProviderKey {
                    provider_id: row.get(0)?,
                    provider_name: row.get(1)?,
                    key_source: AiProviderKeySource::parse(&key_source).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            err.into(),
                        )
                    })?,
                    api_key: row.get(3)?,
                    api_key_env: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    last_used_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()
        .with_context(|| format!("load AI provider API key failed: {provider_id}"))
}

pub fn delete_ai_provider_key_blocking(provider_id: &str) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    delete_ai_provider_key(&connection, provider_id)
}

pub fn delete_ai_provider_key(connection: &Connection, provider_id: &str) -> anyhow::Result<()> {
    connection
        .execute(
            "DELETE FROM ai_provider_keys WHERE provider_id = ?1",
            params![provider_id],
        )
        .with_context(|| format!("delete AI provider API key failed: {provider_id}"))?;
    Ok(())
}

pub fn touch_ai_provider_key_last_used_blocking(provider_id: &str) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    connection
        .execute(
            r#"
            UPDATE ai_provider_keys
            SET last_used_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE provider_id = ?1
            "#,
            params![provider_id],
        )
        .with_context(|| format!("touch AI provider API key failed: {provider_id}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_replaces_ai_provider_api_key() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        save_ai_provider_api_key(&connection, "openai", "OpenAI", "key-1").unwrap();
        save_ai_provider_api_key(&connection, "openai", "OpenAI", "key-2").unwrap();

        let key = load_ai_provider_key(&connection, "openai")
            .unwrap()
            .unwrap();
        assert_eq!(key.provider_id, "openai");
        assert_eq!(key.provider_name, "OpenAI");
        assert_eq!(key.key_source, AiProviderKeySource::Db);
        assert_eq!(key.api_key.as_deref(), Some("key-2"));
        assert_eq!(key.api_key_env, None);
        assert!(key.enabled);
    }

    #[test]
    fn stores_ai_provider_api_key_env() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        save_ai_provider_api_key_env(&connection, "deepseek", "DeepSeek", "DEEPSEEK_API_KEY")
            .unwrap();

        let key = load_ai_provider_key(&connection, "deepseek")
            .unwrap()
            .unwrap();
        assert_eq!(key.key_source, AiProviderKeySource::Env);
        assert_eq!(key.api_key, None);
        assert_eq!(key.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
    }

    #[test]
    fn stores_ai_provider_key_none() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        save_ai_provider_api_key(&connection, "openai", "OpenAI", "key-1").unwrap();
        save_ai_provider_key_none(&connection, "openai", "OpenAI").unwrap();

        let key = load_ai_provider_key(&connection, "openai")
            .unwrap()
            .unwrap();
        assert_eq!(key.key_source, AiProviderKeySource::None);
        assert_eq!(key.api_key, None);
        assert_eq!(key.api_key_env, None);
        assert!(key.enabled);
    }
}

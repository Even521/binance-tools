//! SQLite 数据库基础设施。

use anyhow::Context;
use rusqlite::Connection;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub mod ai;
pub mod ai_threads;
pub mod market;
pub mod spot;
pub mod square;

pub const DEFAULT_DATABASE_PATH: &str = "db/binance_tools.sqlite";

pub fn default_database_path() -> PathBuf {
    PathBuf::from(DEFAULT_DATABASE_PATH)
}

pub fn open_default_connection() -> anyhow::Result<Connection> {
    open_connection(default_database_path())
}

pub fn open_connection(path: impl AsRef<Path>) -> anyhow::Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create database directory failed: {}", parent.display()))?;
    }

    let connection = Connection::open(path)
        .with_context(|| format!("open SQLite database failed: {}", path.display()))?;
    run_migrations(&connection)?;
    Ok(connection)
}

fn run_migrations(connection: &Connection) -> anyhow::Result<()> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS spot_symbols (
                symbol TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                base_asset TEXT NOT NULL,
                quote_asset TEXT NOT NULL,
                base_asset_precision INTEGER NOT NULL,
                quote_asset_precision INTEGER NOT NULL,
                order_types TEXT NOT NULL,
                spot_trading_allowed INTEGER NOT NULL,
                margin_trading_allowed INTEGER NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_spot_symbols_base_asset
                ON spot_symbols(base_asset);

            CREATE INDEX IF NOT EXISTS idx_spot_symbols_quote_asset
                ON spot_symbols(quote_asset);

            CREATE TABLE IF NOT EXISTS spot_klines (
                symbol TEXT NOT NULL,
                interval TEXT NOT NULL,
                open_time INTEGER NOT NULL,
                open_price REAL NOT NULL,
                high_price REAL NOT NULL,
                low_price REAL NOT NULL,
                close_price REAL NOT NULL,
                volume REAL NOT NULL,
                close_time INTEGER NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (symbol, interval, open_time)
            );

            CREATE INDEX IF NOT EXISTS idx_spot_klines_symbol_interval_time
                ON spot_klines(symbol, interval, open_time);

            CREATE TABLE IF NOT EXISTS binance_market_products_cache (
                symbol TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                base_asset TEXT NOT NULL,
                quote_asset TEXT NOT NULL,
                asset_name TEXT NOT NULL,
                quote_name TEXT NOT NULL,
                open_price REAL,
                high_price REAL,
                low_price REAL,
                last_price REAL,
                volume REAL,
                quote_volume REAL,
                circulating_supply REAL,
                market_cap REAL,
                price_change_percent REAL,
                partition TEXT NOT NULL,
                partition_name TEXT NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]',
                is_etf INTEGER NOT NULL DEFAULT 0,
                is_trading INTEGER NOT NULL DEFAULT 0,
                fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_binance_market_products_quote_asset
                ON binance_market_products_cache(quote_asset);

            CREATE INDEX IF NOT EXISTS idx_binance_market_products_fetched_at
                ON binance_market_products_cache(fetched_at);

            CREATE TABLE IF NOT EXISTS binance_square_keys (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                api_key TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

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

            CREATE TABLE IF NOT EXISTS binance_square_send_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id INTEGER,
                status TEXT NOT NULL,
                response_code TEXT,
                message_digest TEXT NOT NULL,
                error_message TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                sent_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_binance_square_send_logs_task_id
                ON binance_square_send_logs(task_id);

            CREATE INDEX IF NOT EXISTS idx_binance_square_send_logs_status
                ON binance_square_send_logs(status);

            CREATE TABLE IF NOT EXISTS binance_square_ai_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                enabled INTEGER NOT NULL DEFAULT 0,
                next_run_at TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS binance_square_ai_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                status TEXT NOT NULL,
                title TEXT,
                message TEXT,
                error_message TEXT,
                created_task_id INTEGER,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

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

            CREATE TABLE IF NOT EXISTS ai_chat_threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                selected_model_json TEXT NOT NULL,
                data_type TEXT NOT NULL
                    CHECK (data_type IN ('json', 'zstd')),
                data BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_ai_chat_threads_updated_at
                ON ai_chat_threads(updated_at DESC);

            "#,
        )
        .context("run SQLite migrations failed")?;

    add_column_if_missing(connection, "binance_square_tasks", "last_sent_at", "TEXT")?;
    add_column_if_missing(connection, "binance_square_tasks", "title", "TEXT")?;
    add_column_if_missing(connection, "binance_square_tasks", "scheduled_at", "TEXT")?;
    add_column_if_missing(
        connection,
        "binance_square_tasks",
        "send_status",
        "TEXT NOT NULL DEFAULT 'pending'",
    )?;
    add_column_if_missing(
        connection,
        "binance_square_tasks",
        "source_type",
        "TEXT NOT NULL DEFAULT 'manual'",
    )?;

    connection
        .execute(
            r#"
            UPDATE binance_square_tasks
            SET scheduled_at = COALESCE(scheduled_at, datetime('now', 'localtime')),
                send_status = COALESCE(NULLIF(send_status, ''), 'pending'),
                source_type = COALESCE(NULLIF(source_type, ''), 'manual')
            "#,
            [],
        )
        .context("backfill Binance Square task migration columns failed")?;

    connection
        .execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_binance_square_tasks_scheduled_status
                ON binance_square_tasks(send_status, scheduled_at);

            CREATE INDEX IF NOT EXISTS idx_binance_square_tasks_source_title
                ON binance_square_tasks(source_type, title);
            "#,
        )
        .context("create Binance Square task indexes failed")?;

    Ok(())
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    column_definition: &str,
) -> anyhow::Result<()> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("prepare PRAGMA table_info failed: {table}"))?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("query table info failed: {table}"))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("read table info failed: {table}"))?
        .iter()
        .any(|name| name == column);

    if !exists {
        connection
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {column_definition}"),
                [],
            )
            .with_context(|| format!("add missing column failed: {table}.{column}"))?;
    }

    Ok(())
}

//! SQLite 数据库基础设施。

use anyhow::Context;
use rusqlite::Connection;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};

pub mod ai;
pub mod ai_rules;
pub mod ai_threads;
pub mod alpha;
pub mod crypto_news;
pub mod entertainment_news;
pub mod market;
pub mod spot;
pub mod square;
pub mod task_board;
pub mod x_assistant;
pub mod x_posting;

pub const DEFAULT_DATABASE_PATH: &str = "db/binance_tools.sqlite";

// 同一进程内按数据库路径缓存 WAL 初始化及迁移状态，避免每次打开连接都争抢 SQLite 写锁。
static WAL_DATABASES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
static MIGRATED_DATABASES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

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

    // 多个识别/统计任务并发访问时，给短暂写锁冲突留出等待时间。
    connection
        .busy_timeout(Duration::from_secs(10))
        .context("configure SQLite busy timeout failed")?;

    // 每个 :memory: 连接都是独立数据库，不能套用文件数据库的 WAL/迁移缓存。
    if path == Path::new(":memory:") {
        run_migrations(&connection)?;
        return Ok(connection);
    }

    let migration_key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    // journal_mode=WAL 是写入型 PRAGMA；不能在每次打开连接时重复执行，
    // 否则 UI 统计查询和后台回复记录同时开连接时，会额外争抢数据库锁。
    let wal_databases = WAL_DATABASES.get_or_init(|| Mutex::new(HashSet::new()));
    let mut wal_databases = wal_databases
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !wal_databases.contains(&migration_key) {
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .with_context(|| format!("enable SQLite WAL failed: {}", path.display()))?;
        }
        wal_databases.insert(migration_key.clone());
    }
    drop(wal_databases);
    let migrated = MIGRATED_DATABASES.get_or_init(|| Mutex::new(HashSet::new()));
    let mut migrated = migrated
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if !migrated.contains(&migration_key) {
        // 持有进程内迁移锁，确保并发首次打开连接时只会有一个线程执行迁移。
        run_migrations(&connection)?;
        migrated.insert(migration_key);
    }

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

            CREATE TABLE IF NOT EXISTS spot_klines_4h (
                symbol TEXT NOT NULL,
                interval TEXT NOT NULL DEFAULT '4h',
                open_time INTEGER NOT NULL,
                open_price REAL NOT NULL,
                high_price REAL NOT NULL,
                low_price REAL NOT NULL,
                close_price REAL NOT NULL,
                volume REAL NOT NULL,
                close_time INTEGER NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (symbol, open_time)
            );

            CREATE INDEX IF NOT EXISTS idx_spot_klines_4h_symbol_time
                ON spot_klines_4h(symbol, open_time);

            CREATE TABLE IF NOT EXISTS spot_klines_1d (
                symbol TEXT NOT NULL,
                interval TEXT NOT NULL DEFAULT '1d',
                open_time INTEGER NOT NULL,
                open_price REAL NOT NULL,
                high_price REAL NOT NULL,
                low_price REAL NOT NULL,
                close_price REAL NOT NULL,
                volume REAL NOT NULL,
                close_time INTEGER NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (symbol, open_time)
            );

            CREATE INDEX IF NOT EXISTS idx_spot_klines_1d_symbol_time
                ON spot_klines_1d(symbol, open_time);

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

            CREATE TABLE IF NOT EXISTS alpha_tokens (
                alpha_id TEXT PRIMARY KEY,
                token_id TEXT NOT NULL,
                chain_id TEXT NOT NULL,
                chain_name TEXT NOT NULL,
                contract_address TEXT NOT NULL,
                name TEXT NOT NULL,
                symbol TEXT NOT NULL,
                price TEXT,
                percent_change_24h TEXT,
                volume_24h TEXT,
                market_cap TEXT,
                liquidity TEXT,
                listing_cex INTEGER NOT NULL DEFAULT 0,
                cex_coin_name TEXT NOT NULL DEFAULT '',
                stock_state INTEGER NOT NULL DEFAULT 0,
                cex_off_display INTEGER NOT NULL DEFAULT 0,
                hot_tag INTEGER NOT NULL DEFAULT 0,
                trade_decimal INTEGER,
                listing_time INTEGER,
                score INTEGER,
                mul_point INTEGER,
                extra_json TEXT NOT NULL DEFAULT '{}',
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_alpha_tokens_chain
                ON alpha_tokens(chain_name, chain_id);

            CREATE INDEX IF NOT EXISTS idx_alpha_tokens_symbol
                ON alpha_tokens(symbol);

            CREATE INDEX IF NOT EXISTS idx_alpha_tokens_updated_at
                ON alpha_tokens(updated_at);

            CREATE TABLE IF NOT EXISTS alpha_assets (
                asset TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS alpha_symbols (
                symbol TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                base_asset TEXT NOT NULL,
                quote_asset TEXT NOT NULL,
                price_precision INTEGER,
                quantity_precision INTEGER,
                base_asset_precision INTEGER,
                quote_precision INTEGER,
                filters_json TEXT NOT NULL DEFAULT '[]',
                order_types_json TEXT NOT NULL DEFAULT '[]',
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_alpha_symbols_base_asset
                ON alpha_symbols(base_asset);

            CREATE INDEX IF NOT EXISTS idx_alpha_symbols_quote_asset
                ON alpha_symbols(quote_asset);

            CREATE INDEX IF NOT EXISTS idx_alpha_symbols_updated_at
                ON alpha_symbols(updated_at);

            CREATE TABLE IF NOT EXISTS alpha_klines (
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

            CREATE INDEX IF NOT EXISTS idx_alpha_klines_symbol_interval_time
                ON alpha_klines(symbol, interval, open_time);

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

            CREATE TABLE IF NOT EXISTS ai_rules (
                context_key TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                format TEXT NOT NULL DEFAULT 'text'
                    CHECK (format IN ('text', 'markdown')),
                data_type TEXT NOT NULL
                    CHECK (data_type IN ('text', 'zstd')),
                data BLOB NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_ai_rules_enabled_updated_at
                ON ai_rules(enabled, updated_at DESC);

            CREATE TABLE IF NOT EXISTS tool_board_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                note TEXT NOT NULL DEFAULT '',
                due_at TEXT NOT NULL,
                completed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_tool_board_tasks_completed_due
                ON tool_board_tasks(completed, due_at);

            CREATE INDEX IF NOT EXISTS idx_tool_board_tasks_updated_at
                ON tool_board_tasks(updated_at DESC);

            CREATE TABLE IF NOT EXISTS x_assistant_reply_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL UNIQUE,
                author_name TEXT NOT NULL DEFAULT '',
                account_handle TEXT NOT NULL DEFAULT '',
                posted_at_text TEXT NOT NULL DEFAULT '',
                post_text TEXT NOT NULL,
                reply_text TEXT NOT NULL DEFAULT '',
                completed_last_step INTEGER NOT NULL DEFAULT 0
                    CHECK (completed_last_step IN (0, 1)),
                created_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours')),
                completed_at TEXT,
                send_status TEXT NOT NULL DEFAULT 'pending',
                send_attempt_count INTEGER NOT NULL DEFAULT 0,
                send_attempted_at TEXT,
                send_error TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_x_assistant_reply_records_created_at
                ON x_assistant_reply_records(created_at DESC);

            CREATE TABLE IF NOT EXISTS crypto_news (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                title TEXT NOT NULL,
                chinese_title TEXT,
                url TEXT UNIQUE,
                summary TEXT,
                chinese_summary TEXT,
                published_at TEXT,
                collected_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_crypto_news_source
                ON crypto_news(source);

            CREATE INDEX IF NOT EXISTS idx_crypto_news_published_at
                ON crypto_news(published_at DESC);

            CREATE TABLE IF NOT EXISTS entertainment_news (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                title TEXT NOT NULL,
                chinese_title TEXT,
                url TEXT UNIQUE,
                summary TEXT,
                chinese_summary TEXT,
                published_at TEXT,
                collected_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_entertainment_news_source
                ON entertainment_news(source);

            CREATE INDEX IF NOT EXISTS idx_entertainment_news_published_at
                ON entertainment_news(published_at DESC);

            "#,
        )
        .context("run SQLite migrations failed")?;

    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS app_schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now', '+8 hours'))
        );
        "#
    )?;

    let reply_timezone_migration_applied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_schema_migrations WHERE version = 'x_reply_timestamps_beijing_v1')",
        [],
        |row| row.get(0),
    )?;
    if !reply_timezone_migration_applied {
        connection.execute_batch(
            r#"
            UPDATE x_assistant_reply_records
            SET created_at = datetime(created_at, '+8 hours')
            WHERE created_at IS NOT NULL;
            UPDATE x_assistant_reply_records
            SET completed_at = datetime(completed_at, '+8 hours')
            WHERE completed_at IS NOT NULL;
            INSERT INTO app_schema_migrations (version)
            VALUES ('x_reply_timestamps_beijing_v1');
            "#
        )?;
    }

    add_column_if_missing(
        connection,
        "x_assistant_reply_records",
        "send_status",
        "TEXT NOT NULL DEFAULT 'pending'",
    )?;
    add_column_if_missing(
        connection,
        "x_assistant_reply_records",
        "send_attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(connection, "x_assistant_reply_records", "send_attempted_at", "TEXT")?;
    add_column_if_missing(connection, "x_assistant_reply_records", "send_error", "TEXT")?;

    // 老数据库的表结构不会因 CREATE TABLE IF NOT EXISTS 自动更新 DEFAULT。
    // 旧默认值可能仍是 UTC；如果创建时间与后续显式写入的北京时间相差约 8 小时，
    // 只修正创建时间，避免把已经是北京时间的历史记录再次偏移。
    let reply_timestamp_fix_applied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_schema_migrations WHERE version = 'x_reply_timestamps_beijing_v2')",
        [],
        |row| row.get(0),
    )?;
    if !reply_timestamp_fix_applied {
        connection.execute_batch(
            r#"
            UPDATE x_assistant_reply_records
            SET created_at = datetime(created_at, '+8 hours')
            WHERE created_at IS NOT NULL
              AND (
                    (completed_at IS NOT NULL
                     AND julianday(completed_at) - julianday(created_at)
                         BETWEEN 7.0 / 24.0 AND 9.0 / 24.0)
                    OR
                    (send_attempted_at IS NOT NULL
                     AND julianday(send_attempted_at) - julianday(created_at)
                         BETWEEN 7.0 / 24.0 AND 9.0 / 24.0)
              );

            INSERT INTO app_schema_migrations (version)
            VALUES ('x_reply_timestamps_beijing_v2');
            "#
        )?;
    }

    // 上次运行留下的未完成记录无法证明是否已提交，启动时统一转为待人工核验，避免重复发送。
    connection.execute_batch(
        "UPDATE x_assistant_reply_records
         SET send_status = 'sent'
         WHERE completed_last_step = 1 AND send_status = 'pending';
         UPDATE x_assistant_reply_records
         SET send_status = 'uncertain',
             send_error = COALESCE(send_error, '程序上次退出时发送状态尚未核验')
         WHERE completed_last_step = 0 AND send_status IN ('pending', 'submitting');"
    )?;

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
    add_column_if_missing(
        connection,
        "ai_rules",
        "format",
        "TEXT NOT NULL DEFAULT 'text' CHECK (format IN ('text', 'markdown'))",
    )?;

    connection
        .execute(
            r#"
            UPDATE binance_square_tasks
            SET scheduled_at = COALESCE(scheduled_at, datetime('now', '+8 hours')),
                send_status = COALESCE(NULLIF(send_status, ''), 'pending'),
                source_type = COALESCE(NULLIF(source_type, ''), 'manual')
            "#,
            [],
        )
        .context("backfill Binance Square task migration columns failed")?;

    connection
        .execute_batch(
            r#"
            INSERT OR IGNORE INTO spot_klines_4h (
                symbol,
                interval,
                open_time,
                open_price,
                high_price,
                low_price,
                close_price,
                volume,
                close_time,
                updated_at
            )
            SELECT
                symbol,
                interval,
                open_time,
                open_price,
                high_price,
                low_price,
                close_price,
                volume,
                close_time,
                updated_at
            FROM spot_klines
            WHERE interval = '4h';

            INSERT OR IGNORE INTO spot_klines_1d (
                symbol,
                interval,
                open_time,
                open_price,
                high_price,
                low_price,
                close_price,
                volume,
                close_time,
                updated_at
            )
            SELECT
                symbol,
                interval,
                open_time,
                open_price,
                high_price,
                low_price,
                close_price,
                volume,
                close_time,
                updated_at
            FROM spot_klines
            WHERE interval = '1d';

            CREATE INDEX IF NOT EXISTS idx_binance_square_tasks_scheduled_status
                ON binance_square_tasks(send_status, scheduled_at);

            CREATE INDEX IF NOT EXISTS idx_binance_square_tasks_source_title
                ON binance_square_tasks(source_type, title);
            "#,
        )
        .context("create Binance Square task indexes failed")?;

    ai_rules::seed_default_ai_rules(connection)?;

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

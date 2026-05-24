//! Binance Alpha SQLite cache.

use crate::binance::alpha::{
    AlphaAsset, AlphaDailyKline, AlphaDailyMaSignal, AlphaExchangeInfo, AlphaSymbol, AlphaToken,
};
use anyhow::Context;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::{
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DAY_MILLIS: i64 = 86_400_000;
const DAILY_INTERVAL: &str = "1d";

pub fn load_or_fetch_alpha_tokens_blocking() -> anyhow::Result<Vec<AlphaToken>> {
    let mut connection = super::open_default_connection()?;
    let tokens = list_alpha_tokens(&connection)?;
    if !tokens.is_empty() {
        return Ok(tokens);
    }

    let tokens = crate::binance::alpha::fetch_alpha_tokens_blocking()?;
    replace_alpha_tokens(&mut connection, &tokens)?;
    list_alpha_tokens(&connection)
}

pub fn refresh_alpha_tokens_blocking() -> anyhow::Result<Vec<AlphaToken>> {
    let tokens = crate::binance::alpha::fetch_alpha_tokens_blocking()?;
    let mut connection = super::open_default_connection()?;
    replace_alpha_tokens(&mut connection, &tokens)?;
    list_alpha_tokens(&connection)
}

pub fn load_or_fetch_alpha_exchange_info_blocking() -> anyhow::Result<AlphaExchangeInfo> {
    let mut connection = super::open_default_connection()?;
    let info = list_alpha_exchange_info(&connection)?;
    if !info.symbols.is_empty() {
        return Ok(info);
    }

    let info = crate::binance::alpha::fetch_alpha_exchange_info_blocking()?;
    replace_alpha_exchange_info(&mut connection, &info)?;
    list_alpha_exchange_info(&connection)
}

pub fn refresh_alpha_exchange_info_blocking() -> anyhow::Result<AlphaExchangeInfo> {
    let info = crate::binance::alpha::fetch_alpha_exchange_info_blocking()?;
    let mut connection = super::open_default_connection()?;
    replace_alpha_exchange_info(&mut connection, &info)?;
    list_alpha_exchange_info(&connection)
}

pub fn load_cached_alpha_usdt_daily_ma_signals_blocking(
    days: u16,
) -> anyhow::Result<Vec<AlphaDailyMaSignal>> {
    let connection = super::open_default_connection()?;
    list_cached_alpha_usdt_daily_ma_signals(&connection, days)
}

pub fn load_or_fetch_alpha_usdt_daily_ma_signals_blocking(
    days: u16,
) -> anyhow::Result<Vec<AlphaDailyMaSignal>> {
    let days = days.clamp(1, 1500);
    let mut connection = super::open_default_connection()?;
    if list_alpha_symbols(&connection)?.is_empty() {
        let info = crate::binance::alpha::fetch_alpha_exchange_info_blocking()?;
        replace_alpha_exchange_info(&mut connection, &info)?;
    }

    let missing_symbols = list_alpha_usdt_symbols_missing_daily_klines(&connection, days)?;
    for symbol in missing_symbols {
        let klines = crate::binance::alpha::fetch_alpha_daily_klines_blocking(&symbol, days)?;
        upsert_alpha_daily_klines(&mut connection, &klines)?;
        sleep(Duration::from_millis(150));
    }

    list_cached_alpha_usdt_daily_ma_signals(&connection, days)
}

pub fn load_or_fetch_alpha_daily_klines_blocking(
    symbol: String,
    days: u16,
) -> anyhow::Result<Vec<AlphaDailyKline>> {
    let days = days.clamp(1, 1500);
    let mut connection = super::open_default_connection()?;
    let cached = list_alpha_daily_klines(&connection, &symbol, days as usize)?;
    if has_complete_daily_window(
        cached.len(),
        cached.last().map(|kline| kline.open_time),
        days,
    ) {
        return Ok(cached);
    }

    let klines = crate::binance::alpha::fetch_alpha_daily_klines_blocking(&symbol, days)?;
    upsert_alpha_daily_klines(&mut connection, &klines)?;
    list_alpha_daily_klines(&connection, &symbol, days as usize)
}

pub fn list_alpha_daily_klines_blocking(
    symbol: String,
    limit: usize,
) -> anyhow::Result<Vec<AlphaDailyKline>> {
    let connection = super::open_default_connection()?;
    list_alpha_daily_klines(&connection, &symbol, limit)
}

pub fn list_alpha_tokens(connection: &Connection) -> anyhow::Result<Vec<AlphaToken>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                token_id, alpha_id, chain_id, chain_name, contract_address,
                name, symbol, price, percent_change_24h, volume_24h,
                market_cap, liquidity, listing_cex, cex_coin_name,
                stock_state, cex_off_display, hot_tag, trade_decimal,
                listing_time, score, mul_point, extra_json
            FROM alpha_tokens
            ORDER BY alpha_id
            "#,
        )
        .context("prepare list alpha tokens SQL failed")?;

    let tokens = statement
        .query_map([], |row| {
            let extra_json: String = row.get(21)?;
            Ok(AlphaToken {
                token_id: row.get(0)?,
                alpha_id: row.get(1)?,
                chain_id: row.get(2)?,
                chain_name: row.get(3)?,
                contract_address: row.get(4)?,
                name: row.get(5)?,
                symbol: row.get(6)?,
                price: row.get(7)?,
                percent_change_24h: row.get(8)?,
                volume_24h: row.get(9)?,
                market_cap: row.get(10)?,
                liquidity: row.get(11)?,
                listing_cex: row.get::<_, i64>(12)? != 0,
                cex_coin_name: row.get(13)?,
                stock_state: row.get::<_, i64>(14)? != 0,
                cex_off_display: row.get::<_, i64>(15)? != 0,
                hot_tag: row.get::<_, i64>(16)? != 0,
                trade_decimal: row.get(17)?,
                listing_time: row.get(18)?,
                score: row.get(19)?,
                mul_point: row.get(20)?,
                extra: serde_json::from_str(&extra_json)
                    .unwrap_or(Value::Object(Default::default())),
            })
        })
        .context("query alpha tokens failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read alpha token rows failed")?;

    Ok(tokens)
}

pub fn replace_alpha_tokens(
    connection: &mut Connection,
    tokens: &[AlphaToken],
) -> anyhow::Result<()> {
    let transaction = connection
        .transaction()
        .context("begin replace alpha tokens transaction failed")?;
    transaction
        .execute("DELETE FROM alpha_tokens", [])
        .context("clear alpha tokens failed")?;

    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO alpha_tokens (
                    token_id, alpha_id, chain_id, chain_name, contract_address,
                    name, symbol, price, percent_change_24h, volume_24h,
                    market_cap, liquidity, listing_cex, cex_coin_name,
                    stock_state, cex_off_display, hot_tag, trade_decimal,
                    listing_time, score, mul_point, extra_json, updated_at
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, CURRENT_TIMESTAMP
                )
                "#,
            )
            .context("prepare insert alpha token SQL failed")?;

        for token in tokens {
            statement
                .execute(params![
                    token.token_id,
                    token.alpha_id,
                    token.chain_id,
                    token.chain_name,
                    token.contract_address,
                    token.name,
                    token.symbol,
                    token.price,
                    token.percent_change_24h,
                    token.volume_24h,
                    token.market_cap,
                    token.liquidity,
                    bool_to_i64(token.listing_cex),
                    token.cex_coin_name,
                    bool_to_i64(token.stock_state),
                    bool_to_i64(token.cex_off_display),
                    bool_to_i64(token.hot_tag),
                    token.trade_decimal,
                    token.listing_time,
                    token.score,
                    token.mul_point,
                    serde_json::to_string(&token.extra).unwrap_or_else(|_| "{}".to_string()),
                ])
                .with_context(|| format!("insert alpha token failed: {}", token.alpha_id))?;
        }
    }

    transaction
        .commit()
        .context("commit replace alpha tokens transaction failed")
}

pub fn list_alpha_exchange_info(connection: &Connection) -> anyhow::Result<AlphaExchangeInfo> {
    let assets = list_alpha_assets(connection)?;
    let symbols = list_alpha_symbols(connection)?;
    Ok(AlphaExchangeInfo {
        timezone: "UTC".to_string(),
        assets,
        symbols,
        order_types: Value::Null,
    })
}

fn list_alpha_assets(connection: &Connection) -> anyhow::Result<Vec<AlphaAsset>> {
    let mut statement = connection
        .prepare("SELECT asset FROM alpha_assets ORDER BY asset")
        .context("prepare list alpha assets SQL failed")?;
    let assets = statement
        .query_map([], |row| Ok(AlphaAsset { asset: row.get(0)? }))
        .context("query alpha assets failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read alpha asset rows failed")?;
    Ok(assets)
}

fn list_alpha_symbols(connection: &Connection) -> anyhow::Result<Vec<AlphaSymbol>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                symbol, status, base_asset, quote_asset, price_precision,
                quantity_precision, base_asset_precision, quote_precision,
                filters_json, order_types_json
            FROM alpha_symbols
            ORDER BY symbol
            "#,
        )
        .context("prepare list alpha symbols SQL failed")?;

    let symbols = statement
        .query_map([], |row| {
            let filters_json: String = row.get(8)?;
            let order_types_json: String = row.get(9)?;
            Ok(AlphaSymbol {
                symbol: row.get(0)?,
                status: row.get(1)?,
                base_asset: row.get(2)?,
                quote_asset: row.get(3)?,
                price_precision: row.get(4)?,
                quantity_precision: row.get(5)?,
                base_asset_precision: row.get(6)?,
                quote_precision: row.get(7)?,
                filters: serde_json::from_str(&filters_json).unwrap_or_default(),
                order_types: serde_json::from_str(&order_types_json).unwrap_or_default(),
            })
        })
        .context("query alpha symbols failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read alpha symbol rows failed")?;
    Ok(symbols)
}

pub fn replace_alpha_exchange_info(
    connection: &mut Connection,
    info: &AlphaExchangeInfo,
) -> anyhow::Result<()> {
    let transaction = connection
        .transaction()
        .context("begin replace alpha exchange info transaction failed")?;
    transaction
        .execute("DELETE FROM alpha_assets", [])
        .context("clear alpha assets failed")?;
    transaction
        .execute("DELETE FROM alpha_symbols", [])
        .context("clear alpha symbols failed")?;

    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO alpha_assets (asset, updated_at)
                VALUES (?1, CURRENT_TIMESTAMP)
                "#,
            )
            .context("prepare insert alpha asset SQL failed")?;
        for asset in &info.assets {
            statement
                .execute(params![asset.asset])
                .with_context(|| format!("insert alpha asset failed: {}", asset.asset))?;
        }
    }

    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO alpha_symbols (
                    symbol, status, base_asset, quote_asset, price_precision,
                    quantity_precision, base_asset_precision, quote_precision,
                    filters_json, order_types_json, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, CURRENT_TIMESTAMP)
                "#,
            )
            .context("prepare insert alpha symbol SQL failed")?;
        for symbol in &info.symbols {
            statement
                .execute(params![
                    symbol.symbol,
                    symbol.status,
                    symbol.base_asset,
                    symbol.quote_asset,
                    symbol.price_precision,
                    symbol.quantity_precision,
                    symbol.base_asset_precision,
                    symbol.quote_precision,
                    serde_json::to_string(&symbol.filters).unwrap_or_else(|_| "[]".to_string()),
                    serde_json::to_string(&symbol.order_types).unwrap_or_else(|_| "[]".to_string()),
                ])
                .with_context(|| format!("insert alpha symbol failed: {}", symbol.symbol))?;
        }
    }

    transaction
        .commit()
        .context("commit replace alpha exchange info transaction failed")
}

pub fn upsert_alpha_daily_klines(
    connection: &mut Connection,
    klines: &[AlphaDailyKline],
) -> anyhow::Result<()> {
    let transaction = connection
        .transaction()
        .context("begin upsert alpha klines transaction failed")?;

    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO alpha_klines (
                    symbol, interval, open_time, open_price, high_price,
                    low_price, close_price, volume, close_time, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)
                ON CONFLICT(symbol, interval, open_time) DO UPDATE SET
                    open_price = excluded.open_price,
                    high_price = excluded.high_price,
                    low_price = excluded.low_price,
                    close_price = excluded.close_price,
                    volume = excluded.volume,
                    close_time = excluded.close_time,
                    updated_at = CURRENT_TIMESTAMP
                "#,
            )
            .context("prepare upsert alpha kline SQL failed")?;

        for kline in klines {
            statement
                .execute(params![
                    kline.symbol,
                    kline.interval,
                    kline.open_time,
                    kline.open_price,
                    kline.high_price,
                    kline.low_price,
                    kline.close_price,
                    kline.volume,
                    kline.close_time,
                ])
                .with_context(|| format!("upsert alpha kline failed: {}", kline.symbol))?;
        }
    }

    transaction
        .commit()
        .context("commit upsert alpha klines transaction failed")
}

pub fn list_cached_alpha_usdt_daily_ma_signals(
    connection: &Connection,
    days: u16,
) -> anyhow::Result<Vec<AlphaDailyMaSignal>> {
    let days = days.clamp(1, 1500);
    let start_time = daily_window_start_time(days);
    let end_time = today_utc_open_time_millis();
    let mut statement = connection
        .prepare(
            r#"
            WITH kline_window AS (
                SELECT
                    symbol,
                    COUNT(*) AS samples,
                    AVG(close_price) AS average_price,
                    MAX(open_time) AS latest_open_time
                FROM alpha_klines
                WHERE interval = ?1
                    AND open_time BETWEEN ?2 AND ?3
                GROUP BY symbol
                HAVING samples >= ?4
            )
            SELECT
                s.symbol,
                s.base_asset,
                s.quote_asset,
                k.samples,
                k.average_price,
                latest.close_price
            FROM kline_window k
            JOIN alpha_symbols s ON s.symbol = k.symbol
            JOIN alpha_klines latest
                ON latest.symbol = k.symbol
                AND latest.interval = ?1
                AND latest.open_time = k.latest_open_time
            WHERE s.quote_asset = 'USDT'
                AND s.status = 'TRADING'
            ORDER BY ((latest.close_price - k.average_price) / k.average_price) DESC
            "#,
        )
        .context("prepare cached Alpha USDT daily MA signal SQL failed")?;

    let signals = statement
        .query_map(
            params![DAILY_INTERVAL, start_time, end_time, i64::from(days)],
            |row| {
                let current_price: f64 = row.get(5)?;
                let average_price: f64 = row.get(4)?;

                Ok(AlphaDailyMaSignal {
                    symbol: row.get(0)?,
                    base_asset: row.get(1)?,
                    quote_asset: row.get(2)?,
                    days,
                    current_price,
                    average_price,
                    difference_percent: ((current_price - average_price) / average_price) * 100.0,
                    samples: row.get::<_, i64>(3)? as usize,
                })
            },
        )
        .context("query cached Alpha USDT daily MA signals failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read cached Alpha USDT daily MA signal rows failed")?;

    Ok(signals)
}

pub fn list_alpha_usdt_symbols_missing_daily_klines(
    connection: &Connection,
    days: u16,
) -> anyhow::Result<Vec<String>> {
    let days = days.clamp(1, 1500);
    let start_time = daily_window_start_time(days);
    let end_time = today_utc_open_time_millis();
    let mut statement = connection
        .prepare(
            r#"
            SELECT s.symbol
            FROM alpha_symbols s
            LEFT JOIN alpha_klines k
                ON k.symbol = s.symbol
                AND k.interval = ?1
                AND k.open_time BETWEEN ?2 AND ?3
            WHERE s.quote_asset = 'USDT'
                AND s.status = 'TRADING'
            GROUP BY s.symbol
            HAVING COUNT(k.open_time) < ?4
            ORDER BY s.symbol
            "#,
        )
        .context("prepare missing Alpha USDT daily kline SQL failed")?;

    let symbols = statement
        .query_map(
            params![DAILY_INTERVAL, start_time, end_time, i64::from(days)],
            |row| row.get::<_, String>(0),
        )
        .context("query missing Alpha USDT daily kline symbols failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read missing Alpha USDT daily kline symbol rows failed")?;

    Ok(symbols)
}

pub fn list_alpha_daily_klines(
    connection: &Connection,
    symbol: &str,
    limit: usize,
) -> anyhow::Result<Vec<AlphaDailyKline>> {
    let limit = i64::try_from(limit.clamp(1, 1500)).unwrap_or(1500);
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                symbol, interval, open_time, open_price, high_price,
                low_price, close_price, volume, close_time
            FROM alpha_klines
            WHERE symbol = ?1
                AND interval = ?2
            ORDER BY open_time DESC
            LIMIT ?3
            "#,
        )
        .context("prepare list alpha daily klines SQL failed")?;

    let mut klines = statement
        .query_map(params![symbol, DAILY_INTERVAL, limit], |row| {
            Ok(AlphaDailyKline {
                symbol: row.get(0)?,
                interval: row.get(1)?,
                open_time: row.get(2)?,
                open_price: row.get(3)?,
                high_price: row.get(4)?,
                low_price: row.get(5)?,
                close_price: row.get(6)?,
                volume: row.get(7)?,
                close_time: row.get(8)?,
            })
        })
        .context("query alpha daily klines failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read alpha daily kline rows failed")?;

    klines.sort_by(|a, b| a.open_time.cmp(&b.open_time));
    Ok(klines)
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn daily_window_start_time(days: u16) -> i64 {
    today_utc_open_time_millis() - (i64::from(days.clamp(1, 1500)) - 1) * DAY_MILLIS
}

fn today_utc_open_time_millis() -> i64 {
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    now_millis / DAY_MILLIS * DAY_MILLIS
}

fn has_complete_daily_window(samples: usize, latest_open_time: Option<i64>, days: u16) -> bool {
    samples >= days as usize
        && latest_open_time.is_some_and(|time| time >= today_utc_open_time_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binance::alpha::AlphaExchangeInfo;

    #[test]
    fn caches_and_reads_alpha_daily_ma_signals() {
        let mut connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();
        replace_alpha_exchange_info(
            &mut connection,
            &AlphaExchangeInfo {
                timezone: "UTC".to_string(),
                assets: Vec::new(),
                symbols: vec![AlphaSymbol {
                    symbol: "ALPHA_1USDT".to_string(),
                    status: "TRADING".to_string(),
                    base_asset: "ALPHA_1".to_string(),
                    quote_asset: "USDT".to_string(),
                    price_precision: Some(8),
                    quantity_precision: Some(8),
                    base_asset_precision: Some(8),
                    quote_precision: Some(8),
                    filters: Vec::new(),
                    order_types: vec!["LIMIT".to_string()],
                }],
                order_types: Value::Null,
            },
        )
        .unwrap();

        let today = today_utc_open_time_millis();
        upsert_alpha_daily_klines(
            &mut connection,
            &[
                AlphaDailyKline {
                    symbol: "ALPHA_1USDT".to_string(),
                    interval: DAILY_INTERVAL.to_string(),
                    open_time: today - DAY_MILLIS,
                    open_price: 1.0,
                    high_price: 1.2,
                    low_price: 0.9,
                    close_price: 1.0,
                    volume: 10.0,
                    close_time: today - 1,
                },
                AlphaDailyKline {
                    symbol: "ALPHA_1USDT".to_string(),
                    interval: DAILY_INTERVAL.to_string(),
                    open_time: today,
                    open_price: 1.0,
                    high_price: 1.5,
                    low_price: 0.9,
                    close_price: 1.4,
                    volume: 12.0,
                    close_time: today + DAY_MILLIS - 1,
                },
            ],
        )
        .unwrap();

        assert!(
            list_alpha_usdt_symbols_missing_daily_klines(&connection, 2)
                .unwrap()
                .is_empty()
        );

        let signals = list_cached_alpha_usdt_daily_ma_signals(&connection, 2).unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].samples, 2);
        assert_eq!(signals[0].average_price, 1.2);
        assert_eq!(signals[0].current_price, 1.4);

        let klines = list_alpha_daily_klines(&connection, "ALPHA_1USDT", 10).unwrap();
        assert_eq!(klines.len(), 2);
        assert_eq!(klines[0].close_price, 1.0);
        assert_eq!(klines[1].close_price, 1.4);
    }

    #[test]
    fn stale_alpha_daily_window_is_not_complete() {
        let latest = today_utc_open_time_millis() - DAY_MILLIS;

        assert!(!has_complete_daily_window(120, Some(latest), 120));
        assert!(has_complete_daily_window(
            120,
            Some(today_utc_open_time_millis()),
            120
        ));
    }
}

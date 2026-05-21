//! SQLite cache for Binance web market products.
use crate::binance::market::MarketProduct;
use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};
use std::time::Duration;

pub const MARKET_PRODUCTS_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

pub fn load_or_fetch_market_products_blocking() -> anyhow::Result<Vec<MarketProduct>> {
    let mut connection = super::open_default_connection()?;
    if is_market_products_cache_fresh(&connection, MARKET_PRODUCTS_CACHE_TTL)? {
        return list_market_products(&connection);
    }

    let products = crate::binance::market::fetch_market_products_blocking()?;
    replace_market_products(&mut connection, &products)?;
    list_market_products(&connection)
}

pub fn refresh_market_products_blocking() -> anyhow::Result<Vec<MarketProduct>> {
    let products = crate::binance::market::fetch_market_products_blocking()?;
    let mut connection = super::open_default_connection()?;
    replace_market_products(&mut connection, &products)?;
    list_market_products(&connection)
}

pub fn list_market_products(connection: &Connection) -> anyhow::Result<Vec<MarketProduct>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                symbol,
                status,
                base_asset,
                quote_asset,
                asset_name,
                quote_name,
                open_price,
                high_price,
                low_price,
                last_price,
                volume,
                quote_volume,
                circulating_supply,
                market_cap,
                price_change_percent,
                partition,
                partition_name,
                tags_json,
                is_etf,
                is_trading
            FROM binance_market_products_cache
            ORDER BY symbol
            "#,
        )
        .context("prepare list market products SQL failed")?;

    let products = statement
        .query_map([], |row| {
            let tags_json: String = row.get(17)?;
            let tags = serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default();

            Ok(MarketProduct {
                symbol: row.get(0)?,
                status: row.get(1)?,
                base_asset: row.get(2)?,
                quote_asset: row.get(3)?,
                asset_name: row.get(4)?,
                quote_name: row.get(5)?,
                open_price: row.get(6)?,
                high_price: row.get(7)?,
                low_price: row.get(8)?,
                last_price: row.get(9)?,
                volume: row.get(10)?,
                quote_volume: row.get(11)?,
                circulating_supply: row.get(12)?,
                market_cap: row.get(13)?,
                price_change_percent: row.get(14)?,
                partition: row.get(15)?,
                partition_name: row.get(16)?,
                tags,
                is_etf: row.get::<_, i64>(18)? != 0,
                is_trading: row.get::<_, i64>(19)? != 0,
            })
        })
        .context("query market products failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read market product rows failed")?;

    Ok(products)
}

pub fn replace_market_products(
    connection: &mut Connection,
    products: &[MarketProduct],
) -> anyhow::Result<()> {
    let transaction = connection
        .transaction()
        .context("begin replace market products transaction failed")?;
    transaction
        .execute("DELETE FROM binance_market_products_cache", [])
        .context("clear market products cache failed")?;

    {
        let mut statement = transaction
            .prepare(
                r#"
                INSERT INTO binance_market_products_cache (
                    symbol,
                    status,
                    base_asset,
                    quote_asset,
                    asset_name,
                    quote_name,
                    open_price,
                    high_price,
                    low_price,
                    last_price,
                    volume,
                    quote_volume,
                    circulating_supply,
                    market_cap,
                    price_change_percent,
                    partition,
                    partition_name,
                    tags_json,
                    is_etf,
                    is_trading,
                    fetched_at
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    CURRENT_TIMESTAMP
                )
                "#,
            )
            .context("prepare insert market product SQL failed")?;

        for product in products {
            let tags_json = serde_json::to_string(&product.tags).unwrap_or_else(|_| "[]".into());
            statement
                .execute(params![
                    product.symbol,
                    product.status,
                    product.base_asset,
                    product.quote_asset,
                    product.asset_name,
                    product.quote_name,
                    product.open_price,
                    product.high_price,
                    product.low_price,
                    product.last_price,
                    product.volume,
                    product.quote_volume,
                    product.circulating_supply,
                    product.market_cap,
                    product.price_change_percent,
                    product.partition,
                    product.partition_name,
                    tags_json,
                    bool_to_i64(product.is_etf),
                    bool_to_i64(product.is_trading),
                ])
                .with_context(|| format!("insert market product failed: {}", product.symbol))?;
        }
    }

    transaction
        .commit()
        .context("commit replace market products transaction failed")
}

pub fn is_market_products_cache_fresh(
    connection: &Connection,
    max_age: Duration,
) -> anyhow::Result<bool> {
    let age_seconds = connection
        .query_row(
            r#"
            SELECT CAST(strftime('%s', 'now') - strftime('%s', MAX(fetched_at)) AS INTEGER)
            FROM binance_market_products_cache
            "#,
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .context("query market products cache age failed")?
        .flatten();

    Ok(age_seconds
        .map(|age| age >= 0 && age <= max_age.as_secs() as i64)
        .unwrap_or(false))
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(symbol: &str) -> MarketProduct {
        MarketProduct {
            symbol: symbol.to_string(),
            status: "TRADING".to_string(),
            base_asset: symbol.trim_end_matches("USDT").to_string(),
            quote_asset: "USDT".to_string(),
            asset_name: "Asset".to_string(),
            quote_name: "TetherUS".to_string(),
            open_price: Some(1.0),
            high_price: Some(1.2),
            low_price: Some(0.9),
            last_price: Some(1.1),
            volume: Some(10.0),
            quote_volume: Some(11.0),
            circulating_supply: Some(100.0),
            market_cap: Some(110.0),
            price_change_percent: Some(10.0),
            partition: "USDT".to_string(),
            partition_name: "USDT".to_string(),
            tags: vec!["Test".to_string()],
            is_etf: false,
            is_trading: true,
        }
    }

    #[test]
    fn replaces_market_products_in_one_cache_table() {
        let mut connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        replace_market_products(&mut connection, &[product("BTCUSDT")]).unwrap();
        replace_market_products(&mut connection, &[product("ETHUSDT"), product("BNBUSDT")])
            .unwrap();

        let products = list_market_products(&connection).unwrap();
        assert_eq!(products.len(), 2);
        assert!(products.iter().any(|product| product.symbol == "ETHUSDT"));
        assert!(is_market_products_cache_fresh(&connection, MARKET_PRODUCTS_CACHE_TTL).unwrap());
    }
}

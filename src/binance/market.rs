//! Binance Web market product list used by binance.com spot ranking pages.
//!
//! The endpoint below is a public Binance web BAPI rather than the official
//! spot REST API. Keep parsing tolerant because Binance can add fields without
//! notice.
use anyhow::Context;
use serde::Deserialize;
use std::time::Duration;

pub const PRODUCTS_URL: &str =
    "https://www.binance.com/bapi/asset/v2/public/asset-service/product/get-products";

#[derive(Debug, Clone, PartialEq)]
pub struct MarketProduct {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub asset_name: String,
    pub quote_name: String,
    pub open_price: Option<f64>,
    pub high_price: Option<f64>,
    pub low_price: Option<f64>,
    pub last_price: Option<f64>,
    pub volume: Option<f64>,
    pub quote_volume: Option<f64>,
    pub circulating_supply: Option<f64>,
    pub market_cap: Option<f64>,
    pub price_change_percent: Option<f64>,
    pub partition: String,
    pub partition_name: String,
    pub tags: Vec<String>,
    pub is_etf: bool,
    pub is_trading: bool,
}

#[derive(Debug, Deserialize)]
struct ProductsResponse {
    code: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Vec<ProductRow>,
}

#[derive(Debug, Deserialize)]
struct ProductRow {
    #[serde(rename = "s", default)]
    symbol: String,
    #[serde(rename = "st", default)]
    status: String,
    #[serde(rename = "b", default)]
    base_asset: String,
    #[serde(rename = "q", default)]
    quote_asset: String,
    #[serde(rename = "an", default)]
    asset_name: String,
    #[serde(rename = "qn", default)]
    quote_name: String,
    #[serde(rename = "o", default)]
    open_price: Option<String>,
    #[serde(rename = "h", default)]
    high_price: Option<String>,
    #[serde(rename = "l", default)]
    low_price: Option<String>,
    #[serde(rename = "c", default)]
    last_price: Option<String>,
    #[serde(rename = "v", default)]
    volume: Option<String>,
    #[serde(rename = "qv", default)]
    quote_volume: Option<String>,
    #[serde(rename = "cs", default)]
    circulating_supply: Option<f64>,
    #[serde(rename = "pm", default)]
    partition: String,
    #[serde(rename = "pn", default)]
    partition_name: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    etf: bool,
}

impl ProductRow {
    fn into_market_product(self) -> MarketProduct {
        let open_price = parse_number(self.open_price.as_deref());
        let last_price = parse_number(self.last_price.as_deref());
        let market_cap = match (last_price, self.circulating_supply) {
            (Some(price), Some(supply)) if price.is_finite() && supply.is_finite() => {
                Some(price * supply)
            }
            _ => None,
        };
        let price_change_percent = match (open_price, last_price) {
            (Some(open), Some(last)) if open > 0.0 => Some((last - open) / open * 100.0),
            _ => None,
        };

        MarketProduct {
            symbol: self.symbol,
            status: self.status.clone(),
            base_asset: self.base_asset,
            quote_asset: self.quote_asset,
            asset_name: self.asset_name,
            quote_name: self.quote_name,
            open_price,
            high_price: parse_number(self.high_price.as_deref()),
            low_price: parse_number(self.low_price.as_deref()),
            last_price,
            volume: parse_number(self.volume.as_deref()),
            quote_volume: parse_number(self.quote_volume.as_deref()),
            circulating_supply: self.circulating_supply,
            market_cap,
            price_change_percent,
            partition: self.partition,
            partition_name: self.partition_name,
            tags: self.tags,
            is_etf: self.etf,
            is_trading: self.status == "TRADING",
        }
    }
}

pub fn fetch_market_products_blocking() -> anyhow::Result<Vec<MarketProduct>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("binance-tools/0.1")
        .build()
        .context("build Binance products HTTP client failed")?;
    let response = client
        .get(PRODUCTS_URL)
        .send()
        .context("Binance products request failed")?
        .error_for_status()
        .context("Binance products request returned error status")?
        .json::<ProductsResponse>()
        .context("parse Binance products response failed")?;

    if response.code != "000000" {
        anyhow::bail!(
            "Binance products response error: {}",
            response.message.unwrap_or(response.code)
        );
    }

    let mut products = response
        .data
        .into_iter()
        .map(ProductRow::into_market_product)
        .filter(|product| !product.symbol.is_empty())
        .collect::<Vec<_>>();
    products.sort_by(|a, b| a.symbol.cmp(&b.symbol));

    Ok(products)
}

fn parse_number(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_change_percent_and_market_cap() {
        let product = ProductRow {
            symbol: "AIUSDT".to_string(),
            status: "TRADING".to_string(),
            base_asset: "AI".to_string(),
            quote_asset: "USDT".to_string(),
            asset_name: "Sleepless AI".to_string(),
            quote_name: "TetherUS".to_string(),
            open_price: Some("0.0283".to_string()),
            high_price: Some("0.0357".to_string()),
            low_price: Some("0.0278".to_string()),
            last_price: Some("0.0327".to_string()),
            volume: Some("268069283.8000".to_string()),
            quote_volume: Some("8569366.86158".to_string()),
            circulating_supply: Some(261250000.0),
            partition: "USDT".to_string(),
            partition_name: "USDT".to_string(),
            tags: vec!["AI".to_string()],
            etf: false,
        }
        .into_market_product();

        assert_eq!(product.symbol, "AIUSDT");
        assert_eq!(product.price_change_percent.unwrap().round(), 16.0);
        assert_eq!(product.market_cap.unwrap().round(), 8542875.0);
        assert!(product.is_trading);
    }
}

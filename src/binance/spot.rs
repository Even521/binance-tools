//! Binance 现货公开市场数据接口。

use super::{BinanceEnvironment, BinanceSettings, sdk};
use anyhow::Context;
use sdk::{
    config::ConfigurationRestApi,
    spot::{
        SpotRestApi,
        rest_api::{
            ExchangeInfoParams, KlinesIntervalEnum, KlinesItemInner, KlinesParams, RestApi,
            TickerPriceParams, TickerPriceResponse,
        },
    },
};
use std::{collections::HashMap, time::Duration};
use tokio::time::sleep;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotSymbolInfo {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub base_asset_precision: i64,
    pub quote_asset_precision: i64,
    pub order_types: Vec<String>,
    pub spot_trading_allowed: bool,
    pub margin_trading_allowed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyMaSignal {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub days: u16,
    pub current_price: f64,
    pub average_price: f64,
    pub difference_percent: f64,
    pub samples: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpotDailyKline {
    pub symbol: String,
    pub interval: String,
    pub open_time: i64,
    pub open_price: f64,
    pub high_price: f64,
    pub low_price: f64,
    pub close_price: f64,
    pub volume: f64,
    pub close_time: i64,
}

impl SpotSymbolInfo {
    fn from_sdk_symbol(symbol: sdk::spot::rest_api::ExchangeInfoResponseSymbolsInner) -> Self {
        Self {
            symbol: symbol.symbol.unwrap_or_default(),
            status: symbol.status.unwrap_or_default(),
            base_asset: symbol.base_asset.unwrap_or_default(),
            quote_asset: symbol.quote_asset.unwrap_or_default(),
            base_asset_precision: symbol.base_asset_precision.unwrap_or_default(),
            quote_asset_precision: symbol.quote_asset_precision.unwrap_or_default(),
            order_types: symbol.order_types.unwrap_or_default(),
            spot_trading_allowed: symbol.is_spot_trading_allowed.unwrap_or(false),
            margin_trading_allowed: symbol.is_margin_trading_allowed.unwrap_or(false),
        }
    }
}

impl DailyMaSignal {
    fn new(
        symbol: SpotSymbolInfo,
        days: u16,
        current_price: f64,
        average_price: f64,
        samples: usize,
    ) -> Self {
        Self {
            symbol: symbol.symbol,
            base_asset: symbol.base_asset,
            quote_asset: symbol.quote_asset,
            days,
            current_price,
            average_price,
            difference_percent: ((current_price - average_price) / average_price) * 100.0,
            samples,
        }
    }
}

pub async fn fetch_spot_symbols(settings: BinanceSettings) -> anyhow::Result<Vec<SpotSymbolInfo>> {
    let client = spot_rest_client(settings)?;

    fetch_spot_symbols_with_client(&client).await
}

pub async fn fetch_usdt_daily_ma_signals(
    settings: BinanceSettings,
    days: u16,
) -> anyhow::Result<Vec<DailyMaSignal>> {
    let days = days.clamp(1, 1000);
    let client = spot_rest_client(settings)?;
    let symbols = fetch_spot_symbols_with_client(&client).await?;
    let prices = fetch_ticker_prices(&client).await?;
    let mut signals = Vec::new();

    for symbol in symbols.into_iter().filter(|symbol| {
        symbol.quote_asset == "USDT"
            && symbol.status == "TRADING"
            && symbol.spot_trading_allowed
            && prices.contains_key(&symbol.symbol)
    }) {
        let Some(current_price) = prices.get(&symbol.symbol).copied() else {
            continue;
        };
        let closes = fetch_daily_closes(&client, &symbol.symbol, days).await?;
        if closes.is_empty() {
            continue;
        }

        let average_price = closes.iter().sum::<f64>() / closes.len() as f64;
        if average_price > 0.0 {
            signals.push(DailyMaSignal::new(
                symbol,
                days,
                current_price,
                average_price,
                closes.len(),
            ));
        }

        sleep(Duration::from_millis(150)).await;
    }

    signals.sort_by(|a, b| {
        b.difference_percent
            .partial_cmp(&a.difference_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(signals)
}

pub async fn fetch_spot_daily_klines(
    settings: BinanceSettings,
    symbol: &str,
    days: u16,
) -> anyhow::Result<Vec<SpotDailyKline>> {
    let days = days.clamp(1, 1000);
    let client = spot_rest_client(settings)?;
    fetch_daily_klines(&client, symbol, days).await
}

fn spot_rest_client(settings: BinanceSettings) -> anyhow::Result<RestApi> {
    let config = ConfigurationRestApi::builder()
        .build()
        .context("build Binance REST API config failed")?;
    Ok(match settings.environment() {
        BinanceEnvironment::Production => SpotRestApi::production(config),
        BinanceEnvironment::Testnet => SpotRestApi::testnet(config),
    })
}

async fn fetch_spot_symbols_with_client(client: &RestApi) -> anyhow::Result<Vec<SpotSymbolInfo>> {
    let response = client
        .exchange_info(ExchangeInfoParams::default())
        .await
        .context("Binance spot exchange_info request failed")?;
    let data = response
        .data()
        .await
        .context("read Binance spot exchange_info response failed")?;

    let mut symbols = data
        .symbols
        .unwrap_or_default()
        .into_iter()
        .map(SpotSymbolInfo::from_sdk_symbol)
        .collect::<Vec<_>>();
    symbols.sort_by(|a, b| a.symbol.cmp(&b.symbol));

    Ok(symbols)
}

async fn fetch_ticker_prices(client: &RestApi) -> anyhow::Result<HashMap<String, f64>> {
    let response = client
        .ticker_price(TickerPriceParams::default())
        .await
        .context("Binance spot ticker_price request failed")?;
    let data = response
        .data()
        .await
        .context("read Binance spot ticker_price response failed")?;

    let mut prices = HashMap::new();
    match data {
        TickerPriceResponse::TickerPriceResponse1(ticker) => {
            if let (Some(symbol), Some(price)) = (ticker.symbol, ticker.price) {
                if let Ok(price) = price.parse::<f64>() {
                    prices.insert(symbol, price);
                }
            }
        }
        TickerPriceResponse::TickerPriceResponse2(tickers) => {
            for ticker in tickers {
                if let (Some(symbol), Some(price)) = (ticker.symbol, ticker.price) {
                    if let Ok(price) = price.parse::<f64>() {
                        prices.insert(symbol, price);
                    }
                }
            }
        }
        TickerPriceResponse::Other(_) => {}
    }

    Ok(prices)
}

async fn fetch_daily_closes(client: &RestApi, symbol: &str, days: u16) -> anyhow::Result<Vec<f64>> {
    Ok(fetch_daily_klines(client, symbol, days)
        .await?
        .into_iter()
        .map(|kline| kline.close_price)
        .collect())
}

async fn fetch_daily_klines(
    client: &RestApi,
    symbol: &str,
    days: u16,
) -> anyhow::Result<Vec<SpotDailyKline>> {
    let params = KlinesParams::builder(symbol.to_string(), KlinesIntervalEnum::Interval1d)
        .limit(i32::from(days))
        .build()
        .context("build Binance klines params failed")?;
    let response = client
        .klines(params)
        .await
        .with_context(|| format!("Binance spot klines request failed: {symbol}"))?;
    let data = response
        .data()
        .await
        .with_context(|| format!("read Binance spot klines response failed: {symbol}"))?;

    Ok(data
        .into_iter()
        .filter_map(|items| kline_from_items(symbol, items))
        .collect())
}

fn kline_from_items(symbol: &str, items: Vec<KlinesItemInner>) -> Option<SpotDailyKline> {
    Some(SpotDailyKline {
        symbol: symbol.to_string(),
        interval: "1d".to_string(),
        open_time: kline_item_as_i64(items.first()?)?,
        open_price: kline_item_as_f64(items.get(1)?)?,
        high_price: kline_item_as_f64(items.get(2)?)?,
        low_price: kline_item_as_f64(items.get(3)?)?,
        close_price: kline_item_as_f64(items.get(4)?)?,
        volume: kline_item_as_f64(items.get(5)?)?,
        close_time: kline_item_as_i64(items.get(6)?)?,
    })
}

fn kline_item_as_i64(item: &KlinesItemInner) -> Option<i64> {
    match item {
        KlinesItemInner::String(value) => value.parse().ok(),
        KlinesItemInner::Integer(value) => Some(*value),
        KlinesItemInner::Other(_) => None,
    }
}

fn kline_item_as_f64(item: &KlinesItemInner) -> Option<f64> {
    match item {
        KlinesItemInner::String(value) => value.parse().ok(),
        KlinesItemInner::Integer(value) => Some(*value as f64),
        KlinesItemInner::Other(_) => None,
    }
}

pub fn fetch_spot_symbols_blocking(
    settings: BinanceSettings,
) -> anyhow::Result<Vec<SpotSymbolInfo>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build Tokio runtime for Binance spot request failed")?;

    runtime.block_on(fetch_spot_symbols(settings))
}

pub fn fetch_usdt_daily_ma_signals_blocking(
    settings: BinanceSettings,
    days: u16,
) -> anyhow::Result<Vec<DailyMaSignal>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build Tokio runtime for Binance spot request failed")?;

    runtime.block_on(fetch_usdt_daily_ma_signals(settings, days))
}

pub fn fetch_spot_daily_klines_blocking(
    settings: BinanceSettings,
    symbol: &str,
    days: u16,
) -> anyhow::Result<Vec<SpotDailyKline>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build Tokio runtime for Binance spot request failed")?;

    runtime.block_on(fetch_spot_daily_klines(settings, symbol, days))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_empty_sdk_symbol_to_stable_defaults() {
        let symbol = SpotSymbolInfo::from_sdk_symbol(
            sdk::spot::rest_api::ExchangeInfoResponseSymbolsInner::new(),
        );

        assert_eq!(symbol.symbol, "");
        assert!(!symbol.spot_trading_allowed);
        assert!(symbol.order_types.is_empty());
    }

    #[test]
    fn reads_kline_close_price() {
        let item = KlinesItemInner::String("123.45".to_string());

        assert_eq!(kline_item_as_f64(&item), Some(123.45));
    }
}

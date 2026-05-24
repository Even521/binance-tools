//! Binance Alpha public market data API.
//!
//! Alpha market data is exposed through Binance web BAPI endpoints. These
//! endpoints are public and do not require API-key authentication.

use anyhow::{Context, bail};
use reqwest::{
    Url,
    header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, ORIGIN, REFERER},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{thread::sleep, time::Duration};

pub const ALPHA_REST_BASE_URL: &str = "https://www.binance.com";
pub const ALPHA_WS_BASE_URL: &str = "wss://nbstream.binance.com/w3w/wsa/stream";

pub const TOKEN_LIST_PATH: &str =
    "/bapi/defi/v1/public/wallet-direct/buw/wallet/cex/alpha/all/token/list";
pub const EXCHANGE_INFO_PATH: &str = "/bapi/defi/v1/public/alpha-trade/get-exchange-info";
pub const AGG_TRADES_PATH: &str = "/bapi/defi/v1/public/alpha-trade/agg-trades";
pub const KLINES_PATH: &str = "/bapi/defi/v1/public/alpha-trade/klines";
pub const TICKER_24HR_PATH: &str = "/bapi/defi/v1/public/alpha-trade/ticker";
pub const FULL_DEPTH_PATH: &str = "/bapi/defi/v1/public/alpha-trade/fullDepth";

#[derive(Debug, Deserialize)]
struct BapiResponse<T> {
    code: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default, rename = "messageDetail")]
    message_detail: Option<String>,
    #[serde(default)]
    success: Option<bool>,
    data: T,
}

impl<T> BapiResponse<T> {
    fn into_data(self) -> anyhow::Result<T> {
        if self.code != "000000" || self.success == Some(false) {
            let message = self
                .message_detail
                .filter(|message| !message.is_empty())
                .or_else(|| self.message.filter(|message| !message.is_empty()))
                .unwrap_or(self.code);
            bail!("Binance Alpha response error: {message}");
        }
        Ok(self.data)
    }
}

#[derive(Debug, Clone)]
pub struct AlphaClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl AlphaClient {
    pub fn new() -> anyhow::Result<Self> {
        Self::with_base_url(ALPHA_REST_BASE_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> anyhow::Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(alpha_default_headers())
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/124.0 Safari/537.36 binance-tools/0.1",
            )
            .build()
            .context("build Binance Alpha HTTP client failed")?;
        Ok(Self {
            base_url: base_url.into(),
            client,
        })
    }

    pub fn token_list(&self) -> anyhow::Result<Vec<AlphaToken>> {
        self.get(TOKEN_LIST_PATH, &[])
            .context("Binance Alpha token list request failed")
    }

    pub fn exchange_info(&self) -> anyhow::Result<AlphaExchangeInfo> {
        self.get(EXCHANGE_INFO_PATH, &[])
            .context("Binance Alpha exchange info request failed")
    }

    pub fn aggregate_trades(
        &self,
        params: AlphaAggregateTradesParams,
    ) -> anyhow::Result<Vec<AlphaAggregateTrade>> {
        self.get(AGG_TRADES_PATH, &params.query()).with_context(|| {
            format!(
                "Binance Alpha aggregate trades request failed: {}",
                params.symbol
            )
        })
    }

    pub fn klines(&self, params: AlphaKlinesParams) -> anyhow::Result<Vec<AlphaKline>> {
        let symbol = params.symbol.clone();
        let rows = self
            .get::<Vec<Vec<Value>>>(KLINES_PATH, &params.query())
            .with_context(|| format!("Binance Alpha klines request failed: {symbol}"))?;
        rows.into_iter()
            .map(|row| AlphaKline::try_from_row(&symbol, row))
            .collect()
    }

    pub fn ticker_24hr(&self, symbol: impl Into<String>) -> anyhow::Result<AlphaTicker24hr> {
        let symbol = symbol.into();
        self.get(TICKER_24HR_PATH, &[("symbol".to_string(), symbol.clone())])
            .with_context(|| format!("Binance Alpha 24hr ticker request failed: {symbol}"))
    }

    pub fn full_depth(&self, params: AlphaDepthParams) -> anyhow::Result<AlphaOrderBook> {
        self.get(FULL_DEPTH_PATH, &params.query())
            .with_context(|| format!("Binance Alpha full depth request failed: {}", params.symbol))
    }

    fn get<T>(&self, path: &str, query: &[(String, String)]) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = endpoint_url(&self.base_url, path, query)?;
        let mut last_error = None;
        for attempt in 0..3 {
            match self.get_once::<T>(url.clone()) {
                Ok(data) => return Ok(data),
                Err(err) => {
                    last_error = Some(err);
                    if attempt < 2 {
                        sleep(Duration::from_millis(800 * (attempt + 1) as u64));
                    }
                }
            }
        }

        Err(last_error.expect("Alpha request retry loop must run at least once"))
    }

    fn get_once<T>(&self, url: Url) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .client
            .get(url.clone())
            .send()
            .with_context(|| format!("send Binance Alpha request failed: {url}"))?
            .error_for_status()
            .with_context(|| format!("Binance Alpha request returned error status: {url}"))?
            .json::<BapiResponse<T>>()
            .with_context(|| format!("parse Binance Alpha response failed: {url}"))?;
        response.into_data()
    }
}

fn alpha_default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    headers.insert(ORIGIN, HeaderValue::from_static("https://www.binance.com"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://www.binance.com/"),
    );
    headers.insert("clienttype", HeaderValue::from_static("web"));
    headers.insert("lang", HeaderValue::from_static("zh-CN"));
    headers
}

pub fn fetch_alpha_tokens_blocking() -> anyhow::Result<Vec<AlphaToken>> {
    AlphaClient::new()?.token_list()
}

pub fn fetch_alpha_exchange_info_blocking() -> anyhow::Result<AlphaExchangeInfo> {
    AlphaClient::new()?.exchange_info()
}

pub fn fetch_alpha_aggregate_trades_blocking(
    params: AlphaAggregateTradesParams,
) -> anyhow::Result<Vec<AlphaAggregateTrade>> {
    AlphaClient::new()?.aggregate_trades(params)
}

pub fn fetch_alpha_klines_blocking(params: AlphaKlinesParams) -> anyhow::Result<Vec<AlphaKline>> {
    AlphaClient::new()?.klines(params)
}

pub fn fetch_alpha_daily_klines_blocking(
    symbol: &str,
    days: u16,
) -> anyhow::Result<Vec<AlphaDailyKline>> {
    let params = AlphaKlinesParams {
        symbol: symbol.to_string(),
        interval: "1d".to_string(),
        limit: Some(days.clamp(1, 1500)),
        start_time: None,
        end_time: None,
    };
    AlphaClient::new()?
        .klines(params)?
        .into_iter()
        .map(AlphaDailyKline::try_from)
        .collect()
}

pub fn fetch_alpha_ticker_24hr_blocking(
    symbol: impl Into<String>,
) -> anyhow::Result<AlphaTicker24hr> {
    AlphaClient::new()?.ticker_24hr(symbol)
}

pub fn fetch_alpha_full_depth_blocking(params: AlphaDepthParams) -> anyhow::Result<AlphaOrderBook> {
    AlphaClient::new()?.full_depth(params)
}

pub fn endpoint_url(base_url: &str, path: &str, query: &[(String, String)]) -> anyhow::Result<Url> {
    let mut url = Url::parse(base_url)
        .and_then(|base| base.join(path))
        .with_context(|| format!("build Binance Alpha endpoint URL failed: {base_url}{path}"))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url)
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AlphaToken {
    #[serde(default, rename = "tokenId")]
    pub token_id: String,
    #[serde(default, rename = "alphaId")]
    pub alpha_id: String,
    #[serde(default, rename = "chainId")]
    pub chain_id: String,
    #[serde(default, rename = "chainName")]
    pub chain_name: String,
    #[serde(default, rename = "contractAddress")]
    pub contract_address: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default, rename = "percentChange24h")]
    pub percent_change_24h: Option<String>,
    #[serde(default, rename = "volume24h")]
    pub volume_24h: Option<String>,
    #[serde(default, rename = "marketCap")]
    pub market_cap: Option<String>,
    #[serde(default)]
    pub liquidity: Option<String>,
    #[serde(default, rename = "listingCex")]
    pub listing_cex: bool,
    #[serde(default, rename = "cexCoinName")]
    pub cex_coin_name: String,
    #[serde(default, rename = "stockState")]
    pub stock_state: bool,
    #[serde(default, rename = "cexOffDisplay")]
    pub cex_off_display: bool,
    #[serde(default, rename = "hotTag")]
    pub hot_tag: bool,
    #[serde(default, rename = "tradeDecimal")]
    pub trade_decimal: Option<i64>,
    #[serde(default, rename = "listingTime")]
    pub listing_time: Option<i64>,
    #[serde(default)]
    pub score: Option<i64>,
    #[serde(default, rename = "mulPoint")]
    pub mul_point: Option<i64>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AlphaExchangeInfo {
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub assets: Vec<AlphaAsset>,
    #[serde(default)]
    pub symbols: Vec<AlphaSymbol>,
    #[serde(default, rename = "orderTypes")]
    pub order_types: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AlphaAsset {
    #[serde(default)]
    pub asset: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AlphaSymbol {
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "baseAsset")]
    pub base_asset: String,
    #[serde(default, rename = "quoteAsset")]
    pub quote_asset: String,
    #[serde(default, rename = "pricePrecision")]
    pub price_precision: Option<i64>,
    #[serde(default, rename = "quantityPrecision")]
    pub quantity_precision: Option<i64>,
    #[serde(default, rename = "baseAssetPrecision")]
    pub base_asset_precision: Option<i64>,
    #[serde(default, rename = "quotePrecision")]
    pub quote_precision: Option<i64>,
    #[serde(default)]
    pub filters: Vec<AlphaSymbolFilter>,
    #[serde(default, rename = "orderTypes")]
    pub order_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AlphaSymbolFilter {
    #[serde(default, rename = "filterType")]
    pub filter_type: String,
    #[serde(default, rename = "minPrice")]
    pub min_price: Option<String>,
    #[serde(default, rename = "maxPrice")]
    pub max_price: Option<String>,
    #[serde(default, rename = "tickSize")]
    pub tick_size: Option<String>,
    #[serde(default, rename = "stepSize")]
    pub step_size: Option<String>,
    #[serde(default, rename = "maxQty")]
    pub max_qty: Option<String>,
    #[serde(default, rename = "minQty")]
    pub min_qty: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default, rename = "minNotional")]
    pub min_notional: Option<String>,
    #[serde(default, rename = "maxNotional")]
    pub max_notional: Option<String>,
    #[serde(default, rename = "multiplierDown")]
    pub multiplier_down: Option<String>,
    #[serde(default, rename = "multiplierUp")]
    pub multiplier_up: Option<String>,
    #[serde(default, rename = "bidMultiplierUp")]
    pub bid_multiplier_up: Option<String>,
    #[serde(default, rename = "askMultiplierUp")]
    pub ask_multiplier_up: Option<String>,
    #[serde(default, rename = "bidMultiplierDown")]
    pub bid_multiplier_down: Option<String>,
    #[serde(default, rename = "askMultiplierDown")]
    pub ask_multiplier_down: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlphaDailyMaSignal {
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
pub struct AlphaDailyKline {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlphaAggregateTradesParams {
    pub symbol: String,
    pub from_id: Option<i64>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub limit: Option<u16>,
}

impl AlphaAggregateTradesParams {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            from_id: None,
            start_time: None,
            end_time: None,
            limit: None,
        }
    }

    fn query(&self) -> Vec<(String, String)> {
        let mut query = vec![("symbol".to_string(), self.symbol.clone())];
        push_opt(&mut query, "fromId", self.from_id);
        push_opt(&mut query, "startTime", self.start_time);
        push_opt(&mut query, "endTime", self.end_time);
        push_opt(&mut query, "limit", self.limit);
        query
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlphaKlinesParams {
    pub symbol: String,
    pub interval: String,
    pub limit: Option<u16>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}

impl AlphaKlinesParams {
    pub fn new(symbol: impl Into<String>, interval: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            interval: interval.into(),
            limit: None,
            start_time: None,
            end_time: None,
        }
    }

    fn query(&self) -> Vec<(String, String)> {
        let mut query = vec![
            ("symbol".to_string(), self.symbol.clone()),
            ("interval".to_string(), self.interval.clone()),
        ];
        push_opt(&mut query, "limit", self.limit);
        push_opt(&mut query, "startTime", self.start_time);
        push_opt(&mut query, "endTime", self.end_time);
        query
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlphaDepthParams {
    pub symbol: String,
    pub limit: Option<u16>,
}

impl AlphaDepthParams {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            limit: None,
        }
    }

    fn query(&self) -> Vec<(String, String)> {
        let mut query = vec![("symbol".to_string(), self.symbol.clone())];
        push_opt(&mut query, "limit", self.limit);
        query
    }
}

fn push_opt<T>(query: &mut Vec<(String, String)>, key: &str, value: Option<T>)
where
    T: ToString,
{
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AlphaAggregateTrade {
    #[serde(rename = "a")]
    pub aggregate_trade_id: i64,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "q")]
    pub quantity: String,
    #[serde(rename = "f")]
    pub first_trade_id: i64,
    #[serde(rename = "l")]
    pub last_trade_id: i64,
    #[serde(rename = "T")]
    pub time: i64,
    #[serde(default, rename = "m")]
    pub buyer_is_maker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlphaKline {
    pub symbol: String,
    pub open_time: i64,
    pub open_price: String,
    pub high_price: String,
    pub low_price: String,
    pub close_price: String,
    pub volume: String,
    pub close_time: i64,
    pub quote_volume: String,
    pub trade_count: i64,
    pub taker_buy_base_volume: String,
    pub taker_buy_quote_volume: String,
}

impl AlphaKline {
    fn try_from_row(symbol: &str, row: Vec<Value>) -> anyhow::Result<Self> {
        Ok(Self {
            symbol: symbol.to_string(),
            open_time: value_i64(row.first(), "open_time")?,
            open_price: value_string(row.get(1), "open_price")?,
            high_price: value_string(row.get(2), "high_price")?,
            low_price: value_string(row.get(3), "low_price")?,
            close_price: value_string(row.get(4), "close_price")?,
            volume: value_string(row.get(5), "volume")?,
            close_time: value_i64(row.get(6), "close_time")?,
            quote_volume: value_string(row.get(7), "quote_volume")?,
            trade_count: value_i64(row.get(8), "trade_count")?,
            taker_buy_base_volume: value_string(row.get(9), "taker_buy_base_volume")?,
            taker_buy_quote_volume: value_string(row.get(10), "taker_buy_quote_volume")?,
        })
    }
}

impl TryFrom<AlphaKline> for AlphaDailyKline {
    type Error = anyhow::Error;

    fn try_from(kline: AlphaKline) -> Result<Self, Self::Error> {
        Ok(Self {
            symbol: kline.symbol,
            interval: "1d".to_string(),
            open_time: kline.open_time,
            open_price: parse_kline_number(&kline.open_price, "open_price")?,
            high_price: parse_kline_number(&kline.high_price, "high_price")?,
            low_price: parse_kline_number(&kline.low_price, "low_price")?,
            close_price: parse_kline_number(&kline.close_price, "close_price")?,
            volume: parse_kline_number(&kline.volume, "volume")?,
            close_time: kline.close_time,
        })
    }
}

fn parse_kline_number(value: &str, field: &str) -> anyhow::Result<f64> {
    value
        .parse::<f64>()
        .with_context(|| format!("invalid Alpha daily kline {field}: {value}"))
}

fn value_string(value: Option<&Value>, field: &str) -> anyhow::Result<String> {
    match value {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        _ => bail!("invalid Alpha kline {field}"),
    }
}

fn value_i64(value: Option<&Value>, field: &str) -> anyhow::Result<i64> {
    match value {
        Some(Value::Number(value)) => value
            .as_i64()
            .with_context(|| format!("invalid Alpha kline {field}")),
        Some(Value::String(value)) => value
            .parse::<i64>()
            .with_context(|| format!("invalid Alpha kline {field}")),
        _ => bail!("invalid Alpha kline {field}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AlphaTicker24hr {
    pub symbol: String,
    #[serde(rename = "priceChange")]
    pub price_change: String,
    #[serde(rename = "priceChangePercent")]
    pub price_change_percent: String,
    #[serde(rename = "weightedAvgPrice")]
    pub weighted_avg_price: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "lastQty")]
    pub last_qty: String,
    #[serde(rename = "openPrice")]
    pub open_price: String,
    #[serde(rename = "highPrice")]
    pub high_price: String,
    #[serde(rename = "lowPrice")]
    pub low_price: String,
    pub volume: String,
    #[serde(rename = "quoteVolume")]
    pub quote_volume: String,
    #[serde(rename = "openTime")]
    pub open_time: i64,
    #[serde(rename = "closeTime")]
    pub close_time: i64,
    #[serde(rename = "firstId")]
    pub first_id: i64,
    #[serde(rename = "lastId")]
    pub last_id: i64,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AlphaOrderBook {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: i64,
    pub symbol: String,
    #[serde(default)]
    pub bids: Vec<AlphaOrderBookLevel>,
    #[serde(default)]
    pub asks: Vec<AlphaOrderBookLevel>,
    #[serde(default, rename = "E")]
    pub event_time: Option<i64>,
    #[serde(default, rename = "T")]
    pub transaction_time: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlphaOrderBookLevel {
    pub price: String,
    pub quantity: String,
}

impl<'de> Deserialize<'de> for AlphaOrderBookLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<String>::deserialize(deserializer)?;
        if values.len() != 2 {
            return Err(serde::de::Error::custom(
                "order book level must have 2 fields",
            ));
        }
        Ok(Self {
            price: values[0].clone(),
            quantity: values[1].clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlphaWebSocketCommand<'a> {
    pub method: &'a str,
    pub params: Vec<String>,
    pub id: Value,
}

pub fn alpha_ws_subscribe_message(
    streams: Vec<String>,
    id: Value,
) -> AlphaWebSocketCommand<'static> {
    AlphaWebSocketCommand {
        method: "SUBSCRIBE",
        params: streams,
        id,
    }
}

pub fn alpha_ws_unsubscribe_message(
    streams: Vec<String>,
    id: Value,
) -> AlphaWebSocketCommand<'static> {
    AlphaWebSocketCommand {
        method: "UNSUBSCRIBE",
        params: streams,
        id,
    }
}

pub fn alpha_ws_list_subscriptions_message(id: Value) -> AlphaWebSocketCommand<'static> {
    AlphaWebSocketCommand {
        method: "LIST_SUBSCRIPTION",
        params: Vec::new(),
        id,
    }
}

pub fn alpha_all_tokens_ticker_24hr_stream() -> &'static str {
    "came@allTokens@ticker24"
}

pub fn alpha_symbol_stream(symbol: &str, stream: &str) -> String {
    format!("{}@{stream}", symbol.to_ascii_lowercase())
}

pub fn alpha_full_depth_stream(symbol: &str, interval: &str) -> String {
    format!("{}@fulldepth@{interval}", symbol.to_ascii_lowercase())
}

pub fn alpha_partial_depth_stream(symbol: &str, levels: u16, interval: Option<&str>) -> String {
    match interval {
        Some(interval) => format!("{}@depth{levels}@{interval}", symbol.to_ascii_lowercase()),
        None => format!("{}@depth{levels}", symbol.to_ascii_lowercase()),
    }
}

pub fn alpha_symbol_kline_stream(symbol: &str, interval: &str) -> String {
    alpha_symbol_stream(symbol, &format!("kline_{interval}"))
}

pub fn alpha_contract_kline_stream(
    contract_address: &str,
    chain_id: &str,
    interval: &str,
) -> String {
    format!("came@{contract_address}@{chain_id}@kline_{interval}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_alpha_query_url() {
        let params = AlphaKlinesParams {
            symbol: "ALPHA_175USDT".to_string(),
            interval: "1h".to_string(),
            limit: Some(2),
            start_time: None,
            end_time: None,
        };

        let url = endpoint_url(ALPHA_REST_BASE_URL, KLINES_PATH, &params.query()).unwrap();

        assert_eq!(
            url.as_str(),
            "https://www.binance.com/bapi/defi/v1/public/alpha-trade/klines?symbol=ALPHA_175USDT&interval=1h&limit=2"
        );
    }

    #[test]
    fn parses_alpha_kline_row() {
        let row = vec![
            json!("1752642000000"),
            json!("0.00171473"),
            json!("0.00172515"),
            json!("0.00171473"),
            json!("0.00172515"),
            json!("1771.86000000"),
            json!("1752645599999"),
            json!("3.05093481"),
            json!("2"),
            json!("1771.86000000"),
            json!("3.05093481"),
            json!(0),
        ];

        let kline = AlphaKline::try_from_row("ALPHA_175USDT", row).unwrap();

        assert_eq!(kline.symbol, "ALPHA_175USDT");
        assert_eq!(kline.open_time, 1752642000000);
        assert_eq!(kline.trade_count, 2);
        assert_eq!(kline.close_price, "0.00172515");
    }

    #[test]
    fn builds_websocket_stream_names() {
        assert_eq!(
            alpha_symbol_stream("ALPHA_116USDT", "aggTrade"),
            "alpha_116usdt@aggTrade"
        );
        assert_eq!(
            alpha_full_depth_stream("ALPHA_474USDT", "500ms"),
            "alpha_474usdt@fulldepth@500ms"
        );
        assert_eq!(
            alpha_symbol_kline_stream("ALPHA_116USDT", "1m"),
            "alpha_116usdt@kline_1m"
        );
        assert_eq!(
            alpha_contract_kline_stream("0xabc", "56", "1s"),
            "came@0xabc@56@kline_1s"
        );
    }
}

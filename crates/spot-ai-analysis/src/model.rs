//! 现货 AI 分析领域数据模型。

#[derive(Clone, Debug)]
pub struct MarketSnapshot {
    pub symbol: String,
    pub base_asset: String,
    pub asset_name: String,
    pub quote_asset: String,
    pub is_trading: bool,
    pub is_etf: bool,
    pub price_change_percent: Option<f64>,
    pub quote_volume: Option<f64>,
    pub last_price: Option<f64>,
    pub high_price: Option<f64>,
    pub low_price: Option<f64>,
    pub volume: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct DailyMaSnapshot {
    pub symbol: String,
    pub difference_percent: f64,
    pub samples: usize,
}

#[derive(Clone, Debug)]
pub struct AnalysisResult {
    pub symbol: String,
    pub trend_score: f64,
    pub volume_score: f64,
    pub technical_score: f64,
    pub risk_score: f64,
    pub sentiment_score: f64,
    pub sentiment_value: f64,
    pub sentiment_classification: String,
    pub sentiment_update_time: String,
    pub liquidity_score: f64,
    pub total_score: f64,
    pub trend_label: String,
    pub basis: String,
}


/// CoinMarketCap 全市场恐惧与贪婪指数。
#[derive(Clone, Debug)]
pub struct MarketSentiment {
    /// 指数原始值，范围 0..=100。
    pub value: f64,
    pub classification: String,
    pub update_time: String,
}

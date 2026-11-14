//! USDT 现货候选筛选与六维基础评分。

use crate::{AnalysisResult, DailyMaSnapshot, MarketSentiment, MarketSnapshot};
use std::{cmp::Ordering, collections::HashMap};

pub const SCORE_WEIGHTS: [f64; 6] = [0.25, 0.20, 0.20, 0.15, 0.10, 0.10];

/// 不参与现货方向性分析的稳定币基础资产（报价资产 USDT 不受影响）。
const STABLECOIN_BASE_ASSETS: &[&str] = &[
    "USDT", "USDC", "BUSD", "TUSD", "FDUSD", "DAI", "USDP", "USDE",
    "PYUSD", "USDS", "GUSD", "FRAX", "LUSD", "USDD", "UST", "USTC",
    "USD1", "USDJ", "CRVUSD", "EURI", "AEUR", "RLUSD", "USDG",
];

fn is_stablecoin_base_asset(asset: &str) -> bool {
    STABLECOIN_BASE_ASSETS
        .iter()
        .any(|stablecoin| asset.eq_ignore_ascii_case(stablecoin))
}

/// 筛选可交易 USDT 现货币种，计算六维基础分并按总分降序排列。
pub fn analyze_market(products: &[MarketSnapshot], cached_ma: &[DailyMaSnapshot]) -> Vec<AnalysisResult> {
    analyze_market_with_sentiment(products, cached_ma, None)
}

/// 使用 CoinMarketCap 全市场恐惧与贪婪指数计算情绪维度。
pub fn analyze_market_with_sentiment(
    products: &[MarketSnapshot],
    cached_ma: &[DailyMaSnapshot],
    sentiment: Option<&MarketSentiment>,
) -> Vec<AnalysisResult> {
    let candidates = products.iter()
        .filter(|p| p.quote_asset == "USDT" && p.is_trading && !p.is_etf && !is_stablecoin_base_asset(&p.base_asset))
        .collect::<Vec<_>>();
    let mut volumes = candidates.iter().filter_map(|p| p.quote_volume)
        .filter(|v| v.is_finite() && *v >= 0.0).collect::<Vec<_>>();
    volumes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let ma = cached_ma.iter().map(|s| (s.symbol.as_str(), s)).collect::<HashMap<_, _>>();

    let mut results = candidates.into_iter().map(|p| {
        let change = p.price_change_percent.unwrap_or(0.0);
        let trend = (50.0 + change * 5.0).clamp(0.0, 100.0);
        let volume = percentile_rank(&volumes, p.quote_volume.unwrap_or(0.0));
        let ma_signal = ma.get(p.symbol.as_str()).copied();
        let technical = ma_signal.map(|s| (55.0 + s.difference_percent * 4.0).clamp(0.0, 100.0)).unwrap_or(50.0);
        let amplitude = p.high_price.zip(p.low_price).filter(|(_, low)| *low > 0.0)
            .map(|(high, low)| (high - low) / low * 100.0).unwrap_or(0.0);
        let risk = (100.0 - amplitude * 4.0).clamp(0.0, 100.0);
        // 使用全市场恐惧与贪婪指数；无数据时回退为中性值。
        let sentiment_score = sentiment.map(|s| s.value.clamp(0.0, 100.0)).unwrap_or(50.0);
        let liquidity = volume * 0.7 + (100.0 - amplitude * 2.0).clamp(0.0, 100.0) * 0.3;
        let scores = [trend, volume, technical, risk, sentiment_score, liquidity];
        let total = scores.iter().zip(SCORE_WEIGHTS).map(|(s, w)| s * w).sum();
        let label = if change >= 3.0 { "趋势偏强" }
            else if change >= 0.8 { "温和走强" }
            else if change <= -3.0 { "趋势偏弱" }
            else if change <= -0.8 { "温和走弱" }
            else { "区间震荡" }.to_string();
        let basis = match ma_signal {
            Some(s) => format!("24 小时涨跌幅 {:+.2}%；近 30 日收盘均价偏离 {:+.2}%，样本 {} 根。成交额分数为当前 USDT 候选币种中的相对排名。", change, s.difference_percent, s.samples),
            None => format!("24 小时涨跌幅 {:+.2}%；当前没有该币种的完整日线均线缓存。成交额分数为当前 USDT 候选币种中的相对排名。", change),
        };
        AnalysisResult {
            symbol: p.symbol.clone(), trend_score: trend, volume_score: volume,
            technical_score: technical, risk_score: risk, sentiment_score,
            sentiment_value: sentiment.map(|s| s.value).unwrap_or(50.0),
            sentiment_classification: sentiment.map(|s| s.classification.clone()).unwrap_or_else(|| \"数据暂不可用（中性占位）\".to_string()),
            sentiment_update_time: sentiment.map(|s| s.update_time.clone()).unwrap_or_else(|| \"\".to_string()),
            liquidity_score: liquidity, total_score: total, trend_label: label, basis,
        }
    }).collect::<Vec<_>>();
    results.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(Ordering::Equal));
    results
}

fn percentile_rank(values: &[f64], value: f64) -> f64 {
    if values.is_empty() { return 50.0; }
    values.partition_point(|candidate| *candidate <= value) as f64 / values.len() as f64 * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(symbol: &str, change: f64, turnover: f64) -> MarketSnapshot {
        MarketSnapshot {
            symbol: symbol.into(), base_asset: symbol.into(), asset_name: symbol.into(),
            quote_asset: "USDT".into(), is_trading: true, is_etf: false,
            price_change_percent: Some(change), quote_volume: Some(turnover),
            last_price: Some(100.0), high_price: Some(105.0), low_price: Some(95.0), volume: Some(1000.0),
        }
    }

    #[test]
    fn filters_and_ranks() {
        let mut inactive = sample("OFFUSDT", 20.0, 9000.0);
        inactive.is_trading = false;
        let rows = analyze_market(&[sample("BTCUSDT", 2.0, 5000.0), sample("ETHUSDT", 1.0, 3000.0), inactive], &[]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "BTCUSDT");
    }

    #[test]
    fn score_weights_sum_to_one() {
        assert!((SCORE_WEIGHTS.iter().sum::<f64>() - 1.0).abs() < f64::EPSILON);
    }
}

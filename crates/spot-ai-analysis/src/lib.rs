//! 现货 AI 分析领域包：行情 DTO、六维评分和 AI 报告提示词。
//! 不依赖 GPUI、桌面窗口或 Binance SDK。

use std::{cmp::Ordering, collections::HashMap};

pub const SCORE_WEIGHTS: [f64; 6] = [0.25, 0.20, 0.20, 0.15, 0.10, 0.10];

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
    pub liquidity_score: f64,
    pub total_score: f64,
    pub trend_label: String,
    pub basis: String,
}

/// 筛选可交易 USDT 现货币种，计算六维基础分并按总分降序排列。
pub fn analyze_market(products: &[MarketSnapshot], cached_ma: &[DailyMaSnapshot]) -> Vec<AnalysisResult> {
    let candidates = products.iter()
        .filter(|p| p.quote_asset == "USDT" && p.is_trading && !p.is_etf)
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
        // 新闻情绪源尚未接入，使用中性占位并在报告中披露。
        let sentiment = 50.0;
        let liquidity = volume * 0.7 + (100.0 - amplitude * 2.0).clamp(0.0, 100.0) * 0.3;
        let scores = [trend, volume, technical, risk, sentiment, liquidity];
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
            technical_score: technical, risk_score: risk, sentiment_score: sentiment,
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

/// 只依据已提供数据生成提示词，禁止 AI 编造缺失指标、新闻或盘口。
pub fn build_report_prompt(m: &MarketSnapshot, a: &AnalysisResult) -> String {
    format!(
        "你是一名严谨的加密货币市场研究分析师。请用中文输出结构清晰、内容充分的研究报告，不要给出保证收益或直接下单指令。只能依据下列数据，不得编造 K 线、指标、支撑阻力、新闻或盘口数据。缺失数据必须明确写“数据不足”。\n\n交易对：{}\n最新价：{}\n24小时涨跌：{}\n24小时最高：{}\n24小时最低：{}\n24小时成交量：{}\n24小时成交额：{}\n日线均线缓存偏离：{}\n六维评分：趋势 {:.1}、成交活跃度 {:.1}、技术指标 {:.1}、波动风险 {:.1}、市场情绪 {:.1}、流动性 {:.1}；加权总分 {:.1}/100。\n数据限制：当前未接入 15m/1h/4h 完整指标、新闻情绪和实时盘口深度。成交额排名仅代表相对活跃程度，不代表资金净流入。\n\n请按章节输出：一、核心结论；二、趋势分析；三、成交量与活跃度；四、技术指标；五、风险分析；六、市场情绪；七、流动性；八、潜在机会；九、主要风险；十、需要进一步确认的条件。不要将评分直接当作买卖信号。",
        m.symbol, fmt_price(m.last_price), fmt_percent(m.price_change_percent),
        fmt_price(m.high_price), fmt_price(m.low_price), fmt_compact(m.volume),
        fmt_compact(m.quote_volume), a.basis, a.trend_score, a.volume_score,
        a.technical_score, a.risk_score, a.sentiment_score, a.liquidity_score, a.total_score
    )
}

fn fmt_percent(v: Option<f64>) -> String { v.map(|n| format!("{n:+.2}%")).unwrap_or_else(|| "-".into()) }
fn fmt_price(v: Option<f64>) -> String {
    let Some(v) = v else { return "-".into(); };
    if v >= 100.0 { format!("{v:.2}") } else if v >= 1.0 { format!("{v:.4}") }
    else { format!("{v:.8}").trim_end_matches('0').trim_end_matches('.').to_string() }
}
fn fmt_compact(v: Option<f64>) -> String {
    let Some(v) = v else { return "-".into(); };
    let a = v.abs();
    if a >= 1e9 { format!("{:.2}B", v / 1e9) }
    else if a >= 1e6 { format!("{:.2}M", v / 1e6) }
    else if a >= 1e3 { format!("{:.2}K", v / 1e3) }
    else { format!("{v:.2}") }
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
    fn prompt_discloses_missing_sources() {
        let market = sample("BTCUSDT", 2.0, 5000.0);
        let analysis = analyze_market(&[market.clone()], &[]).remove(0);
        let prompt = build_report_prompt(&market, &analysis);
        assert!(prompt.contains("新闻情绪"));
        assert!(prompt.contains("盘口深度"));
    }
}

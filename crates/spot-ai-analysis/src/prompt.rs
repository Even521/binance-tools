//! AI 研究报告提示词生成。

use crate::{AnalysisResult, MarketSnapshot};

/// 只依据已提供数据生成提示词，禁止 AI 编造缺失指标、新闻或盘口。
pub fn build_report_prompt(m: &MarketSnapshot, a: &AnalysisResult) -> String {
    format!(
        "你是一名严谨的加密货币市场研究分析师。请用中文输出结构清晰、内容充分的研究报告，不要给出保证收益或直接下单指令。只能依据下列数据，不得编造 K 线、指标、支撑阻力、新闻或盘口数据。缺失数据必须明确写“数据不足”。\n\n交易对：{}\n最新价：{}\n24小时涨跌：{}\n24小时最高：{}\n24小时最低：{}\n24小时成交量：{}\n24小时成交额：{}\nV2 特征依据：{}\n六维评分：趋势与动量 {:.1}、成交活跃度 {:.1}、风险控制 {:.1}、入场位置 {:.1}、市场情绪 {:.1}、流动性参考 {:.1}；加权总分 {:.1}/100，数据完整度 {:.0}%。\n市场情绪来源：CoinMarketCap 恐惧与贪婪指数，指数值 {:.0}/100，情绪分类：{}，数据更新时间：{}。这是全市场情绪，不是该交易对独立情绪。若情绪数据不可用，必须说明该因子已从总分权重中剔除；若分类包含“缓存数据”，必须说明使用的是上次成功获取的数据。\n数据限制：当前评分使用已收盘日线缓存；4小时结构、新闻情绪和实时盘口深度尚未接入。成交额排名仅代表相对活跃程度，不代表资金净流入。\n\n请按章节输出：一、核心结论；二、趋势分析；三、成交量与活跃度；四、技术指标；五、风险分析；六、市场情绪；七、流动性；八、潜在机会；九、主要风险；十、需要进一步确认的条件。不要将评分直接当作买卖信号。",
        m.symbol,
        fmt_price(m.last_price),
        fmt_percent(m.price_change_percent),
        fmt_price(m.high_price),
        fmt_price(m.low_price),
        fmt_compact(m.volume),
        fmt_compact(m.quote_volume),
        a.basis,
        a.trend_score,
        a.volume_score,
        a.technical_score,
        a.risk_score,
        a.sentiment_score,
        a.liquidity_score,
        a.total_score,
        a.data_completeness,
        a.sentiment_value,
        a.sentiment_classification,
        a.sentiment_update_time
    )
}

fn fmt_percent(v: Option<f64>) -> String {
    v.map(|n| format!("{n:+.2}%")).unwrap_or_else(|| "-".into())
}
fn fmt_price(v: Option<f64>) -> String {
    let Some(v) = v else {
        return "-".into();
    };
    if v >= 100.0 {
        format!("{v:.2}")
    } else if v >= 1.0 {
        format!("{v:.4}")
    } else {
        format!("{v:.8}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}
fn fmt_compact(v: Option<f64>) -> String {
    let Some(v) = v else {
        return "-".into();
    };
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.2}B", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.2}M", v / 1e6)
    } else if a >= 1e3 {
        format!("{:.2}K", v / 1e3)
    } else {
        format!("{v:.2}")
    }
}

#[cfg(test)]
mod tests {
    use crate::{DailyKlineSnapshot, MarketSnapshot, analyze_market};

    #[test]
    fn prompt_discloses_missing_sources() {
        let market = MarketSnapshot {
            symbol: "BTCUSDT".into(),
            base_asset: "BTC".into(),
            asset_name: "Bitcoin".into(),
            quote_asset: "USDT".into(),
            is_trading: true,
            is_etf: false,
            price_change_percent: Some(2.0),
            quote_volume: Some(5000.0),
            last_price: Some(100.0),
            high_price: Some(105.0),
            low_price: Some(95.0),
            volume: Some(1000.0),
            daily_klines: Vec::<DailyKlineSnapshot>::new(),
        };
        let analysis = analyze_market(&[market.clone()], &[]).remove(0);
        let prompt = build_report_prompt(&market, &analysis);
        assert!(prompt.contains("新闻情绪"));
        assert!(prompt.contains("盘口深度"));
    }
}

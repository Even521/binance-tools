//! 使用历史日线进行无未来数据泄漏的横截面策略回测。
use crate::{analyze_market_with_sentiment, DailyKlineSnapshot, MarketSnapshot};

#[derive(Clone, Debug)]
pub struct BacktestPoint {
    pub date: String,
    pub equity: f64,
    pub benchmark: f64,
    pub drawdown_percent: f64,
}

#[derive(Clone, Debug, Default)]
pub struct BacktestResult {
    pub points: Vec<BacktestPoint>,
    pub total_return_percent: f64,
    pub benchmark_return_percent: f64,
    pub max_drawdown_percent: f64,
    pub win_rate_percent: f64,
    pub profit_factor: f64,
    pub trade_days: usize,
    pub selected_days: usize,
    pub data_symbols: usize,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Clone, Debug)]
pub struct BacktestCandle {
    pub symbol: String,
    pub base_asset: String,
    pub asset_name: String,
    pub open_time: i64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub quote_turnover: f64,
}

const INITIAL_EQUITY: f64 = 10_000.0;
const MAX_POSITIONS: usize = 5;
const MIN_HISTORY: usize = 30;
const ROUND_TRIP_COST: f64 = 0.003; // 手续费与双边滑点合计估算 0.30%。

/// 每 3 个交易日收盘后按当日可见数据评分，下一交易日买入并持有 3 根日线；等权持有排名前五的币。
/// 只纳入至少 30 根历史日线且当日数据完整度不低于 70% 的交易对。
pub fn run_backtest(candles: &[BacktestCandle], period_days: usize) -> BacktestResult {
    use std::collections::{BTreeMap, HashMap};
    let mut by_symbol: HashMap<String, Vec<&BacktestCandle>> = HashMap::new();
    let mut dates = BTreeMap::<i64, ()>::new();
    for candle in candles {
        if candle.close.is_finite() && candle.close > 0.0 {
            by_symbol.entry(candle.symbol.clone()).or_default().push(candle);
            dates.insert(candle.open_time, ());
        }
    }
    for rows in by_symbol.values_mut() {
        rows.sort_by_key(|c| c.open_time);
    }
    let all_dates = dates.keys().copied().collect::<Vec<_>>();
    if all_dates.len() < MIN_HISTORY + 2 {
        return BacktestResult { data_symbols: by_symbol.len(), ..Default::default() };
    }
    let eval_start = all_dates.len().saturating_sub(period_days.saturating_add(1));
    let mut equity = INITIAL_EQUITY;
    let mut peak = equity;
    let mut max_drawdown: f64 = 0.0;
    let mut points = Vec::new();
    let mut daily_returns = Vec::new();
    let mut gross_profit = 0.0;
    let mut gross_loss = 0.0;
    let mut selected_days = 0usize;
    let mut btc_start = None::<f64>;
    let mut btc_last = None::<f64>;

    // eval_start 前的数据只作为指标预热；每天只使用当日及以前的已收盘 K 线。
    for day_index in (eval_start..all_dates.len().saturating_sub(3)).step_by(3) {
        let day = all_dates[day_index];
        let next_day = all_dates[day_index + 3];
        let mut snapshots = Vec::new();
        let mut next_prices = HashMap::<String, f64>::new();
        for (symbol, history) in &by_symbol {
            let end = history.partition_point(|c| c.open_time <= day);
            if end < MIN_HISTORY { continue; }
            let Some(today) = history.get(end - 1) else { continue };
            let Some(next) = history.get(end + 3) else { continue };
            if next.open_time != next_day { continue; }
            let closes = history[..end].iter().map(|c| c.close).collect::<Vec<_>>();
            let previous = closes.get(closes.len().saturating_sub(2)).copied().unwrap_or(today.close);
            let change = if previous > 0.0 { Some((today.close / previous - 1.0) * 100.0) } else { None };
            let daily_klines = history[..end].iter().map(|c| DailyKlineSnapshot {
                high: c.high, low: c.low, close: c.close, quote_turnover: c.quote_turnover,
            }).collect();
            snapshots.push(MarketSnapshot {
                symbol: symbol.clone(), base_asset: today.base_asset.clone(),
                asset_name: today.asset_name.clone(), quote_asset: "USDT".into(),
                is_trading: true, is_etf: false, price_change_percent: change,
                quote_volume: Some(today.quote_turnover), last_price: Some(today.close),
                high_price: Some(today.high), low_price: Some(today.low), volume: None,
                daily_klines,
            });
            next_prices.insert(symbol.clone(), next.close);
        }
        if snapshots.is_empty() { continue; }
        let scores = analyze_market_with_sentiment(&snapshots, &[], None);
        let mut ranked = scores.into_iter()
            .filter(|s| s.data_completeness >= 70.0)
            .filter_map(|s| next_prices.get(&s.symbol).copied().map(|next| (s.symbol, s.total_score, next)))
            .collect::<Vec<_>>();
        ranked.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let selected = ranked.into_iter().take(MAX_POSITIONS).collect::<Vec<_>>();
        let mut portfolio_return = 0.0;
        if !selected.is_empty() {
            selected_days += 1;
            for (symbol, _, next_close) in &selected {
                let Some(current) = snapshots.iter().find(|s| s.symbol == *symbol).and_then(|s| s.last_price) else { continue };
                if current <= 0.0 { continue; }
                let ret = next_close / current - 1.0 - ROUND_TRIP_COST;
                portfolio_return += ret;
                daily_returns.push(ret);
                if ret > 0.0 { gross_profit += ret; }
                else { gross_loss += ret.abs(); }
            }
            portfolio_return /= selected.len() as f64;
        } else {
            daily_returns.push(0.0);
        }
        equity *= 1.0 + portfolio_return;
        peak = peak.max(equity);
        let dd = if peak > 0.0 { (equity / peak - 1.0) * 100.0 } else { 0.0 };
        max_drawdown = max_drawdown.min(dd);

        // BTC 仅作买入持有基准，不参与策略选币。
        if let Some(btc) = by_symbol.get("BTCUSDT") {
            let start = btc.iter().find(|c| c.open_time == day).map(|c| c.close);
            let now = btc.iter().find(|c| c.open_time == next_day).map(|c| c.close);
            if btc_start.is_none() { btc_start = start; }
            if let (Some(a), Some(b)) = (btc_start, now) { btc_last = Some(b); }
        }
        let date = chrono::DateTime::from_timestamp_millis(next_day + 8 * 60 * 60 * 1000)
            .map(|d| d.format("%m-%d").to_string()).unwrap_or_else(|| next_day.to_string());
        points.push(BacktestPoint { date, equity, benchmark, drawdown_percent: dd });
    }
    let wins = daily_returns.iter().filter(|r| **r > 0.0).count();
    let total_return = (equity / INITIAL_EQUITY - 1.0) * 100.0;
    let benchmark_return = btc_start.zip(btc_last).filter(|(a,_)|*a>0.0)
        .map(|(a,b)|(b/a-1.0)*100.0).unwrap_or(0.0);
    let start_date = points.first().map(|p|p.date.clone()).unwrap_or_default();
    let end_date = points.last().map(|p|p.date.clone()).unwrap_or_default();
    BacktestResult {
        points, total_return_percent: total_return, benchmark_return_percent: benchmark_return,
        max_drawdown_percent: max_drawdown, win_rate_percent: if daily_returns.is_empty(){0.0}else{wins as f64/daily_returns.len() as f64*100.0},
        profit_factor: if gross_loss > 0.0 { gross_profit/gross_loss } else if gross_profit > 0.0 { f64::INFINITY } else { 0.0 },
        trade_days: daily_returns.len(), selected_days, data_symbols: by_symbol.len(), start_date, end_date,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_history_returns_empty_result() {
        let result = run_backtest(&[], 180);
        assert!(result.points.is_empty());
        assert_eq!(result.data_symbols, 0);
    }
}

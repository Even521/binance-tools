use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::BacktestResult;
use super::common::{
    SimpleBacktestState, average_true_range, validate_common, validate_optional_percent,
    validate_positive_percent,
};

#[derive(Clone, Debug)]
pub struct SuperTrendBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub atr_window: usize,
    pub multiplier: f64,
    pub unit_pct: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_supertrend_backtest(
    klines: &[VisionKline],
    params: SuperTrendBacktestParams,
) -> Result<BacktestResult> {
    validate_common(params.initial_cash, params.fee_rate)?;
    if params.atr_window < 2 {
        bail!("SuperTrend ATR window must be at least 2");
    }
    if params.multiplier <= 0.0 {
        bail!("SuperTrend multiplier must be greater than 0");
    }
    validate_positive_percent(params.unit_pct, "unit percent")?;
    validate_optional_percent(params.take_profit_pct, "take profit percent", 100.0)?;
    validate_optional_percent(params.stop_loss_pct, "stop loss percent", 50.0)?;
    if klines.len() < params.atr_window + 2 {
        bail!("not enough klines, need at least {}", params.atr_window + 2);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let mut state = SimpleBacktestState::new(params.initial_cash);
    let mut trend_up = false;

    for index in params.atr_window..sorted.len() {
        let atr = average_true_range(&sorted, index, params.atr_window);
        let mid = (sorted[index].high_price + sorted[index].low_price) / 2.0;
        let upper = mid + params.multiplier * atr;
        let lower = mid - params.multiplier * atr;
        let previous_close = sorted[index - 1].close_price;
        let price = sorted[index].close_price;
        let was_up = trend_up;
        if price > upper || (trend_up && price > lower) {
            trend_up = true;
        } else if price < lower || (!trend_up && price < upper) {
            trend_up = false;
        }

        if !state.has_position() && !was_up && trend_up && price > previous_close {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if state.has_position()
            && (!trend_up
                || state.take_profit_hit(price, params.take_profit_pct)
                || state.stop_hit(price, params.stop_loss_pct))
        {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(price);
    }

    Ok(state.finish(&sorted))
}

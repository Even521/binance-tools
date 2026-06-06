use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::BacktestResult;
use super::common::{
    SimpleBacktestState, validate_common, validate_optional_percent, validate_positive_percent,
};

#[derive(Clone, Debug)]
pub struct CciBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub period: usize,
    pub oversold: f64,
    pub overbought: f64,
    pub unit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_cci_backtest(
    klines: &[VisionKline],
    params: CciBacktestParams,
) -> Result<BacktestResult> {
    validate_common(params.initial_cash, params.fee_rate)?;
    if params.period < 2 {
        bail!("CCI period must be at least 2");
    }
    if params.oversold >= params.overbought {
        bail!("CCI oversold threshold must be smaller than overbought threshold");
    }
    validate_positive_percent(params.unit_pct, "unit percent")?;
    validate_optional_percent(params.stop_loss_pct, "stop loss percent", 50.0)?;
    if klines.len() < params.period + 2 {
        bail!("not enough klines, need at least {}", params.period + 2);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let typical_prices: Vec<f64> = sorted
        .iter()
        .map(|kline| (kline.high_price + kline.low_price + kline.close_price) / 3.0)
        .collect();
    let mut state = SimpleBacktestState::new(params.initial_cash);

    for index in params.period - 1..sorted.len() {
        let window = &typical_prices[index + 1 - params.period..=index];
        let mean = window.iter().sum::<f64>() / params.period as f64;
        let mean_deviation =
            window.iter().map(|value| (value - mean).abs()).sum::<f64>() / params.period as f64;
        if mean_deviation <= f64::EPSILON {
            continue;
        }
        let cci = (typical_prices[index] - mean) / (0.015 * mean_deviation);
        let price = sorted[index].close_price;
        if !state.has_position() && cci <= params.oversold {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if state.has_position()
            && (cci >= params.overbought || state.stop_hit(price, params.stop_loss_pct))
        {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(price);
    }

    Ok(state.finish(&sorted))
}

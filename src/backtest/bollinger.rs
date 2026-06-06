use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::BacktestResult;
use super::common::{
    SimpleBacktestState, sma, validate_common, validate_optional_percent, validate_positive_percent,
};

#[derive(Clone, Debug)]
pub struct BollingerBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub period: usize,
    pub std_multiplier: f64,
    pub unit_pct: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_bollinger_backtest(
    klines: &[VisionKline],
    params: BollingerBacktestParams,
) -> Result<BacktestResult> {
    validate_common(params.initial_cash, params.fee_rate)?;
    if params.period < 2 {
        bail!("Bollinger period must be at least 2");
    }
    if params.std_multiplier <= 0.0 {
        bail!("standard deviation multiplier must be greater than 0");
    }
    validate_positive_percent(params.unit_pct, "unit percent")?;
    validate_optional_percent(params.take_profit_pct, "take profit percent", 100.0)?;
    validate_optional_percent(params.stop_loss_pct, "stop loss percent", 50.0)?;
    if klines.len() < params.period + 2 {
        bail!("not enough klines, need at least {}", params.period + 2);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();
    let mut state = SimpleBacktestState::new(params.initial_cash);

    for index in params.period - 1..sorted.len() {
        let middle = sma(&closes, index, params.period);
        let std_dev = std_dev(&closes[index + 1 - params.period..=index], middle);
        let lower = middle - params.std_multiplier * std_dev;
        let price = sorted[index].close_price;
        if !state.has_position() && price <= lower {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if state.has_position()
            && (price >= middle
                || state.take_profit_hit(price, params.take_profit_pct)
                || state.stop_hit(price, params.stop_loss_pct))
        {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(price);
    }

    Ok(state.finish(&sorted))
}

fn std_dev(values: &[f64], mean: f64) -> f64 {
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

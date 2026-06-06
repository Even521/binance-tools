use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::BacktestResult;
use super::common::{
    SimpleBacktestState, highest_high, lowest_low, validate_common, validate_positive_percent,
};

#[derive(Clone, Debug)]
pub struct StochasticBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub k_window: usize,
    pub d_window: usize,
    pub oversold: f64,
    pub overbought: f64,
    pub unit_pct: f64,
}

pub fn run_stochastic_backtest(
    klines: &[VisionKline],
    params: StochasticBacktestParams,
) -> Result<BacktestResult> {
    validate_common(params.initial_cash, params.fee_rate)?;
    if params.k_window < 2 || params.d_window < 2 {
        bail!("Stochastic windows must be at least 2");
    }
    if !(0.0..100.0).contains(&params.oversold) || !(0.0..100.0).contains(&params.overbought) {
        bail!("Stochastic thresholds must be between 0 and 100");
    }
    if params.oversold >= params.overbought {
        bail!("Stochastic oversold threshold must be smaller than overbought threshold");
    }
    validate_positive_percent(params.unit_pct, "unit percent")?;
    let warmup = params.k_window + params.d_window;
    if klines.len() < warmup + 1 {
        bail!("not enough klines, need at least {}", warmup + 1);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let k_values = stochastic_k_series(&sorted, params.k_window);
    let mut state = SimpleBacktestState::new(params.initial_cash);

    for index in warmup..sorted.len() {
        let Some(k) = k_values[index] else { continue };
        let Some(prev_k) = k_values[index - 1] else {
            continue;
        };
        let Some(d) = average_optional(&k_values[index + 1 - params.d_window..=index]) else {
            continue;
        };
        let Some(prev_d) = average_optional(&k_values[index - params.d_window..index]) else {
            continue;
        };

        let bullish_cross = prev_k <= prev_d && k > d && k <= params.oversold;
        let bearish_cross = prev_k >= prev_d && k < d && k >= params.overbought;
        if !state.has_position() && bullish_cross {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if state.has_position() && bearish_cross {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(sorted[index].close_price);
    }

    Ok(state.finish(&sorted))
}

fn stochastic_k_series(klines: &[VisionKline], window: usize) -> Vec<Option<f64>> {
    let mut values = vec![None; klines.len()];
    for index in window - 1..klines.len() {
        let range = &klines[index + 1 - window..=index];
        let high = highest_high(range);
        let low = lowest_low(range);
        if high > low {
            values[index] = Some((klines[index].close_price - low) / (high - low) * 100.0);
        }
    }
    values
}

fn average_optional(values: &[Option<f64>]) -> Option<f64> {
    let mut total = 0.0;
    for value in values {
        total += (*value)?;
    }
    Some(total / values.len() as f64)
}

use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::BacktestResult;
use super::common::{
    SimpleBacktestState, sma, validate_common, validate_optional_percent, validate_positive_percent,
};

#[derive(Clone, Debug)]
pub struct ObvBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub obv_window: usize,
    pub price_window: usize,
    pub unit_pct: f64,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_obv_backtest(
    klines: &[VisionKline],
    params: ObvBacktestParams,
) -> Result<BacktestResult> {
    validate_common(params.initial_cash, params.fee_rate)?;
    if params.obv_window < 2 || params.price_window < 2 {
        bail!("OBV and price windows must be at least 2");
    }
    validate_positive_percent(params.unit_pct, "unit percent")?;
    validate_optional_percent(params.take_profit_pct, "take profit percent", 100.0)?;
    validate_optional_percent(params.stop_loss_pct, "stop loss percent", 50.0)?;
    let warmup = params.obv_window.max(params.price_window) + 1;
    if klines.len() < warmup + 1 {
        bail!("not enough klines, need at least {}", warmup + 1);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();
    let obv_values = obv_series(&sorted);
    let mut state = SimpleBacktestState::new(params.initial_cash);

    for index in warmup..sorted.len() {
        let price = sorted[index].close_price;
        let price_sma = sma(&closes, index, params.price_window);
        let obv_ma = sma(&obv_values, index, params.obv_window);
        let prev_obv_ma = sma(&obv_values, index - 1, params.obv_window);
        let obv_crosses_up = obv_values[index - 1] <= prev_obv_ma && obv_values[index] > obv_ma;
        let obv_crosses_down = obv_values[index - 1] >= prev_obv_ma && obv_values[index] < obv_ma;

        if !state.has_position() && obv_crosses_up && price > price_sma {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if state.has_position()
            && (obv_crosses_down
                || state.take_profit_hit(price, params.take_profit_pct)
                || state.stop_hit(price, params.stop_loss_pct))
        {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(price);
    }

    Ok(state.finish(&sorted))
}

fn obv_series(klines: &[VisionKline]) -> Vec<f64> {
    let mut values = vec![0.0; klines.len()];
    for index in 1..klines.len() {
        values[index] = if klines[index].close_price > klines[index - 1].close_price {
            values[index - 1] + klines[index].volume
        } else if klines[index].close_price < klines[index - 1].close_price {
            values[index - 1] - klines[index].volume
        } else {
            values[index - 1]
        };
    }
    values
}

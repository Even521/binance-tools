use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::BacktestResult;
use super::common::{
    SimpleBacktestState, highest_high, sma, validate_common, validate_optional_percent,
    validate_positive_percent,
};

#[derive(Clone, Debug)]
pub struct VolumeSpikeBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub breakout_window: usize,
    pub volume_window: usize,
    pub spike_ratio: f64,
    pub unit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_volume_spike_backtest(
    klines: &[VisionKline],
    params: VolumeSpikeBacktestParams,
) -> Result<BacktestResult> {
    validate_common(params.initial_cash, params.fee_rate)?;
    if params.breakout_window < 2 || params.volume_window < 2 {
        bail!("breakout and volume windows must be at least 2");
    }
    if params.spike_ratio <= 1.0 {
        bail!("volume spike ratio must be greater than 1");
    }
    validate_positive_percent(params.unit_pct, "unit percent")?;
    validate_optional_percent(params.stop_loss_pct, "stop loss percent", 50.0)?;
    let warmup = params.breakout_window.max(params.volume_window);
    if klines.len() < warmup + 2 {
        bail!("not enough klines, need at least {}", warmup + 2);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let volumes: Vec<f64> = sorted.iter().map(|kline| kline.volume).collect();
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();
    let mut state = SimpleBacktestState::new(params.initial_cash);

    for index in warmup..sorted.len() {
        let prior_high = highest_high(&sorted[index - params.breakout_window..index]);
        let avg_volume = sma(&volumes, index - 1, params.volume_window);
        let price_sma = sma(&closes, index - 1, params.breakout_window);
        let price = sorted[index].close_price;
        let volume_spike =
            avg_volume > 0.0 && sorted[index].volume / avg_volume >= params.spike_ratio;
        if !state.has_position() && price > prior_high && volume_spike {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if state.has_position()
            && (price < price_sma || state.stop_hit(price, params.stop_loss_pct))
        {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(price);
    }

    Ok(state.finish(&sorted))
}

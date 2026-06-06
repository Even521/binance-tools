use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct BacktestParams {
    pub initial_cash: f64,
    pub short_window: usize,
    pub long_window: usize,
    pub fee_rate: f64,
}

pub fn run_ma_cross_backtest(
    klines: &[VisionKline],
    params: BacktestParams,
) -> Result<BacktestResult> {
    if params.initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if params.short_window == 0 || params.long_window == 0 {
        bail!("MA windows must be greater than 0");
    }
    if params.short_window >= params.long_window {
        bail!("short MA window must be smaller than long MA window");
    }
    if !(0.0..0.1).contains(&params.fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    if klines.len() < params.long_window + 2 {
        bail!(
            "not enough klines, need at least {}",
            params.long_window + 2
        );
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();

    let mut cash = params.initial_cash;
    let mut position = 0.0;
    let mut equity_peak = params.initial_cash;
    let mut max_drawdown_pct: f64 = 0.0;
    let mut trades = Vec::new();
    let mut entry_equity = None::<f64>;
    let mut wins = 0usize;
    let mut completed_trades = 0usize;

    for index in params.long_window..sorted.len() {
        let short_ma = sma(&closes, index, params.short_window);
        let long_ma = sma(&closes, index, params.long_window);
        let prev_short_ma = sma(&closes, index - 1, params.short_window);
        let prev_long_ma = sma(&closes, index - 1, params.long_window);
        let price = sorted[index].close_price;

        let crosses_up = prev_short_ma <= prev_long_ma && short_ma > long_ma;
        let crosses_down = prev_short_ma >= prev_long_ma && short_ma < long_ma;

        if crosses_up && position <= f64::EPSILON {
            let gross_quantity = cash / price;
            let fee_quantity = gross_quantity * params.fee_rate;
            position = gross_quantity - fee_quantity;
            cash = 0.0;
            let equity = position * price;
            entry_equity = Some(equity);
            trades.push(BacktestTrade {
                time: sorted[index].open_time,
                action: BacktestAction::Buy,
                price,
                quantity: position,
                equity,
            });
        } else if crosses_down && position > f64::EPSILON {
            let gross_cash = position * price;
            cash = gross_cash * (1.0 - params.fee_rate);
            position = 0.0;
            completed_trades += 1;
            if entry_equity.map(|entry| cash > entry).unwrap_or(false) {
                wins += 1;
            }
            entry_equity = None;
            trades.push(BacktestTrade {
                time: sorted[index].open_time,
                action: BacktestAction::Sell,
                price,
                quantity: 0.0,
                equity: cash,
            });
        }

        let equity = cash + position * price;
        equity_peak = equity_peak.max(equity);
        if equity_peak > 0.0 {
            max_drawdown_pct = max_drawdown_pct.max((equity_peak - equity) / equity_peak * 100.0);
        }
    }

    let last_price = sorted
        .last()
        .map(|kline| kline.close_price)
        .unwrap_or_default();
    let final_equity = cash + position * last_price;
    let return_pct = (final_equity / params.initial_cash - 1.0) * 100.0;
    let win_rate_pct = if completed_trades == 0 {
        0.0
    } else {
        wins as f64 / completed_trades as f64 * 100.0
    };

    Ok(BacktestResult {
        initial_cash: params.initial_cash,
        final_equity,
        return_pct,
        max_drawdown_pct,
        trade_count: trades.len(),
        win_rate_pct,
        trades,
    })
}

fn sma(values: &[f64], end_index: usize, window: usize) -> f64 {
    let start = end_index + 1 - window;
    values[start..=end_index].iter().sum::<f64>() / window as f64
}

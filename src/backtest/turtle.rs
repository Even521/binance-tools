use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct TurtleBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub entry_window: usize,
    pub exit_window: usize,
    pub unit_pct: f64,
    pub atr_window: usize,
    pub stop_atr: f64,
}

pub fn run_turtle_backtest(
    klines: &[VisionKline],
    params: TurtleBacktestParams,
) -> Result<BacktestResult> {
    if params.initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if !(0.0..0.1).contains(&params.fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    if params.entry_window < 2 || params.exit_window < 2 || params.atr_window < 2 {
        bail!("turtle strategy windows must be at least 2");
    }
    if params.exit_window >= params.entry_window {
        bail!("exit window should be smaller than entry window");
    }
    if !(0.0..=100.0).contains(&params.unit_pct) || params.unit_pct <= 0.0 {
        bail!("unit percent must be between 0 and 100");
    }
    if params.stop_atr <= 0.0 {
        bail!("stop ATR multiplier must be greater than 0");
    }
    let warmup = params.entry_window.max(params.atr_window) + 1;
    if klines.len() < warmup + 1 {
        bail!("not enough klines, need at least {}", warmup + 1);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);

    let mut cash = params.initial_cash;
    let mut position = 0.0;
    let mut entry_price = 0.0;
    let mut equity_peak = params.initial_cash;
    let mut max_drawdown_pct: f64 = 0.0;
    let mut trades = Vec::new();
    let mut wins = 0usize;
    let mut completed_trades = 0usize;
    let mut entry_equity = None::<f64>;

    for index in warmup..sorted.len() {
        let price = sorted[index].close_price;
        let entry_high = highest_high(&sorted[index - params.entry_window..index]);
        let exit_low = lowest_low(&sorted[index - params.exit_window..index]);
        let atr = average_true_range(&sorted, index, params.atr_window);
        let stop_price = entry_price - atr * params.stop_atr;

        if position <= f64::EPSILON && price > entry_high {
            let spend = cash * (params.unit_pct / 100.0);
            if spend > f64::EPSILON && price > 0.0 {
                let gross_quantity = spend / price;
                let quantity = gross_quantity * (1.0 - params.fee_rate);
                cash -= spend;
                position = quantity;
                entry_price = price;
                let equity = cash + position * price;
                entry_equity = Some(equity);
                trades.push(BacktestTrade {
                    time: sorted[index].open_time,
                    action: BacktestAction::Buy,
                    price,
                    quantity: position,
                    equity,
                });
            }
        } else if position > f64::EPSILON && (price < exit_low || price <= stop_price) {
            let gross_cash = position * price;
            cash += gross_cash * (1.0 - params.fee_rate);
            position = 0.0;
            completed_trades += 1;
            if entry_equity.map(|entry| cash > entry).unwrap_or(false) {
                wins += 1;
            }
            entry_equity = None;
            let equity = cash;
            trades.push(BacktestTrade {
                time: sorted[index].open_time,
                action: BacktestAction::Sell,
                price,
                quantity: 0.0,
                equity,
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

fn highest_high(klines: &[VisionKline]) -> f64 {
    klines
        .iter()
        .map(|kline| kline.high_price)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn lowest_low(klines: &[VisionKline]) -> f64 {
    klines
        .iter()
        .map(|kline| kline.low_price)
        .fold(f64::INFINITY, f64::min)
}

fn average_true_range(klines: &[VisionKline], end_index: usize, window: usize) -> f64 {
    let start = end_index + 1 - window;
    let mut total = 0.0;
    for index in start..=end_index {
        let previous_close = if index == 0 {
            klines[index].close_price
        } else {
            klines[index - 1].close_price
        };
        let high_low = klines[index].high_price - klines[index].low_price;
        let high_close = (klines[index].high_price - previous_close).abs();
        let low_close = (klines[index].low_price - previous_close).abs();
        total += high_low.max(high_close).max(low_close);
    }
    total / window as f64
}

use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct TrendGridBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub lower_price: f64,
    pub upper_price: f64,
    pub grid_count: usize,
    pub trend_window: usize,
    pub stop_loss_pct: f64,
}

pub fn run_trend_grid_backtest(
    klines: &[VisionKline],
    params: TrendGridBacktestParams,
) -> Result<BacktestResult> {
    if params.initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if !(0.0..0.1).contains(&params.fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    if params.lower_price <= 0.0 || params.upper_price <= 0.0 {
        bail!("grid price bounds must be greater than 0");
    }
    if params.lower_price >= params.upper_price {
        bail!("grid lower price must be smaller than upper price");
    }
    if params.grid_count < 2 {
        bail!("grid count must be at least 2");
    }
    if params.trend_window < 2 {
        bail!("trend EMA window must be at least 2");
    }
    if !(0.0..=50.0).contains(&params.stop_loss_pct) {
        bail!("stop loss percent must be between 0 and 50");
    }
    if klines.len() < params.trend_window + 2 {
        bail!(
            "not enough klines, need at least {}",
            params.trend_window + 2
        );
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();
    let ema_values = ema_series(&closes, params.trend_window);

    let step = (params.upper_price - params.lower_price) / params.grid_count as f64;
    let order_cash = params.initial_cash / params.grid_count as f64;
    let mut cash = params.initial_cash;
    let mut position = 0.0;
    let mut equity_peak = params.initial_cash;
    let mut max_drawdown_pct: f64 = 0.0;
    let mut trades = Vec::new();
    let mut wins = 0usize;
    let mut completed_trades = 0usize;
    let mut last_grid = grid_index(
        sorted[0].close_price,
        params.lower_price,
        step,
        params.grid_count,
    );
    let mut entry_equity = None::<f64>;

    for (index, kline) in sorted.iter().enumerate().skip(1) {
        let price = kline.close_price;
        let Some(trend_ema) = ema_values[index] else {
            continue;
        };
        let current_grid = grid_index(price, params.lower_price, step, params.grid_count);
        let allow_buy = price >= trend_ema;
        let must_stop = price < trend_ema * (1.0 - params.stop_loss_pct / 100.0);

        if must_stop && position > f64::EPSILON {
            let gross_cash = position * price;
            cash += gross_cash * (1.0 - params.fee_rate);
            position = 0.0;
            completed_trades += 1;
            let equity = cash;
            if entry_equity.map(|entry| equity > entry).unwrap_or(false) {
                wins += 1;
            }
            entry_equity = None;
            trades.push(BacktestTrade {
                time: kline.open_time,
                action: BacktestAction::Sell,
                price,
                quantity: 0.0,
                equity,
            });
        } else if current_grid > last_grid {
            for _ in last_grid..current_grid {
                if position <= f64::EPSILON {
                    break;
                }
                let target_quantity = (order_cash / price).min(position);
                let gross_cash = target_quantity * price;
                cash += gross_cash * (1.0 - params.fee_rate);
                position -= target_quantity;
                completed_trades += 1;
                let equity = cash + position * price;
                if entry_equity.map(|entry| equity > entry).unwrap_or(false) {
                    wins += 1;
                }
                entry_equity = None;
                trades.push(BacktestTrade {
                    time: kline.open_time,
                    action: BacktestAction::Sell,
                    price,
                    quantity: position,
                    equity,
                });
            }
        }

        if allow_buy && current_grid < last_grid {
            for _ in current_grid..last_grid {
                if cash <= f64::EPSILON || price <= 0.0 {
                    break;
                }
                let spend = cash.min(order_cash);
                let gross_quantity = spend / price;
                let quantity = gross_quantity * (1.0 - params.fee_rate);
                cash -= spend;
                position += quantity;
                let equity = cash + position * price;
                entry_equity.get_or_insert(equity);
                trades.push(BacktestTrade {
                    time: kline.open_time,
                    action: BacktestAction::Buy,
                    price,
                    quantity: position,
                    equity,
                });
            }
        }

        last_grid = current_grid;
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

fn grid_index(price: f64, lower_price: f64, step: f64, grid_count: usize) -> usize {
    if price <= lower_price {
        return 0;
    }
    ((price - lower_price) / step)
        .floor()
        .clamp(0.0, grid_count as f64) as usize
}

fn ema_series(values: &[f64], window: usize) -> Vec<Option<f64>> {
    if values.is_empty() || window == 0 {
        return Vec::new();
    }

    let multiplier = 2.0 / (window as f64 + 1.0);
    let mut ema = 0.0;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            ema = if index == 0 {
                *value
            } else {
                (*value - ema) * multiplier + ema
            };
            if index + 1 >= window { Some(ema) } else { None }
        })
        .collect()
}

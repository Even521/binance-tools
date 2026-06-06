use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct GridBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub lower_price: f64,
    pub upper_price: f64,
    pub grid_count: usize,
}

pub fn run_grid_backtest(
    klines: &[VisionKline],
    params: GridBacktestParams,
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
    if klines.len() < 2 {
        bail!("not enough klines, need at least 2");
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);

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

    for kline in sorted.iter().skip(1) {
        let price = kline.close_price;
        let current_grid = grid_index(price, params.lower_price, step, params.grid_count);

        if current_grid < last_grid {
            for _ in current_grid..last_grid {
                if cash <= f64::EPSILON || price <= 0.0 {
                    break;
                }
                let spend = cash.min(order_cash);
                let gross_quantity = spend / price;
                let fee_quantity = gross_quantity * params.fee_rate;
                let quantity = gross_quantity - fee_quantity;
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
        } else if current_grid > last_grid {
            for _ in last_grid..current_grid {
                if position <= f64::EPSILON {
                    break;
                }
                let target_quantity = (order_cash / price).min(position);
                let gross_cash = target_quantity * price;
                let net_cash = gross_cash * (1.0 - params.fee_rate);
                position -= target_quantity;
                cash += net_cash;
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

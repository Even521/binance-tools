use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct MartingaleBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub drop_pct: f64,
    pub take_profit_pct: f64,
    pub max_levels: usize,
    pub first_order_pct: f64,
    pub multiplier: f64,
}

pub fn run_martingale_backtest(
    klines: &[VisionKline],
    params: MartingaleBacktestParams,
) -> Result<BacktestResult> {
    if params.initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if !(0.0..0.1).contains(&params.fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    if !(0.0..=80.0).contains(&params.drop_pct) || params.drop_pct <= 0.0 {
        bail!("drop percent must be between 0 and 80");
    }
    if !(0.0..=100.0).contains(&params.take_profit_pct) || params.take_profit_pct <= 0.0 {
        bail!("take profit percent must be between 0 and 100");
    }
    if !(0.0..=100.0).contains(&params.first_order_pct) || params.first_order_pct <= 0.0 {
        bail!("first order percent must be between 0 and 100");
    }
    if params.max_levels == 0 {
        bail!("max levels must be greater than 0");
    }
    if params.multiplier < 1.0 {
        bail!("martingale multiplier must be greater than or equal to 1");
    }
    if klines.len() < 2 {
        bail!("not enough klines, need at least 2");
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);

    let mut cash = params.initial_cash;
    let mut position = 0.0;
    let mut cost = 0.0;
    let mut levels = 0usize;
    let mut next_order_cash = params.initial_cash * params.first_order_pct / 100.0;
    let mut equity_peak = params.initial_cash;
    let mut max_drawdown_pct: f64 = 0.0;
    let mut trades = Vec::new();
    let mut wins = 0usize;
    let mut completed_trades = 0usize;

    for kline in &sorted {
        let price = kline.close_price;
        if price <= 0.0 {
            continue;
        }

        if position <= f64::EPSILON {
            let spend = cash.min(next_order_cash);
            if spend > f64::EPSILON {
                let quantity = spend / price * (1.0 - params.fee_rate);
                cash -= spend;
                cost = spend;
                position = quantity;
                levels = 1;
                trades.push(BacktestTrade {
                    time: kline.open_time,
                    action: BacktestAction::Buy,
                    price,
                    quantity: position,
                    equity: cash + position * price,
                });
            }
        } else {
            let avg_price = cost / position;
            let take_price = avg_price * (1.0 + params.take_profit_pct / 100.0);
            let add_price = avg_price * (1.0 - params.drop_pct / 100.0);

            if price >= take_price {
                let gross_cash = position * price;
                cash += gross_cash * (1.0 - params.fee_rate);
                position = 0.0;
                cost = 0.0;
                levels = 0;
                next_order_cash = params.initial_cash * params.first_order_pct / 100.0;
                completed_trades += 1;
                wins += 1;
                trades.push(BacktestTrade {
                    time: kline.open_time,
                    action: BacktestAction::Sell,
                    price,
                    quantity: 0.0,
                    equity: cash,
                });
            } else if price <= add_price && levels < params.max_levels && cash > f64::EPSILON {
                next_order_cash *= params.multiplier;
                let spend = cash.min(next_order_cash);
                let quantity = spend / price * (1.0 - params.fee_rate);
                cash -= spend;
                cost += spend;
                position += quantity;
                levels += 1;
                trades.push(BacktestTrade {
                    time: kline.open_time,
                    action: BacktestAction::Buy,
                    price,
                    quantity: position,
                    equity: cash + position * price,
                });
            }
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

use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct RsiBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub period: usize,
    pub oversold: f64,
    pub overbought: f64,
    pub unit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_rsi_backtest(
    klines: &[VisionKline],
    params: RsiBacktestParams,
) -> Result<BacktestResult> {
    if params.initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if !(0.0..0.1).contains(&params.fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    if params.period < 2 {
        bail!("RSI period must be at least 2");
    }
    if !(0.0..100.0).contains(&params.oversold) {
        bail!("RSI oversold threshold must be between 0 and 100");
    }
    if !(0.0..100.0).contains(&params.overbought) {
        bail!("RSI overbought threshold must be between 0 and 100");
    }
    if params.oversold >= params.overbought {
        bail!("RSI oversold threshold must be smaller than overbought threshold");
    }
    if !(0.0..=100.0).contains(&params.unit_pct) || params.unit_pct <= 0.0 {
        bail!("unit percent must be between 0 and 100");
    }
    if !(0.0..=50.0).contains(&params.stop_loss_pct) {
        bail!("stop loss percent must be between 0 and 50");
    }
    if klines.len() < params.period + 2 {
        bail!("not enough klines, need at least {}", params.period + 2);
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();
    let rsi_values = rsi_series(&closes, params.period);

    let mut cash = params.initial_cash;
    let mut position = 0.0;
    let mut entry_price = 0.0;
    let mut entry_equity = None::<f64>;
    let mut equity_peak = params.initial_cash;
    let mut max_drawdown_pct: f64 = 0.0;
    let mut trades = Vec::new();
    let mut wins = 0usize;
    let mut completed_trades = 0usize;

    for index in params.period..sorted.len() {
        let price = sorted[index].close_price;
        let Some(rsi) = rsi_values[index] else {
            continue;
        };

        if position <= f64::EPSILON && rsi <= params.oversold {
            let spend = cash * params.unit_pct / 100.0;
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
        } else if position > f64::EPSILON {
            let stop_hit = params.stop_loss_pct > 0.0
                && price <= entry_price * (1.0 - params.stop_loss_pct / 100.0);
            if rsi >= params.overbought || stop_hit {
                let gross_cash = position * price;
                cash += gross_cash * (1.0 - params.fee_rate);
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

fn rsi_series(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut output = vec![None; closes.len()];
    if closes.len() <= period || period == 0 {
        return output;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;
    for index in 1..=period {
        let change = closes[index] - closes[index - 1];
        if change >= 0.0 {
            avg_gain += change;
        } else {
            avg_loss += change.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;
    output[period] = Some(rsi_from_averages(avg_gain, avg_loss));

    for index in period + 1..closes.len() {
        let change = closes[index] - closes[index - 1];
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);
        avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
        output[index] = Some(rsi_from_averages(avg_gain, avg_loss));
    }

    output
}

fn rsi_from_averages(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss <= f64::EPSILON {
        return 100.0;
    }
    if avg_gain <= f64::EPSILON {
        return 0.0;
    }
    let relative_strength = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + relative_strength)
}

use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

#[derive(Clone, Debug)]
pub struct MacdBacktestParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub fast_window: usize,
    pub slow_window: usize,
    pub signal_window: usize,
    pub unit_pct: f64,
    pub stop_loss_pct: f64,
}

pub fn run_macd_backtest(
    klines: &[VisionKline],
    params: MacdBacktestParams,
) -> Result<BacktestResult> {
    if params.initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if !(0.0..0.1).contains(&params.fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    if params.fast_window < 2 || params.slow_window < 2 || params.signal_window < 2 {
        bail!("MACD windows must be at least 2");
    }
    if params.fast_window >= params.slow_window {
        bail!("MACD fast window must be smaller than slow window");
    }
    validate_percent(params.unit_pct, "unit percent")?;
    validate_stop(params.stop_loss_pct)?;
    if klines.len() < params.slow_window + params.signal_window + 2 {
        bail!(
            "not enough klines, need at least {}",
            params.slow_window + params.signal_window + 2
        );
    }

    let mut sorted = klines.to_vec();
    sorted.sort_by_key(|kline| kline.open_time);
    let closes: Vec<f64> = sorted.iter().map(|kline| kline.close_price).collect();
    let fast = ema_series(&closes, params.fast_window);
    let slow = ema_series(&closes, params.slow_window);
    let macd_line: Vec<Option<f64>> = fast
        .iter()
        .zip(slow.iter())
        .map(|(fast, slow)| fast.zip(*slow).map(|(fast, slow)| fast - slow))
        .collect();
    let signal = ema_optional_series(&macd_line, params.signal_window);

    let mut state = BacktestState::new(params.initial_cash);
    for index in 1..sorted.len() {
        let price = sorted[index].close_price;
        let Some(macd) = macd_line[index] else {
            continue;
        };
        let Some(sig) = signal[index] else { continue };
        let Some(prev_macd) = macd_line[index - 1] else {
            continue;
        };
        let Some(prev_sig) = signal[index - 1] else {
            continue;
        };

        let crosses_up = prev_macd <= prev_sig && macd > sig && macd > 0.0;
        let crosses_down = prev_macd >= prev_sig && macd < sig;
        let stop_hit = state.stop_hit(price, params.stop_loss_pct);

        if crosses_up {
            state.buy(&sorted[index], params.unit_pct, params.fee_rate);
        } else if crosses_down || stop_hit {
            state.sell(&sorted[index], params.fee_rate);
        }
        state.update_drawdown(price);
    }

    Ok(state.finish(&sorted))
}

fn ema_series(values: &[f64], window: usize) -> Vec<Option<f64>> {
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
            (index + 1 >= window).then_some(ema)
        })
        .collect()
}

fn ema_optional_series(values: &[Option<f64>], window: usize) -> Vec<Option<f64>> {
    let multiplier = 2.0 / (window as f64 + 1.0);
    let mut ema = 0.0;
    let mut seen = 0usize;
    values
        .iter()
        .map(|value| {
            let value = (*value)?;
            ema = if seen == 0 {
                value
            } else {
                (value - ema) * multiplier + ema
            };
            seen += 1;
            (seen >= window).then_some(ema)
        })
        .collect()
}

struct BacktestState {
    initial_cash: f64,
    cash: f64,
    position: f64,
    entry_price: f64,
    entry_equity: Option<f64>,
    equity_peak: f64,
    max_drawdown_pct: f64,
    trades: Vec<BacktestTrade>,
    wins: usize,
    completed_trades: usize,
}

impl BacktestState {
    fn new(initial_cash: f64) -> Self {
        Self {
            initial_cash,
            cash: initial_cash,
            position: 0.0,
            entry_price: 0.0,
            entry_equity: None,
            equity_peak: initial_cash,
            max_drawdown_pct: 0.0,
            trades: Vec::new(),
            wins: 0,
            completed_trades: 0,
        }
    }

    fn buy(&mut self, kline: &VisionKline, unit_pct: f64, fee_rate: f64) {
        if self.position > f64::EPSILON || kline.close_price <= 0.0 {
            return;
        }
        let spend = self.cash * unit_pct / 100.0;
        if spend <= f64::EPSILON {
            return;
        }
        self.position = spend / kline.close_price * (1.0 - fee_rate);
        self.cash -= spend;
        self.entry_price = kline.close_price;
        let equity = self.equity(kline.close_price);
        self.entry_equity = Some(equity);
        self.trades.push(BacktestTrade {
            time: kline.open_time,
            action: BacktestAction::Buy,
            price: kline.close_price,
            quantity: self.position,
            equity,
        });
    }

    fn sell(&mut self, kline: &VisionKline, fee_rate: f64) {
        if self.position <= f64::EPSILON {
            return;
        }
        self.cash += self.position * kline.close_price * (1.0 - fee_rate);
        self.position = 0.0;
        self.completed_trades += 1;
        if self
            .entry_equity
            .map(|entry| self.cash > entry)
            .unwrap_or(false)
        {
            self.wins += 1;
        }
        self.entry_equity = None;
        self.trades.push(BacktestTrade {
            time: kline.open_time,
            action: BacktestAction::Sell,
            price: kline.close_price,
            quantity: 0.0,
            equity: self.cash,
        });
    }

    fn stop_hit(&self, price: f64, stop_loss_pct: f64) -> bool {
        self.position > f64::EPSILON
            && stop_loss_pct > 0.0
            && price <= self.entry_price * (1.0 - stop_loss_pct / 100.0)
    }

    fn equity(&self, price: f64) -> f64 {
        self.cash + self.position * price
    }

    fn update_drawdown(&mut self, price: f64) {
        let equity = self.equity(price);
        self.equity_peak = self.equity_peak.max(equity);
        if self.equity_peak > 0.0 {
            self.max_drawdown_pct = self
                .max_drawdown_pct
                .max((self.equity_peak - equity) / self.equity_peak * 100.0);
        }
    }

    fn finish(self, klines: &[VisionKline]) -> BacktestResult {
        let last_price = klines
            .last()
            .map(|kline| kline.close_price)
            .unwrap_or_default();
        let final_equity = self.equity(last_price);
        BacktestResult {
            initial_cash: self.initial_cash,
            final_equity,
            return_pct: (final_equity / self.initial_cash - 1.0) * 100.0,
            max_drawdown_pct: self.max_drawdown_pct,
            trade_count: self.trades.len(),
            win_rate_pct: if self.completed_trades == 0 {
                0.0
            } else {
                self.wins as f64 / self.completed_trades as f64 * 100.0
            },
            trades: self.trades,
        }
    }
}

fn validate_percent(value: f64, label: &str) -> Result<()> {
    if !(0.0..=100.0).contains(&value) || value <= 0.0 {
        bail!("{label} must be between 0 and 100");
    }
    Ok(())
}

fn validate_stop(value: f64) -> Result<()> {
    if !(0.0..=50.0).contains(&value) {
        bail!("stop loss percent must be between 0 and 50");
    }
    Ok(())
}

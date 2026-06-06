use crate::binance::vision::VisionKline;
use anyhow::{Result, bail};

use super::{BacktestAction, BacktestResult, BacktestTrade};

pub(crate) struct SimpleBacktestState {
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

impl SimpleBacktestState {
    pub(crate) fn new(initial_cash: f64) -> Self {
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

    pub(crate) fn has_position(&self) -> bool {
        self.position > f64::EPSILON
    }

    pub(crate) fn buy(&mut self, kline: &VisionKline, unit_pct: f64, fee_rate: f64) {
        if self.has_position() || kline.close_price <= 0.0 {
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

    pub(crate) fn sell(&mut self, kline: &VisionKline, fee_rate: f64) {
        if !self.has_position() {
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

    pub(crate) fn stop_hit(&self, price: f64, stop_loss_pct: f64) -> bool {
        self.has_position()
            && stop_loss_pct > 0.0
            && price <= self.entry_price * (1.0 - stop_loss_pct / 100.0)
    }

    pub(crate) fn take_profit_hit(&self, price: f64, take_profit_pct: f64) -> bool {
        self.has_position()
            && take_profit_pct > 0.0
            && price >= self.entry_price * (1.0 + take_profit_pct / 100.0)
    }

    pub(crate) fn equity(&self, price: f64) -> f64 {
        self.cash + self.position * price
    }

    pub(crate) fn update_drawdown(&mut self, price: f64) {
        let equity = self.equity(price);
        self.equity_peak = self.equity_peak.max(equity);
        if self.equity_peak > 0.0 {
            self.max_drawdown_pct = self
                .max_drawdown_pct
                .max((self.equity_peak - equity) / self.equity_peak * 100.0);
        }
    }

    pub(crate) fn finish(self, klines: &[VisionKline]) -> BacktestResult {
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

pub(crate) fn validate_common(initial_cash: f64, fee_rate: f64) -> Result<()> {
    if initial_cash <= 0.0 {
        bail!("initial cash must be greater than 0");
    }
    if !(0.0..0.1).contains(&fee_rate) {
        bail!("fee rate must be between 0 and 0.1");
    }
    Ok(())
}

pub(crate) fn validate_positive_percent(value: f64, label: &str) -> Result<()> {
    if !(0.0..=100.0).contains(&value) || value <= 0.0 {
        bail!("{label} must be between 0 and 100");
    }
    Ok(())
}

pub(crate) fn validate_optional_percent(value: f64, label: &str, max: f64) -> Result<()> {
    if !(0.0..=max).contains(&value) {
        bail!("{label} must be between 0 and {max}");
    }
    Ok(())
}

pub(crate) fn sma(values: &[f64], end_index: usize, window: usize) -> f64 {
    let start = end_index + 1 - window;
    values[start..=end_index].iter().sum::<f64>() / window as f64
}

pub(crate) fn highest_high(klines: &[VisionKline]) -> f64 {
    klines
        .iter()
        .map(|kline| kline.high_price)
        .fold(f64::NEG_INFINITY, f64::max)
}

pub(crate) fn lowest_low(klines: &[VisionKline]) -> f64 {
    klines
        .iter()
        .map(|kline| kline.low_price)
        .fold(f64::INFINITY, f64::min)
}

pub(crate) fn average_true_range(klines: &[VisionKline], end_index: usize, window: usize) -> f64 {
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

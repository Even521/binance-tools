use crate::binance::vision::VisionKline;
use anyhow::Result;

mod bollinger;
mod cci;
mod common;
mod grid;
mod ma_cross;
mod macd;
mod martingale;
mod obv;
mod rsi;
mod stochastic;
mod supertrend;
mod trend_grid;
mod turtle;
mod volume_spike;

pub use bollinger::{BollingerBacktestParams, run_bollinger_backtest};
pub use cci::{CciBacktestParams, run_cci_backtest};
pub use grid::{GridBacktestParams, run_grid_backtest};
pub use ma_cross::{BacktestParams, run_ma_cross_backtest};
pub use macd::{MacdBacktestParams, run_macd_backtest};
pub use martingale::{MartingaleBacktestParams, run_martingale_backtest};
pub use obv::{ObvBacktestParams, run_obv_backtest};
pub use rsi::{RsiBacktestParams, run_rsi_backtest};
pub use stochastic::{StochasticBacktestParams, run_stochastic_backtest};
pub use supertrend::{SuperTrendBacktestParams, run_supertrend_backtest};
pub use trend_grid::{TrendGridBacktestParams, run_trend_grid_backtest};
pub use turtle::{TurtleBacktestParams, run_turtle_backtest};
pub use volume_spike::{VolumeSpikeBacktestParams, run_volume_spike_backtest};

#[derive(Clone, Debug)]
pub enum BacktestStrategy {
    MaCross {
        short_window: usize,
        long_window: usize,
    },
    Grid {
        lower_price: f64,
        upper_price: f64,
        grid_count: usize,
    },
    TrendGrid {
        lower_price: f64,
        upper_price: f64,
        grid_count: usize,
        trend_window: usize,
        stop_loss_pct: f64,
    },
    Turtle {
        entry_window: usize,
        exit_window: usize,
        unit_pct: f64,
        atr_window: usize,
        stop_atr: f64,
    },
    Martingale {
        drop_pct: f64,
        take_profit_pct: f64,
        max_levels: usize,
        first_order_pct: f64,
        multiplier: f64,
    },
    Rsi {
        period: usize,
        oversold: f64,
        overbought: f64,
        unit_pct: f64,
        stop_loss_pct: f64,
    },
    Macd {
        fast_window: usize,
        slow_window: usize,
        signal_window: usize,
        unit_pct: f64,
        stop_loss_pct: f64,
    },
    Bollinger {
        period: usize,
        std_multiplier: f64,
        unit_pct: f64,
        take_profit_pct: f64,
        stop_loss_pct: f64,
    },
    VolumeSpike {
        breakout_window: usize,
        volume_window: usize,
        spike_ratio: f64,
        unit_pct: f64,
        stop_loss_pct: f64,
    },
    Obv {
        obv_window: usize,
        price_window: usize,
        unit_pct: f64,
        take_profit_pct: f64,
        stop_loss_pct: f64,
    },
    Stochastic {
        k_window: usize,
        d_window: usize,
        oversold: f64,
        overbought: f64,
        unit_pct: f64,
    },
    Cci {
        period: usize,
        oversold: f64,
        overbought: f64,
        unit_pct: f64,
        stop_loss_pct: f64,
    },
    SuperTrend {
        atr_window: usize,
        multiplier: f64,
        unit_pct: f64,
        take_profit_pct: f64,
        stop_loss_pct: f64,
    },
}

impl BacktestStrategy {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::MaCross { .. } => "MA Cross",
            Self::Grid { .. } => "Grid",
            Self::TrendGrid { .. } => "Trend Grid",
            Self::Turtle { .. } => "Turtle",
            Self::Martingale { .. } => "Martingale",
            Self::Rsi { .. } => "RSI",
            Self::Macd { .. } => "MACD",
            Self::Bollinger { .. } => "Bollinger Bands",
            Self::VolumeSpike { .. } => "Volume Spike",
            Self::Obv { .. } => "OBV",
            Self::Stochastic { .. } => "Stochastic",
            Self::Cci { .. } => "CCI",
            Self::SuperTrend { .. } => "SuperTrend",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BacktestRunParams {
    pub initial_cash: f64,
    pub fee_rate: f64,
    pub strategy: BacktestStrategy,
}

#[derive(Clone, Debug)]
pub struct BacktestTrade {
    pub time: i64,
    pub action: BacktestAction,
    pub price: f64,
    pub quantity: f64,
    pub equity: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BacktestAction {
    Buy,
    Sell,
}

impl BacktestAction {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Buy => "买入",
            Self::Sell => "卖出",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BacktestResult {
    pub initial_cash: f64,
    pub final_equity: f64,
    pub return_pct: f64,
    pub max_drawdown_pct: f64,
    pub trade_count: usize,
    pub win_rate_pct: f64,
    pub trades: Vec<BacktestTrade>,
}

pub fn run_backtest(klines: &[VisionKline], params: BacktestRunParams) -> Result<BacktestResult> {
    match params.strategy {
        BacktestStrategy::MaCross {
            short_window,
            long_window,
        } => run_ma_cross_backtest(
            klines,
            BacktestParams {
                initial_cash: params.initial_cash,
                short_window,
                long_window,
                fee_rate: params.fee_rate,
            },
        ),
        BacktestStrategy::Grid {
            lower_price,
            upper_price,
            grid_count,
        } => run_grid_backtest(
            klines,
            GridBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                lower_price,
                upper_price,
                grid_count,
            },
        ),
        BacktestStrategy::TrendGrid {
            lower_price,
            upper_price,
            grid_count,
            trend_window,
            stop_loss_pct,
        } => run_trend_grid_backtest(
            klines,
            TrendGridBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                lower_price,
                upper_price,
                grid_count,
                trend_window,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::Turtle {
            entry_window,
            exit_window,
            unit_pct,
            atr_window,
            stop_atr,
        } => run_turtle_backtest(
            klines,
            TurtleBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                entry_window,
                exit_window,
                unit_pct,
                atr_window,
                stop_atr,
            },
        ),
        BacktestStrategy::Martingale {
            drop_pct,
            take_profit_pct,
            max_levels,
            first_order_pct,
            multiplier,
        } => run_martingale_backtest(
            klines,
            MartingaleBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                drop_pct,
                take_profit_pct,
                max_levels,
                first_order_pct,
                multiplier,
            },
        ),
        BacktestStrategy::Rsi {
            period,
            oversold,
            overbought,
            unit_pct,
            stop_loss_pct,
        } => run_rsi_backtest(
            klines,
            RsiBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                period,
                oversold,
                overbought,
                unit_pct,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::Macd {
            fast_window,
            slow_window,
            signal_window,
            unit_pct,
            stop_loss_pct,
        } => run_macd_backtest(
            klines,
            MacdBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                fast_window,
                slow_window,
                signal_window,
                unit_pct,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::Bollinger {
            period,
            std_multiplier,
            unit_pct,
            take_profit_pct,
            stop_loss_pct,
        } => run_bollinger_backtest(
            klines,
            BollingerBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                period,
                std_multiplier,
                unit_pct,
                take_profit_pct,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::VolumeSpike {
            breakout_window,
            volume_window,
            spike_ratio,
            unit_pct,
            stop_loss_pct,
        } => run_volume_spike_backtest(
            klines,
            VolumeSpikeBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                breakout_window,
                volume_window,
                spike_ratio,
                unit_pct,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::Obv {
            obv_window,
            price_window,
            unit_pct,
            take_profit_pct,
            stop_loss_pct,
        } => run_obv_backtest(
            klines,
            ObvBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                obv_window,
                price_window,
                unit_pct,
                take_profit_pct,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::Stochastic {
            k_window,
            d_window,
            oversold,
            overbought,
            unit_pct,
        } => run_stochastic_backtest(
            klines,
            StochasticBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                k_window,
                d_window,
                oversold,
                overbought,
                unit_pct,
            },
        ),
        BacktestStrategy::Cci {
            period,
            oversold,
            overbought,
            unit_pct,
            stop_loss_pct,
        } => run_cci_backtest(
            klines,
            CciBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                period,
                oversold,
                overbought,
                unit_pct,
                stop_loss_pct,
            },
        ),
        BacktestStrategy::SuperTrend {
            atr_window,
            multiplier,
            unit_pct,
            take_profit_pct,
            stop_loss_pct,
        } => run_supertrend_backtest(
            klines,
            SuperTrendBacktestParams {
                initial_cash: params.initial_cash,
                fee_rate: params.fee_rate,
                atr_window,
                multiplier,
                unit_pct,
                take_profit_pct,
                stop_loss_pct,
            },
        ),
    }
}

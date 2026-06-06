use crate::ui::palette;
use binance_tools::{
    backtest::{
        BacktestAction, BacktestResult, BacktestRunParams, BacktestStrategy, BacktestTrade,
        run_backtest,
    },
    binance::vision::{
        SUPPORTED_SPOT_KLINE_INTERVALS, VisionKline, download_spot_daily_klines_blocking,
    },
};
use chrono::{DateTime, Duration, Local, NaiveDate};
use gpui::{InteractiveElement as _, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, IndexPath, PixelsExt, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    chart::{BarChart, CandlestickChart},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};

const MIN_VISIBLE_KLINES: usize = 20;
const ZOOM_STEP: usize = 20;
const EMA_PERIODS: [usize; 4] = [7, 25, 99, 120];
const VOLUME_MA_PERIODS: [usize; 2] = [5, 10];
const MA_CROSS_STRATEGY: &str = "MA Cross";
const GRID_STRATEGY: &str = "Grid";
const TREND_GRID_STRATEGY: &str = "Trend Grid";
const TURTLE_STRATEGY: &str = "Turtle";
const MARTINGALE_STRATEGY: &str = "Martingale";
const RSI_STRATEGY: &str = "RSI";
const MACD_STRATEGY: &str = "MACD";
const BOLLINGER_STRATEGY: &str = "Bollinger Bands";
const VOLUME_SPIKE_STRATEGY: &str = "Volume Spike";
const OBV_STRATEGY: &str = "OBV";
const STOCHASTIC_STRATEGY: &str = "Stochastic";
const CCI_STRATEGY: &str = "CCI";
const SUPERTREND_STRATEGY: &str = "SuperTrend";

pub struct SpotBacktestPage {
    strategy_select: Entity<SelectState<Vec<String>>>,
    symbol_input: Entity<InputState>,
    interval_input: Entity<InputState>,
    start_input: Entity<InputState>,
    end_input: Entity<InputState>,
    short_input: Entity<InputState>,
    long_input: Entity<InputState>,
    grid_count_input: Entity<InputState>,
    trend_window_input: Entity<InputState>,
    stop_loss_input: Entity<InputState>,
    cash_input: Entity<InputState>,
    fee_input: Entity<InputState>,
    loading: bool,
    status: Option<String>,
    error: Option<String>,
    rows: usize,
    cached_files: usize,
    downloaded_files: usize,
    missing_files: usize,
    klines: Vec<VisionKline>,
    current_interval: String,
    visible_start: usize,
    visible_count: usize,
    price_chart_bounds: Option<Bounds<Pixels>>,
    drag_start_x: Option<Pixels>,
    drag_start_visible_start: usize,
    result: Option<BacktestResult>,
    _task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SpotBacktestPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let today = Local::now().date_naive();
        let default_start = today - Duration::days(120);
        let strategy_select = cx.new(|cx| {
            SelectState::new(
                backtest_strategy_options(),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let symbol_input = input(window, cx, "BTCUSDT");
        let interval_input = input(window, cx, "1d");
        let start_input = input(window, cx, &default_start.format("%Y-%m-%d").to_string());
        let end_input = input(window, cx, &today.format("%Y-%m-%d").to_string());
        let short_input = input(window, cx, "7");
        let long_input = input(window, cx, "30");
        let grid_count_input = input(window, cx, "20");
        let trend_window_input = input(window, cx, "120");
        let stop_loss_input = input(window, cx, "3");
        let cash_input = input(window, cx, "10000");
        let fee_input = input(window, cx, "0.001");
        let _subscriptions = vec![
            cx.subscribe_in(&strategy_select, window, Self::on_strategy_event),
            cx.subscribe_in(&symbol_input, window, Self::on_input_event),
            cx.subscribe_in(&interval_input, window, Self::on_input_event),
            cx.subscribe_in(&start_input, window, Self::on_input_event),
            cx.subscribe_in(&end_input, window, Self::on_input_event),
            cx.subscribe_in(&short_input, window, Self::on_input_event),
            cx.subscribe_in(&long_input, window, Self::on_input_event),
            cx.subscribe_in(&grid_count_input, window, Self::on_input_event),
            cx.subscribe_in(&trend_window_input, window, Self::on_input_event),
            cx.subscribe_in(&stop_loss_input, window, Self::on_input_event),
            cx.subscribe_in(&cash_input, window, Self::on_input_event),
            cx.subscribe_in(&fee_input, window, Self::on_input_event),
        ];

        Self {
            strategy_select,
            symbol_input,
            interval_input,
            start_input,
            end_input,
            short_input,
            long_input,
            grid_count_input,
            trend_window_input,
            stop_loss_input,
            cash_input,
            fee_input,
            loading: false,
            status: None,
            error: None,
            rows: 0,
            cached_files: 0,
            downloaded_files: 0,
            missing_files: 0,
            klines: Vec::new(),
            current_interval: "1d".to_string(),
            visible_start: 0,
            visible_count: 120,
            price_chart_bounds: None,
            drag_start_x: None,
            drag_start_visible_start: 0,
            result: None,
            _task: Task::ready(()),
            _subscriptions,
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.status = None;
            self.error = None;
            cx.notify();
        }
    }

    fn on_strategy_event(
        &mut self,
        _: &Entity<SelectState<Vec<String>>>,
        event: &SelectEvent<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(strategy) = event;
        if let Some(strategy) = strategy {
            self.apply_strategy_defaults(strategy, window, cx);
        }
        self.status = None;
        self.error = None;
        cx.notify();
    }

    fn apply_strategy_defaults(
        &mut self,
        strategy: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match strategy {
            MA_CROSS_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("7", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("30", window, cx));
            }
            GRID_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("65000", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("85000", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
            }
            TREND_GRID_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("65000", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("85000", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("120", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("3", window, cx));
            }
            TURTLE_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("10", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("25", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("14", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("2", window, cx));
            }
            MARTINGALE_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("3", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("2", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("5", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("5", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("1.5", window, cx));
            }
            RSI_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("14", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("30", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("70", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            MACD_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("12", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("26", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("9", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            BOLLINGER_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("2", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("5", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            VOLUME_SPIKE_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("2", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            OBV_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("30", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("5", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            STOCHASTIC_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("14", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("3", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("80", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
            }
            CCI_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("20", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("-100", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            SUPERTREND_STRATEGY => {
                self.short_input
                    .update(cx, |input, cx| input.set_value("10", window, cx));
                self.long_input
                    .update(cx, |input, cx| input.set_value("3", window, cx));
                self.grid_count_input
                    .update(cx, |input, cx| input.set_value("100", window, cx));
                self.trend_window_input
                    .update(cx, |input, cx| input.set_value("5", window, cx));
                self.stop_loss_input
                    .update(cx, |input, cx| input.set_value("8", window, cx));
            }
            _ => {}
        }
    }

    fn run(&mut self, cx: &mut Context<Self>) {
        let request = match self.parse_request(cx) {
            Ok(request) => request,
            Err(err) => {
                self.error = Some(err);
                self.status = None;
                cx.notify();
                return;
            }
        };

        self.loading = true;
        self.error = None;
        self.status = Some("正在从 Binance Vision 读取/补齐 K 线并回测...".to_string());
        self.result = None;
        self.klines.clear();
        cx.notify();

        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let downloaded = download_spot_daily_klines_blocking(
                        &request.symbol,
                        &request.interval,
                        request.start,
                        request.end,
                    )?;
                    let rows = downloaded.klines.len();
                    let backtest = run_backtest(&downloaded.klines, request.params)?;
                    anyhow::Ok(BacktestRunOutput {
                        rows,
                        cached_files: downloaded.cached_files,
                        downloaded_files: downloaded.downloaded_files,
                        missing_files: downloaded.missing_files,
                        klines: downloaded.klines,
                        interval: request.interval,
                        backtest,
                    })
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(output) => {
                        this.rows = output.rows;
                        this.cached_files = output.cached_files;
                        this.downloaded_files = output.downloaded_files;
                        this.missing_files = output.missing_files;
                        this.klines = output.klines;
                        this.current_interval = output.interval;
                        this.reset_visible_window();
                        this.result = Some(output.backtest);
                        this.status = Some("回测完成".to_string());
                        this.error = None;
                    }
                    Err(err) => {
                        this.error = Some(err.to_string());
                        this.status = None;
                    }
                }
                cx.notify();
            });
        });
    }

    fn parse_request(&self, cx: &mut Context<Self>) -> Result<BacktestRunRequest, String> {
        let symbol = self.symbol_input.read(cx).text().to_string();
        let interval = self.interval_input.read(cx).text().to_string();
        let start_text = self.start_input.read(cx).text().to_string();
        let end_text = self.end_input.read(cx).text().to_string();
        let short_text = self.short_input.read(cx).text().to_string();
        let long_text = self.long_input.read(cx).text().to_string();
        let grid_count_text = self.grid_count_input.read(cx).text().to_string();
        let trend_window_text = self.trend_window_input.read(cx).text().to_string();
        let stop_loss_text = self.stop_loss_input.read(cx).text().to_string();
        let cash_text = self.cash_input.read(cx).text().to_string();
        let fee_text = self.fee_input.read(cx).text().to_string();
        let interval = interval.trim().to_lowercase();
        let strategy_label = self
            .strategy_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| MA_CROSS_STRATEGY.to_string());

        if !SUPPORTED_SPOT_KLINE_INTERVALS.contains(&interval.as_str()) {
            return Err(format!(
                "周期 `{}` 不支持。可用周期：{}。120 日回测请填 `1d`，并选择 120 天日期范围。",
                interval,
                SUPPORTED_SPOT_KLINE_INTERVALS.join(", ")
            ));
        }

        Ok(BacktestRunRequest {
            symbol: symbol.trim().to_uppercase(),
            interval,
            start: parse_date(start_text.trim(), "开始日期")?,
            end: parse_date(end_text.trim(), "结束日期")?,
            params: BacktestRunParams {
                initial_cash: parse_f64(cash_text.trim(), "初始资金")?,
                fee_rate: parse_f64(fee_text.trim(), "手续费率")?,
                strategy: parse_backtest_strategy(
                    &strategy_label,
                    short_text.trim(),
                    long_text.trim(),
                    grid_count_text.trim(),
                    trend_window_text.trim(),
                    stop_loss_text.trim(),
                )?,
            },
        })
    }

    fn visible_klines(&self) -> Vec<VisionKline> {
        let count = self.visible_count.min(self.klines.len());
        let max_start = self.klines.len().saturating_sub(count);
        let start = self.visible_start.min(max_start);
        self.klines
            .iter()
            .skip(start)
            .take(count)
            .cloned()
            .collect()
    }

    fn reset_visible_window(&mut self) {
        self.visible_count = self.visible_count.clamp(
            MIN_VISIBLE_KLINES,
            self.klines.len().max(MIN_VISIBLE_KLINES),
        );
        let count = self.visible_count.min(self.klines.len());
        self.visible_start = self.klines.len().saturating_sub(count);
        self.price_chart_bounds = None;
        self.drag_start_x = None;
        self.drag_start_visible_start = self.visible_start;
    }

    fn zoom_in(&mut self, cx: &mut Context<Self>) {
        if self.klines.is_empty() {
            return;
        }
        let old_count = self.visible_count.min(self.klines.len()).max(1);
        self.visible_count = self
            .visible_count
            .saturating_sub(ZOOM_STEP)
            .max(MIN_VISIBLE_KLINES);
        let new_count = self.visible_count.min(self.klines.len()).max(1);
        let anchor = self.visible_start + old_count;
        self.visible_start = anchor
            .saturating_sub(new_count)
            .min(self.klines.len().saturating_sub(new_count));
        cx.notify();
    }

    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        if self.klines.is_empty() {
            return;
        }
        let old_count = self.visible_count.min(self.klines.len()).max(1);
        self.visible_count =
            (self.visible_count + ZOOM_STEP).min(self.klines.len().max(MIN_VISIBLE_KLINES));
        let new_count = self.visible_count.min(self.klines.len()).max(1);
        let anchor = self.visible_start + old_count;
        self.visible_start = anchor
            .saturating_sub(new_count)
            .min(self.klines.len().saturating_sub(new_count));
        cx.notify();
    }

    fn begin_drag(&mut self, position: Point<Pixels>) {
        self.drag_start_x = Some(position.x);
        self.drag_start_visible_start = self.visible_start;
    }

    fn end_drag(&mut self) {
        self.drag_start_x = None;
        self.drag_start_visible_start = self.visible_start;
    }

    fn pan_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.price_chart_bounds else {
            return;
        };
        let Some(start_x) = self.drag_start_x else {
            return;
        };
        let visible_len = self.visible_count.min(self.klines.len()).max(1);
        if bounds.size.width.as_f32() <= 0.0 {
            return;
        }
        let candle_width = bounds.size.width.as_f32() / visible_len as f32;
        let delta = ((start_x - position.x).as_f32() / candle_width).round() as isize;
        let max_start = self.klines.len().saturating_sub(visible_len);
        let next_start = if delta >= 0 {
            self.drag_start_visible_start
                .saturating_add(delta as usize)
                .min(max_start)
        } else {
            self.drag_start_visible_start
                .saturating_sub(delta.unsigned_abs())
        };

        if self.visible_start != next_start {
            self.visible_start = next_start;
            cx.notify();
        }
    }

    fn volume_high(&self) -> f64 {
        self.visible_klines()
            .iter()
            .map(|kline| kline.volume)
            .reduce(f64::max)
            .unwrap_or(0.0)
    }

    fn render_field(&self, label: &'static str, input: &Entity<InputState>) -> AnyElement {
        v_flex()
            .gap_0p5()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0x6b7280))
                    .child(label),
            )
            .child(
                div()
                    .w(px(118.))
                    .h(px(30.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(0xd9dde3))
                    .bg(rgb(0xffffff))
                    .child(Input::new(input).appearance(false)),
            )
            .into_any_element()
    }

    fn render_strategy_select(&self) -> AnyElement {
        v_flex()
            .gap_0p5()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0x6b7280))
                    .child("策略"),
            )
            .child(
                div().w(px(160.)).child(
                    Select::new(&self.strategy_select)
                        .placeholder("选择策略")
                        .menu_width(px(180.))
                        .with_size(gpui_component::Size::Small),
                ),
            )
            .into_any_element()
    }

    fn selected_strategy_label(&self, cx: &mut Context<Self>) -> String {
        self.strategy_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| MA_CROSS_STRATEGY.to_string())
    }

    fn render_metric(
        &self,
        label: &'static str,
        value: String,
        good: Option<bool>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        div()
            .min_w(px(104.))
            .px_3()
            .py_2()
            .rounded(px(6.))
            .border_1()
            .border_color(palette::border(app_theme))
            .bg(app_theme.background)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::muted(app_theme))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_semibold()
                            .text_color(match good {
                                Some(true) => app_theme.success,
                                Some(false) => app_theme.danger,
                                None => palette::text_strong(app_theme),
                            })
                            .child(value),
                    ),
            )
            .into_any_element()
    }

    fn render_trade(&self, trade: &BacktestTrade, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        h_flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(palette::border(app_theme))
            .text_size(px(12.))
            .child(div().w(px(150.)).child(format_time(trade.time)))
            .child(
                div()
                    .w(px(52.))
                    .font_semibold()
                    .text_color(if trade.action == BacktestAction::Buy {
                        app_theme.success
                    } else {
                        app_theme.danger
                    })
                    .child(action_label(trade.action)),
            )
            .child(div().w(px(110.)).child(format!("{:.6}", trade.price)))
            .child(div().w(px(120.)).child(format!("{:.6}", trade.quantity)))
            .child(div().w(px(120.)).child(format!("{:.2}", trade.equity)))
            .into_any_element()
    }

    fn render_chart_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let visible = self.visible_klines();
        let Some(range) = price_range_for(&visible) else {
            return div()
                .h(px(620.))
                .rounded(px(6.))
                .border_1()
                .border_color(palette::border(app_theme))
                .bg(palette::surface(app_theme))
                .flex()
                .items_center()
                .justify_center()
                .text_color(palette::muted(app_theme))
                .child("点击下载并回测后显示 K 线走势")
                .into_any_element();
        };

        let trades = self
            .result
            .as_ref()
            .map(|result| result.trades.clone())
            .unwrap_or_default();
        let tick_margin = (visible.len() / 8).max(1);
        let symbol = self.symbol_input.read(cx).text().to_string();
        let interval = self.current_interval.clone();
        let latest = visible.last();
        let ema_values = EMA_PERIODS
            .iter()
            .map(|period| (*period, ema_last(&visible, *period)))
            .collect::<Vec<_>>();
        let volume_ma_values = VOLUME_MA_PERIODS
            .iter()
            .map(|period| (*period, volume_ma_last(&visible, *period)))
            .collect::<Vec<_>>();
        let chart_bg = palette::surface(app_theme);
        let chart_border = palette::border(app_theme);
        let chart_axis = palette::muted(app_theme);
        let weak = cx.weak_entity();

        v_flex()
            .rounded(px(6.))
            .border_1()
            .border_color(chart_border)
            .bg(chart_bg)
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(chart_border)
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_semibold()
                                    .text_color(rgb(0xf8fafc))
                                    .child(format!("{symbol} {}", interval)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(chart_axis)
                                    .child(format!(
                                        "显示 {} / {} 根",
                                        visible.len(),
                                        self.klines.len()
                                    )),
                            )
                            .when_some(latest, |parent, latest| {
                                parent.child(div().text_size(px(12.)).text_color(chart_axis).child(
                                    format!("最新收盘 {}", format_price(latest.close_price)),
                                ))
                            })
                            .children(ema_values.iter().map(|(period, value)| {
                                indicator_label(
                                    &format!("EMA({period})"),
                                    *value,
                                    ema_color(*period),
                                )
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_size(px(12.))
                            .child(
                                Button::new("backtest-kline-zoom-in")
                                    .outline()
                                    .xsmall()
                                    .label("+")
                                    .on_click(cx.listener(|this, _, _, cx| this.zoom_in(cx))),
                            )
                            .child(
                                Button::new("backtest-kline-zoom-out")
                                    .outline()
                                    .xsmall()
                                    .label("-")
                                    .on_click(cx.listener(|this, _, _, cx| this.zoom_out(cx))),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .text_color(chart_axis)
                                    .child(
                                        div()
                                            .w(px(8.))
                                            .h(px(8.))
                                            .rounded_full()
                                            .bg(app_theme.success),
                                    )
                                    .child("买入"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .text_color(chart_axis)
                                    .child(
                                        div()
                                            .w(px(8.))
                                            .h(px(8.))
                                            .rounded_full()
                                            .bg(app_theme.danger),
                                    )
                                    .child("卖出"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .h(px(620.))
                    .p_2()
                    .gap_2()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .bg(chart_bg)
                            .child(
                                div()
                                    .absolute()
                                    .top(px(170.))
                                    .left_0()
                                    .right_0()
                                    .text_center()
                                    .text_size(px(48.))
                                    .font_semibold()
                                    .text_color(chart_axis.opacity(0.12))
                                    .child(symbol),
                            )
                            .child(
                                CandlestickChart::new(visible.clone())
                                    .x({
                                        let interval = self.current_interval.clone();
                                        move |kline| kline_tick_label(kline.open_time, &interval)
                                    })
                                    .open(|kline| kline.open_price)
                                    .high(|kline| kline.high_price)
                                    .low(|kline| kline.low_price)
                                    .close(|kline| kline.close_price)
                                    .tick_margin(tick_margin)
                                    .body_width_ratio(0.72),
                            )
                            .children(EMA_PERIODS.iter().map(|period| {
                                ema_overlay(visible.clone(), range, *period, ema_color(*period))
                                    .into_any_element()
                            }))
                            .children(trade_marker_elements(
                                visible.clone(),
                                trades,
                                range,
                                self.price_chart_bounds.map(|bounds| bounds.size),
                            ))
                            .child(
                                div()
                                    .id("backtest-kline-interaction-layer")
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full()
                                    .occlude()
                                    .bg(transparent_black())
                                    .child(
                                        canvas(
                                            {
                                                let weak = weak.clone();
                                                move |bounds, _, cx| {
                                                    _ = weak.update(cx, |this, cx| {
                                                        let should_notify = this
                                                            .price_chart_bounds
                                                            .map(|old| old.size != bounds.size)
                                                            .unwrap_or(true);
                                                        this.price_chart_bounds = Some(bounds);
                                                        if should_notify {
                                                            cx.notify();
                                                        }
                                                    });
                                                }
                                            },
                                            |_, _, _, _| {},
                                        )
                                        .size_full(),
                                    )
                                    .on_mouse_down(MouseButton::Left, {
                                        let weak = weak.clone();
                                        move |event, _, cx| {
                                            _ = weak.update(cx, |this, _| {
                                                this.begin_drag(event.position);
                                            });
                                        }
                                    })
                                    .on_mouse_move({
                                        let weak = weak.clone();
                                        move |event, _, cx| {
                                            _ = weak.update(cx, |this, cx| {
                                                this.pan_to(event.position, cx);
                                            });
                                        }
                                    })
                                    .on_mouse_up(MouseButton::Left, {
                                        let weak = weak.clone();
                                        move |_, _, cx| {
                                            _ = weak.update(cx, |this, _| {
                                                this.end_drag();
                                            });
                                        }
                                    })
                                    .on_scroll_wheel({
                                        let weak = weak.clone();
                                        move |event, _, cx| {
                                            _ = weak.update(cx, |this, cx| {
                                                let delta_y = match event.delta {
                                                    ScrollDelta::Pixels(delta) => delta.y.as_f32(),
                                                    ScrollDelta::Lines(delta) => delta.y,
                                                };
                                                if delta_y < 0.0 {
                                                    this.zoom_in(cx);
                                                } else if delta_y > 0.0 {
                                                    this.zoom_out(cx);
                                                }
                                            });
                                        }
                                    }),
                            ),
                    )
                    .child(price_axis(range, chart_axis)),
            )
            .child(
                h_flex()
                    .h(px(128.))
                    .p_2()
                    .pt_0()
                    .gap_2()
                    .border_t_1()
                    .border_color(chart_border)
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .child(
                                h_flex()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .gap_3()
                                    .items_center()
                                    .text_size(px(12.))
                                    .text_color(chart_axis)
                                    .child(format!(
                                        "VOL: {}",
                                        latest
                                            .map(|kline| format_volume(kline.volume))
                                            .unwrap_or_else(|| "0".to_string())
                                    ))
                                    .children(volume_ma_values.iter().map(|(period, value)| {
                                        volume_indicator_label(
                                            &format!("MA({period})"),
                                            *value,
                                            volume_ma_color(*period),
                                        )
                                    })),
                            )
                            .child(
                                BarChart::new(visible.clone())
                                    .x({
                                        let interval = self.current_interval.clone();
                                        move |kline| kline_tick_label(kline.open_time, &interval)
                                    })
                                    .y(|kline| kline.volume)
                                    .fill(|kline| {
                                        if kline.close_price >= kline.open_price {
                                            hsla(0.45, 0.72, 0.52, 1.0)
                                        } else {
                                            hsla(0.98, 0.84, 0.61, 1.0)
                                        }
                                    })
                                    .tick_margin(tick_margin),
                            )
                            .children(VOLUME_MA_PERIODS.iter().map(|period| {
                                volume_ma_overlay(
                                    visible.clone(),
                                    self.volume_high(),
                                    *period,
                                    volume_ma_color(*period),
                                )
                                .into_any_element()
                            })),
                    )
                    .child(volume_axis(self.volume_high(), chart_axis)),
            )
            .into_any_element()
    }
}

impl Render for SpotBacktestPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self
            .result
            .as_ref()
            .map(|result| {
                vec![
                    self.render_metric("K 线条数", self.rows.to_string(), None, cx),
                    self.render_metric("缓存文件", self.cached_files.to_string(), None, cx),
                    self.render_metric("下载文件", self.downloaded_files.to_string(), None, cx),
                    self.render_metric("缺失文件", self.missing_files.to_string(), None, cx),
                    self.render_metric("初始资金", format!("{:.2}", result.initial_cash), None, cx),
                    self.render_metric(
                        "最终权益",
                        format!("{:.2}", result.final_equity),
                        Some(result.final_equity >= result.initial_cash),
                        cx,
                    ),
                    self.render_metric(
                        "收益率",
                        format!("{:.2}%", result.return_pct),
                        Some(result.return_pct >= 0.0),
                        cx,
                    ),
                    self.render_metric(
                        "最大回撤",
                        format!("{:.2}%", result.max_drawdown_pct),
                        Some(false),
                        cx,
                    ),
                    self.render_metric("交易次数", result.trade_count.to_string(), None, cx),
                    self.render_metric("胜率", format!("{:.2}%", result.win_rate_pct), None, cx),
                ]
            })
            .unwrap_or_default();
        let trade_rows = self
            .result
            .as_ref()
            .map(|result| {
                result
                    .trades
                    .iter()
                    .map(|trade| self.render_trade(trade, cx))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let chart_panel = self.render_chart_panel(cx);
        let selected_strategy = self.selected_strategy_label(cx);
        let is_grid_strategy =
            selected_strategy == GRID_STRATEGY || selected_strategy == TREND_GRID_STRATEGY;
        let uses_five_strategy_params = uses_five_strategy_params(&selected_strategy);
        let uses_third_strategy_param = is_grid_strategy || uses_five_strategy_params;
        let app_theme = cx.theme();
        let status = self
            .error
            .as_ref()
            .map(|error| {
                div()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(app_theme.danger.opacity(0.12))
                    .text_color(app_theme.danger)
                    .child(error.clone())
                    .into_any_element()
            })
            .or_else(|| {
                self.status.as_ref().map(|status| {
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(app_theme.muted.opacity(0.12))
                        .text_color(palette::muted(app_theme))
                        .child(status.clone())
                        .into_any_element()
                })
            });

        v_flex()
            .size_full()
            .gap_2()
            .px_4()
            .py_2()
            .overflow_hidden()
            .child(
                div()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .px_3()
                    .py_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_size(px(16.)).font_semibold().child("现货回测"))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(app_theme))
                                            .child("从本地回测数据包读取 K 线，缺失日期自动从 Binance Vision 补齐。"),
                                    ),
                            )
                            .child(
                                Button::new("run-backtest")
                                    .primary()
                                    .small()
                                    .label(if self.loading { "回测中" } else { "下载并回测" })
                                    .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .px_3()
                    .py_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_end()
                            .flex_wrap()
                            .child(self.render_strategy_select())
                            .child(self.render_field("交易对", &self.symbol_input))
                            .child(self.render_field("周期", &self.interval_input))
                            .child(self.render_field("开始日期", &self.start_input))
                            .child(self.render_field("结束日期", &self.end_input))
                            .child(self.render_field(
                                strategy_param_label(&selected_strategy, 0),
                                &self.short_input,
                            ))
                            .child(self.render_field(
                                strategy_param_label(&selected_strategy, 1),
                                &self.long_input,
                            ))
                            .when(uses_third_strategy_param, |parent| {
                                parent.child(self.render_field(
                                    strategy_param_label(&selected_strategy, 2),
                                    &self.grid_count_input,
                                ))
                            })
                            .when(uses_five_strategy_params, |parent| {
                                parent.child(self.render_field(
                                    strategy_param_label(&selected_strategy, 3),
                                    &self.trend_window_input,
                                ))
                            })
                            .when(uses_five_strategy_params, |parent| {
                                parent.child(self.render_field(
                                    strategy_param_label(&selected_strategy, 4),
                                    &self.stop_loss_input,
                                ))
                            })
                            .child(self.render_field("初始资金", &self.cash_input))
                            .child(self.render_field("手续费率", &self.fee_input)),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child("K 线图支持鼠标拖动平移、滚轮缩放，也可以用 + / - 调整显示根数。"),
            )
            .when_some(status, |parent, status| parent.child(status))
            .child(chart_panel)
            .child(h_flex().gap_2().flex_wrap().children(metrics))
            .child(
                div()
                    .flex_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .size_full()
                            .child(
                                h_flex()
                                    .items_center()
                                    .h(px(34.))
                                    .px_3()
                                    .border_b_1()
                                    .border_color(palette::border(app_theme))
                                    .font_semibold()
                                    .child("交易记录"),
                            )
                            .child(
                                div().flex_1().overflow_hidden().child(
                                    v_flex().size_full().overflow_y_scrollbar().map(|list| {
                                        if trade_rows.is_empty() {
                                            list.child(
                                                div()
                                                    .p_4()
                                                    .text_color(palette::muted(app_theme))
                                                    .child("点击下载并回测后查看交易记录"),
                                            )
                                        } else {
                                            list.children(trade_rows)
                                        }
                                    }),
                                ),
                            ),
                    ),
            )
    }
}

struct BacktestRunRequest {
    symbol: String,
    interval: String,
    start: NaiveDate,
    end: NaiveDate,
    params: BacktestRunParams,
}

struct BacktestRunOutput {
    rows: usize,
    cached_files: usize,
    downloaded_files: usize,
    missing_files: usize,
    klines: Vec<VisionKline>,
    interval: String,
    backtest: BacktestResult,
}

#[derive(Clone, Copy)]
struct BacktestPriceRange {
    high: f64,
    low: f64,
    mid: f64,
}

fn input(
    window: &mut Window,
    cx: &mut Context<SpotBacktestPage>,
    value: &str,
) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}

fn backtest_strategy_options() -> Vec<String> {
    vec![
        MA_CROSS_STRATEGY.to_string(),
        GRID_STRATEGY.to_string(),
        TREND_GRID_STRATEGY.to_string(),
        TURTLE_STRATEGY.to_string(),
        MARTINGALE_STRATEGY.to_string(),
        RSI_STRATEGY.to_string(),
        MACD_STRATEGY.to_string(),
        BOLLINGER_STRATEGY.to_string(),
        VOLUME_SPIKE_STRATEGY.to_string(),
        OBV_STRATEGY.to_string(),
        STOCHASTIC_STRATEGY.to_string(),
        CCI_STRATEGY.to_string(),
        SUPERTREND_STRATEGY.to_string(),
    ]
}

fn parse_backtest_strategy(
    strategy: &str,
    param_a: &str,
    param_b: &str,
    param_c: &str,
    param_d: &str,
    param_e: &str,
) -> Result<BacktestStrategy, String> {
    match strategy {
        MA_CROSS_STRATEGY => Ok(BacktestStrategy::MaCross {
            short_window: parse_usize(param_a, "短均线")?,
            long_window: parse_usize(param_b, "长均线")?,
        }),
        GRID_STRATEGY => Ok(BacktestStrategy::Grid {
            lower_price: parse_f64(param_a, "网格下限")?,
            upper_price: parse_f64(param_b, "网格上限")?,
            grid_count: parse_usize(param_c, "网格数量")?,
        }),
        TREND_GRID_STRATEGY => Ok(BacktestStrategy::TrendGrid {
            lower_price: parse_f64(param_a, "网格下限")?,
            upper_price: parse_f64(param_b, "网格上限")?,
            grid_count: parse_usize(param_c, "网格数量")?,
            trend_window: parse_usize(param_d, "趋势 EMA")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        TURTLE_STRATEGY => Ok(BacktestStrategy::Turtle {
            entry_window: parse_usize(param_a, "突破周期")?,
            exit_window: parse_usize(param_b, "退出周期")?,
            unit_pct: parse_f64(param_c, "单次仓位")?,
            atr_window: parse_usize(param_d, "ATR 周期")?,
            stop_atr: parse_f64(param_e, "止损 ATR")?,
        }),
        MARTINGALE_STRATEGY => Ok(BacktestStrategy::Martingale {
            drop_pct: parse_f64(param_a, "加仓跌幅")?,
            take_profit_pct: parse_f64(param_b, "止盈比例")?,
            max_levels: parse_usize(param_c, "最大层数")?,
            first_order_pct: parse_f64(param_d, "首单资金")?,
            multiplier: parse_f64(param_e, "加仓倍数")?,
        }),
        RSI_STRATEGY => Ok(BacktestStrategy::Rsi {
            period: parse_usize(param_a, "RSI周期")?,
            oversold: parse_f64(param_b, "超卖")?,
            unit_pct: parse_f64(param_c, "单次仓位")?,
            overbought: parse_f64(param_d, "超买")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        MACD_STRATEGY => Ok(BacktestStrategy::Macd {
            fast_window: parse_usize(param_a, "快线 EMA")?,
            slow_window: parse_usize(param_b, "慢线 EMA")?,
            signal_window: parse_usize(param_c, "信号线")?,
            unit_pct: parse_f64(param_d, "单次仓位")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        BOLLINGER_STRATEGY => Ok(BacktestStrategy::Bollinger {
            period: parse_usize(param_a, "周期")?,
            std_multiplier: parse_f64(param_b, "标准差倍数")?,
            unit_pct: parse_f64(param_c, "单次仓位")?,
            take_profit_pct: parse_f64(param_d, "止盈比例")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        VOLUME_SPIKE_STRATEGY => Ok(BacktestStrategy::VolumeSpike {
            breakout_window: parse_usize(param_a, "突破周期")?,
            volume_window: parse_usize(param_b, "成交量均线")?,
            spike_ratio: parse_f64(param_c, "放量倍数")?,
            unit_pct: parse_f64(param_d, "单次仓位")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        OBV_STRATEGY => Ok(BacktestStrategy::Obv {
            obv_window: parse_usize(param_a, "OBV均线")?,
            price_window: parse_usize(param_b, "价格均线")?,
            unit_pct: parse_f64(param_c, "单次仓位")?,
            take_profit_pct: parse_f64(param_d, "止盈比例")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        STOCHASTIC_STRATEGY => Ok(BacktestStrategy::Stochastic {
            k_window: parse_usize(param_a, "K周期")?,
            d_window: parse_usize(param_b, "D周期")?,
            oversold: parse_f64(param_c, "超卖")?,
            overbought: parse_f64(param_d, "超买")?,
            unit_pct: parse_f64(param_e, "单次仓位")?,
        }),
        CCI_STRATEGY => Ok(BacktestStrategy::Cci {
            period: parse_usize(param_a, "CCI周期")?,
            oversold: parse_f64(param_b, "超卖")?,
            overbought: parse_f64(param_c, "超买")?,
            unit_pct: parse_f64(param_d, "单次仓位")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        SUPERTREND_STRATEGY => Ok(BacktestStrategy::SuperTrend {
            atr_window: parse_usize(param_a, "ATR周期")?,
            multiplier: parse_f64(param_b, "ATR倍数")?,
            unit_pct: parse_f64(param_c, "单次仓位")?,
            take_profit_pct: parse_f64(param_d, "止盈比例")?,
            stop_loss_pct: parse_f64(param_e, "止损比例")?,
        }),
        other => Err(format!("暂不支持回测策略：{other}")),
    }
}

fn uses_five_strategy_params(strategy: &str) -> bool {
    matches!(
        strategy,
        TREND_GRID_STRATEGY
            | TURTLE_STRATEGY
            | MARTINGALE_STRATEGY
            | RSI_STRATEGY
            | MACD_STRATEGY
            | BOLLINGER_STRATEGY
            | VOLUME_SPIKE_STRATEGY
            | OBV_STRATEGY
            | STOCHASTIC_STRATEGY
            | CCI_STRATEGY
            | SUPERTREND_STRATEGY
    )
}

fn strategy_param_label(strategy: &str, index: usize) -> &'static str {
    match (strategy, index) {
        (GRID_STRATEGY | TREND_GRID_STRATEGY, 0) => "网格下限",
        (GRID_STRATEGY | TREND_GRID_STRATEGY, 1) => "网格上限",
        (GRID_STRATEGY | TREND_GRID_STRATEGY, 2) => "网格数量",
        (TREND_GRID_STRATEGY, 3) => "趋势 EMA",
        (TREND_GRID_STRATEGY, 4) => "止损%",
        (TURTLE_STRATEGY, 0) => "突破周期",
        (TURTLE_STRATEGY, 1) => "退出周期",
        (TURTLE_STRATEGY, 2) => "单次仓位%",
        (TURTLE_STRATEGY, 3) => "ATR周期",
        (TURTLE_STRATEGY, 4) => "止损ATR",
        (MARTINGALE_STRATEGY, 0) => "加仓跌幅%",
        (MARTINGALE_STRATEGY, 1) => "止盈%",
        (MARTINGALE_STRATEGY, 2) => "最大层数",
        (MARTINGALE_STRATEGY, 3) => "首单资金%",
        (MARTINGALE_STRATEGY, 4) => "加仓倍数",
        (RSI_STRATEGY, 0) => "RSI周期",
        (RSI_STRATEGY, 1) => "超卖",
        (RSI_STRATEGY, 2) => "单次仓位%",
        (RSI_STRATEGY, 3) => "超买",
        (RSI_STRATEGY, 4) => "止损%",
        (MACD_STRATEGY, 0) => "快线 EMA",
        (MACD_STRATEGY, 1) => "慢线 EMA",
        (MACD_STRATEGY, 2) => "信号线",
        (MACD_STRATEGY, 3) => "单次仓位%",
        (MACD_STRATEGY, 4) => "止损%",
        (BOLLINGER_STRATEGY, 0) => "周期",
        (BOLLINGER_STRATEGY, 1) => "标准差倍数",
        (BOLLINGER_STRATEGY, 2) => "单次仓位%",
        (BOLLINGER_STRATEGY, 3) => "止盈%",
        (BOLLINGER_STRATEGY, 4) => "止损%",
        (VOLUME_SPIKE_STRATEGY, 0) => "突破周期",
        (VOLUME_SPIKE_STRATEGY, 1) => "成交量均线",
        (VOLUME_SPIKE_STRATEGY, 2) => "放量倍数",
        (VOLUME_SPIKE_STRATEGY, 3) => "单次仓位%",
        (VOLUME_SPIKE_STRATEGY, 4) => "止损%",
        (OBV_STRATEGY, 0) => "OBV均线",
        (OBV_STRATEGY, 1) => "价格均线",
        (OBV_STRATEGY, 2) => "单次仓位%",
        (OBV_STRATEGY, 3) => "止盈%",
        (OBV_STRATEGY, 4) => "止损%",
        (STOCHASTIC_STRATEGY, 0) => "K周期",
        (STOCHASTIC_STRATEGY, 1) => "D周期",
        (STOCHASTIC_STRATEGY, 2) => "超卖",
        (STOCHASTIC_STRATEGY, 3) => "超买",
        (STOCHASTIC_STRATEGY, 4) => "单次仓位%",
        (CCI_STRATEGY, 0) => "CCI周期",
        (CCI_STRATEGY, 1) => "超卖",
        (CCI_STRATEGY, 2) => "超买",
        (CCI_STRATEGY, 3) => "单次仓位%",
        (CCI_STRATEGY, 4) => "止损%",
        (SUPERTREND_STRATEGY, 0) => "ATR周期",
        (SUPERTREND_STRATEGY, 1) => "ATR倍数",
        (SUPERTREND_STRATEGY, 2) => "单次仓位%",
        (SUPERTREND_STRATEGY, 3) => "止盈%",
        (SUPERTREND_STRATEGY, 4) => "止损%",
        (_, 0) => "短均线",
        (_, 1) => "长均线",
        _ => "参数",
    }
}

fn parse_date(value: &str, label: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("{label}格式错误，请使用 YYYY-MM-DD"))
}

fn parse_usize(value: &str, label: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{label}必须是整数"))
}

fn parse_f64(value: &str, label: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("{label}必须是数字"))
}

fn action_label(action: BacktestAction) -> &'static str {
    match action {
        BacktestAction::Buy => "买入",
        BacktestAction::Sell => "卖出",
    }
}

fn price_range_for(klines: &[VisionKline]) -> Option<BacktestPriceRange> {
    let high = klines
        .iter()
        .map(|kline| kline.high_price)
        .reduce(f64::max)?;
    let low = klines
        .iter()
        .map(|kline| kline.low_price)
        .reduce(f64::min)?;
    Some(BacktestPriceRange {
        high,
        low,
        mid: (high + low) / 2.0,
    })
}

fn backtest_chart_axis() -> Hsla {
    hsla(0.61, 0.16, 0.68, 1.0)
}

fn ema_color(period: usize) -> Hsla {
    match period {
        7 => hsla(0.13, 0.86, 0.52, 1.0),
        25 => hsla(0.88, 0.76, 0.58, 1.0),
        99 => hsla(0.72, 0.56, 0.62, 1.0),
        120 => hsla(0.36, 0.68, 0.54, 1.0),
        _ => backtest_chart_axis(),
    }
}

fn volume_ma_color(period: usize) -> Hsla {
    match period {
        5 => hsla(0.13, 0.86, 0.52, 1.0),
        10 => hsla(0.72, 0.56, 0.62, 1.0),
        _ => backtest_chart_axis(),
    }
}

fn indicator_label(label: &str, value: Option<f64>, color: Hsla) -> AnyElement {
    div()
        .text_size(px(12.))
        .font_medium()
        .text_color(color)
        .child(match value {
            Some(value) => format!("{label}: {}", format_price(value)),
            None => format!("{label}: --"),
        })
        .into_any_element()
}

fn volume_indicator_label(label: &str, value: Option<f64>, color: Hsla) -> AnyElement {
    div()
        .text_size(px(12.))
        .font_medium()
        .text_color(color)
        .child(match value {
            Some(value) => format!("{label}: {}", format_volume(value)),
            None => format!("{label}: --"),
        })
        .into_any_element()
}

fn ema_last(klines: &[VisionKline], period: usize) -> Option<f64> {
    ema_series(klines, period).into_iter().flatten().last()
}

fn ema_series(klines: &[VisionKline], period: usize) -> Vec<Option<f64>> {
    if klines.is_empty() || period == 0 {
        return Vec::new();
    }

    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut ema = 0.0;
    klines
        .iter()
        .enumerate()
        .map(|(index, kline)| {
            ema = if index == 0 {
                kline.close_price
            } else {
                (kline.close_price - ema) * multiplier + ema
            };

            if index + 1 >= period { Some(ema) } else { None }
        })
        .collect()
}

fn volume_ma_last(klines: &[VisionKline], period: usize) -> Option<f64> {
    moving_average_last(klines.iter().map(|kline| kline.volume), period)
}

fn moving_average_last(values: impl Iterator<Item = f64>, period: usize) -> Option<f64> {
    if period == 0 {
        return None;
    }

    let values = values.collect::<Vec<_>>();
    if values.len() < period {
        return None;
    }
    Some(values[values.len() - period..].iter().sum::<f64>() / period as f64)
}

fn ema_overlay(
    klines: Vec<VisionKline>,
    range: BacktestPriceRange,
    period: usize,
    color: Hsla,
) -> impl IntoElement {
    let series = ema_series(&klines, period);

    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if klines.len() <= 1 || range.high <= range.low {
                return;
            }

            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32();
            if width <= 0.0 || height <= 0.0 {
                return;
            }

            let mut builder = PathBuilder::stroke(px(1.55));
            let mut drawing = false;
            for (index, value) in series.iter().enumerate() {
                let Some(value) = value else {
                    drawing = false;
                    continue;
                };
                let x = index as f32 / (klines.len() - 1) as f32 * width;
                let y = ((range.high - value) / (range.high - range.low) * height as f64)
                    .clamp(0.0, height as f64) as f32;
                let point = point(bounds.left() + px(x), bounds.top() + px(y));
                if drawing {
                    builder.line_to(point);
                } else {
                    builder.move_to(point);
                    drawing = true;
                }
            }

            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn volume_ma_overlay(
    klines: Vec<VisionKline>,
    high: f64,
    period: usize,
    color: Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if klines.len() <= 1 || high <= 0.0 || period == 0 {
                return;
            }

            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32();
            if width <= 0.0 || height <= 0.0 {
                return;
            }

            let mut builder = PathBuilder::stroke(px(1.2));
            let mut drawing = false;
            for index in 0..klines.len() {
                if index + 1 < period {
                    drawing = false;
                    continue;
                }

                let volume = klines[index + 1 - period..=index]
                    .iter()
                    .map(|kline| kline.volume)
                    .sum::<f64>()
                    / period as f64;
                let x = index as f32 / (klines.len() - 1) as f32 * width;
                let volume_height =
                    (volume / high * height as f64).clamp(0.0, height as f64) as f32;
                let y = height - volume_height;
                let point = point(bounds.left() + px(x), bounds.top() + px(y));
                if drawing {
                    builder.line_to(point);
                } else {
                    builder.move_to(point);
                    drawing = true;
                }
            }

            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn trade_marker_elements(
    klines: Vec<VisionKline>,
    trades: Vec<BacktestTrade>,
    range: BacktestPriceRange,
    chart_size: Option<Size<Pixels>>,
) -> Vec<AnyElement> {
    let Some(chart_size) = chart_size else {
        return Vec::new();
    };
    if klines.is_empty() || range.high <= range.low {
        return Vec::new();
    }

    let markers = build_trade_markers(&trades);
    let width = chart_size.width.as_f32();
    let height = chart_size.height.as_f32();
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }

    let mut elements = Vec::new();
    for marker in markers {
        let Some(index) = klines
            .iter()
            .position(|kline| kline.open_time == marker.time)
        else {
            continue;
        };
        let kline = &klines[index];
        let x = if klines.len() <= 1 {
            width / 2.0
        } else {
            index as f32 / (klines.len() - 1) as f32 * width
        };
        let anchor_price = if marker.action == BacktestAction::Buy {
            kline.low_price
        } else {
            kline.high_price
        };
        let anchor_y = ((range.high - anchor_price) / (range.high - range.low) * height as f64)
            .clamp(0.0, height as f64) as f32;
        let y = if marker.action == BacktestAction::Buy {
            (anchor_y + 13.0).min(height - 16.0)
        } else {
            (anchor_y - 23.0).max(2.0)
        };
        let badge_color = if marker.action == BacktestAction::Buy {
            hsla(0.45, 0.82, 0.50, 1.0)
        } else {
            hsla(0.98, 0.84, 0.61, 1.0)
        };
        let label = if marker.action == BacktestAction::Buy {
            "B"
        } else {
            "S"
        };

        elements.push(
            div()
                .absolute()
                .left(px((x - 7.0).clamp(0.0, width - 14.0)))
                .top(px(y))
                .w(px(14.))
                .h(px(14.))
                .rounded(px(4.))
                .border_1()
                .border_color(rgb(0xf8fafc))
                .bg(badge_color)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.))
                .font_semibold()
                .text_color(rgb(0xffffff))
                .child(label)
                .into_any_element(),
        );

        if let Some(pnl) = marker.label {
            let is_loss = pnl.starts_with('-');
            let pnl_y = if marker.action == BacktestAction::Buy {
                (y + 16.0).min(height - 14.0)
            } else {
                (y - 15.0).max(0.0)
            };
            elements.push(
                div()
                    .absolute()
                    .left(px((x + 9.0).clamp(0.0, width - 54.0)))
                    .top(px(pnl_y))
                    .text_size(px(10.))
                    .font_medium()
                    .text_color(if is_loss {
                        hsla(0.98, 0.84, 0.61, 1.0)
                    } else {
                        hsla(0.45, 0.82, 0.50, 1.0)
                    })
                    .child(pnl)
                    .into_any_element(),
            );
        }
    }

    elements
}

#[derive(Clone)]
struct TradeMarker {
    time: i64,
    action: BacktestAction,
    label: Option<String>,
}

fn build_trade_markers(trades: &[BacktestTrade]) -> Vec<TradeMarker> {
    let mut markers = Vec::new();
    let mut entry_equity = None::<f64>;

    for trade in trades {
        match trade.action {
            BacktestAction::Buy => {
                entry_equity = Some(trade.equity);
                markers.push(TradeMarker {
                    time: trade.time,
                    action: trade.action,
                    label: None,
                });
            }
            BacktestAction::Sell => {
                let label = entry_equity.and_then(|entry| {
                    if entry.abs() > f64::EPSILON {
                        Some(format!("{:+.2}%", (trade.equity / entry - 1.0) * 100.0))
                    } else {
                        None
                    }
                });
                entry_equity = None;
                markers.push(TradeMarker {
                    time: trade.time,
                    action: trade.action,
                    label,
                });
            }
        }
    }

    markers
}

fn price_axis(range: BacktestPriceRange, muted_color: Hsla) -> impl IntoElement {
    v_flex()
        .h_full()
        .w(px(88.))
        .justify_between()
        .items_end()
        .text_size(px(12.))
        .text_color(muted_color)
        .py_3()
        .child(format_price(range.high))
        .child(format_price(range.mid))
        .child(format_price(range.low))
}

fn volume_axis(high: f64, muted_color: Hsla) -> impl IntoElement {
    v_flex()
        .h_full()
        .w(px(88.))
        .justify_between()
        .items_end()
        .text_size(px(12.))
        .text_color(muted_color)
        .py_3()
        .child(format_volume(high))
        .child(format_volume(high / 2.0))
        .child("0")
}

fn format_time(timestamp_ms: i64) -> String {
    DateTime::from_timestamp_millis(timestamp_ms)
        .map(|time| {
            time.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn kline_tick_label(timestamp_ms: i64, interval: &str) -> String {
    DateTime::from_timestamp_millis(timestamp_ms)
        .map(|time| {
            let time = time.with_timezone(&Local);
            if is_intraday_interval(interval) {
                time.format("%m/%d %H:%M").to_string()
            } else {
                time.format("%m/%d").to_string()
            }
        })
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn is_intraday_interval(interval: &str) -> bool {
    interval.ends_with('s') || interval.ends_with('m') || interval.ends_with('h')
}

fn format_price(value: f64) -> String {
    if value >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.8}")
    }
}

fn format_volume(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else {
        format!("{value:.2}")
    }
}

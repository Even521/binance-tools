use crate::ui::palette;
use binance_tools::binance::{BinanceSettings, spot::DailyMaSignal};
use gpui::{EventEmitter, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

pub struct DailyMaSignalPage {
    settings: BinanceSettings,
    days: u16,
    days_input: Entity<InputState>,
    table: Entity<TableState<DailyMaSignalTableDelegate>>,
    error: Option<String>,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
pub enum DailyMaSignalEvent {
    OpenKline(String),
}

impl DailyMaSignalPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let days = 120;
        let days_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("日数")
                .default_value(days.to_string())
        });
        let page = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(DailyMaSignalTableDelegate::new(page), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let _subscriptions = vec![
            cx.subscribe_in(&days_input, window, Self::on_days_input_event),
            cx.subscribe_in(&days_input, window, Self::on_days_step_event),
        ];

        Self {
            settings: BinanceSettings::production(),
            days,
            days_input,
            table,
            error: None,
            _load_task: Task::ready(()),
            _subscriptions,
        }
        .load_cached(cx)
    }

    fn load_cached(mut self, cx: &mut Context<Self>) -> Self {
        self.load_cached_signals(cx);
        self
    }

    fn open_kline(&mut self, symbol: String, cx: &mut Context<Self>) {
        cx.emit(DailyMaSignalEvent::OpenKline(symbol));
    }

    fn on_days_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            if let Ok(days) = input.read(cx).value().parse::<u16>() {
                self.days = days.clamp(1, 1000);
                cx.notify();
            }
        }
    }

    fn on_days_step_event(
        &mut self,
        input: &Entity<InputState>,
        event: &NumberInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next_days = match event {
            NumberInputEvent::Step(StepAction::Decrement) => self.days.saturating_sub(1).max(1),
            NumberInputEvent::Step(StepAction::Increment) => self.days.saturating_add(1).min(1000),
        };
        self.days = next_days;
        input.update(cx, |input, cx| {
            input.set_value(next_days.to_string(), window, cx);
        });
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let days = self.days.clamp(1, 1000);
        self.error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(days);
            table.refresh(cx);
        });

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::spot::load_or_fetch_usdt_daily_ma_signals_blocking(
                        settings, days,
                    )
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(signals) => {
                        this.error = None;
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_signals(signals, days);
                            table.refresh(cx);
                        });
                    }
                    Err(err) => {
                        this.error = Some(err.to_string());
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_error();
                            table.refresh(cx);
                        });
                    }
                }
                cx.notify();
            });
        });
    }

    fn load_cached_signals(&mut self, cx: &mut Context<Self>) {
        let days = self.days.clamp(1, 1000);
        self.error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(days);
            table.refresh(cx);
        });

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::spot::load_cached_usdt_daily_ma_signals_blocking(days)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(signals) => {
                        this.error = None;
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_signals(signals, days);
                            table.refresh(cx);
                        });
                    }
                    Err(err) => {
                        this.error = Some(err.to_string());
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_error();
                            table.refresh(cx);
                        });
                    }
                }
                cx.notify();
            });
        });
    }
}

impl Render for DailyMaSignalPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.table.read(cx).delegate();
        let signal_count = delegate.signals.len();
        let loading = delegate.loading;

        v_flex()
            .gap_3()
            .size_full()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .rounded(px(8.))
                    .bg(palette::surface_strong(cx.theme()))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_size(px(16.)).font_semibold().child("日均线信号"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(cx.theme()))
                                    .child(format!(
                                        "筛选 USDT 现货交易对，计算当前价相对 {} 日均线的偏离，当前 {} 条",
                                        self.days, signal_count
                                    )),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_size(px(12.)).child("均线天数"))
                            .child(
                                div()
                                    .w(px(130.))
                                    .child(NumberInput::new(&self.days_input).small()),
                            )
                            .child(
                                Button::new("daily-ma-refresh")
                                    .primary()
                                    .xsmall()
                                    .label(if loading { "查询中" } else { "查询信号" })
                                    .disabled(loading)
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(cx.theme().danger.opacity(0.12))
                        .text_color(cx.theme().danger_foreground.opacity(0.9))
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_h(px(420.))
                    .w_full()
                    .child(
                        div().flex_1().size_full().overflow_hidden().child(
                            DataTable::new(&self.table)
                                .stripe(true)
                                .bordered(true)
                                .scrollbar_visible(true, true),
                        ),
                    ),
            )
    }
}

impl EventEmitter<DailyMaSignalEvent> for DailyMaSignalPage {}

#[derive(Clone)]
struct DailyMaSignalTableDelegate {
    columns: Vec<Column>,
    signals: Vec<DailyMaSignal>,
    loading: bool,
    days: u16,
    page: WeakEntity<DailyMaSignalPage>,
}

impl DailyMaSignalTableDelegate {
    fn new(page: WeakEntity<DailyMaSignalPage>) -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "Symbol")
                    .width(px(138.))
                    .fixed_left()
                    .sortable(),
                Column::new("base_asset", "Base").width(px(84.)).sortable(),
                Column::new("current_price", "当前价")
                    .width(px(106.))
                    .sortable(),
                Column::new("average_price", "均线价")
                    .width(px(106.))
                    .sortable(),
                Column::new("difference_percent", "偏离%")
                    .width(px(92.))
                    .sortable(),
                Column::new("days", "天数").width(px(64.)).sortable(),
                Column::new("samples", "样本数").width(px(74.)).sortable(),
            ],
            signals: Vec::new(),
            loading: false,
            days: 120,
            page,
        }
    }

    fn set_loading(&mut self, days: u16) {
        self.loading = true;
        self.days = days;
    }

    fn set_signals(&mut self, signals: Vec<DailyMaSignal>, days: u16) {
        self.signals = signals;
        self.days = days;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.signals.clear();
        self.loading = false;
    }

    fn cell(value: impl Into<SharedString>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px_1()
            .text_size(px(11.))
            .child(value.into())
            .into_any_element()
    }

    fn price_cell(value: f64) -> AnyElement {
        Self::cell(format!("{value:.8}"))
    }

    fn percent_cell(value: f64) -> AnyElement {
        Self::cell(format!("{value:.2}%"))
    }

    fn symbol_cell(&self, row_ix: usize, symbol: String) -> AnyElement {
        let page = self.page.clone();
        let open_symbol = symbol.clone();

        h_flex()
            .id(("kline-chart-cell", row_ix))
            .size_full()
            .items_center()
            .justify_between()
            .gap_1()
            .px_1()
            .text_size(px(11.))
            .child(div().min_w_0().flex_1().child(symbol))
            .child(
                Button::new(("kline-chart", row_ix))
                    .ghost()
                    .xsmall()
                    .w(px(22.))
                    .h(px(18.))
                    .tooltip("K 线图")
                    .child(candlestick_icon(hsla(0.61, 0.08, 0.55, 1.0)))
                    .on_click(move |_, _, cx| {
                        _ = page.update(cx, |this, cx| {
                            this.open_kline(open_symbol.clone(), cx);
                        });
                    }),
            )
            .into_any_element()
    }
}

fn candlestick_icon(color: Hsla) -> AnyElement {
    div()
        .relative()
        .w(px(16.))
        .h(px(14.))
        .child(
            div()
                .absolute()
                .left(px(4.))
                .top(px(0.))
                .w(px(1.5))
                .h(px(14.))
                .bg(color),
        )
        .child(
            div()
                .absolute()
                .left(px(2.))
                .top(px(3.))
                .w(px(5.))
                .h(px(8.))
                .border_1()
                .border_color(color)
                .rounded(px(1.5)),
        )
        .child(
            div()
                .absolute()
                .left(px(11.))
                .top(px(1.))
                .w(px(1.5))
                .h(px(12.))
                .bg(color),
        )
        .child(
            div()
                .absolute()
                .left(px(9.))
                .top(px(4.))
                .w(px(5.))
                .h(px(6.))
                .border_1()
                .border_color(color)
                .rounded(px(1.5)),
        )
        .into_any_element()
}

impl TableDelegate for DailyMaSignalTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.signals.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(signal) = self.signals.get(row_ix) else {
            return Self::cell("");
        };
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "symbol" => self.symbol_cell(row_ix, signal.symbol.clone()),
            "base_asset" => Self::cell(signal.base_asset.clone()),
            "current_price" => Self::price_cell(signal.current_price),
            "average_price" => Self::price_cell(signal.average_price),
            "difference_percent" => Self::percent_cell(signal.difference_percent),
            "days" => Self::cell(signal.days.to_string()),
            "samples" => Self::cell(signal.samples.to_string()),
            _ => Self::cell(""),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let descending = matches!(sort, ColumnSort::Descending);
        let key = self.columns[col_ix].key.to_string();

        self.signals.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "symbol" => a.symbol.cmp(&b.symbol),
                "base_asset" => a.base_asset.cmp(&b.base_asset),
                "current_price" => a
                    .current_price
                    .partial_cmp(&b.current_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "average_price" => a
                    .average_price
                    .partial_cmp(&b.average_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "difference_percent" => a
                    .difference_percent
                    .partial_cmp(&b.difference_percent)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "days" => a.days.cmp(&b.days),
                "samples" => a.samples.cmp(&b.samples),
                _ => a.symbol.cmp(&b.symbol),
            };
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }
}

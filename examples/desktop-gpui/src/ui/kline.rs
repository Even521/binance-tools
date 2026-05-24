use crate::ui::palette;
use binance_tools::binance::{BinanceSettings, alpha::AlphaDailyKline, spot::SpotDailyKline};
use gpui::{InteractiveElement as _, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, PixelsExt, Sizable, StyledExt,
    button::Button,
    chart::{BarChart, CandlestickChart},
    h_flex, v_flex,
};

const MIN_PRICE_CHART_HEIGHT: f32 = 360.0;
const VOLUME_CHART_HEIGHT: f32 = 116.0;
const MIN_VISIBLE_KLINES: usize = 20;
const ZOOM_STEP: usize = 20;

pub struct KlineCandlestickPage {
    symbol: String,
    source: KlineSource,
    klines: Vec<KlineData>,
    visible_start: usize,
    visible_count: usize,
    hover_index: Option<usize>,
    hover_point: Option<Point<Pixels>>,
    price_chart_bounds: Option<Bounds<Pixels>>,
    drag_start_x: Option<Pixels>,
    drag_start_visible_start: usize,
    error: Option<String>,
    _load_task: Task<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KlineSource {
    Spot,
    Alpha,
}

impl KlineSource {
    fn title(self) -> &'static str {
        match self {
            Self::Spot => "现货",
            Self::Alpha => "Alpha",
        }
    }

    fn table_name(self) -> &'static str {
        match self {
            Self::Spot => "spot_klines",
            Self::Alpha => "alpha_klines",
        }
    }
}

#[derive(Clone, Debug)]
struct KlineData {
    open_time: i64,
    open_price: f64,
    high_price: f64,
    low_price: f64,
    close_price: f64,
    volume: f64,
}

impl From<SpotDailyKline> for KlineData {
    fn from(kline: SpotDailyKline) -> Self {
        Self {
            open_time: kline.open_time,
            open_price: kline.open_price,
            high_price: kline.high_price,
            low_price: kline.low_price,
            close_price: kline.close_price,
            volume: kline.volume,
        }
    }
}

impl From<AlphaDailyKline> for KlineData {
    fn from(kline: AlphaDailyKline) -> Self {
        Self {
            open_time: kline.open_time,
            open_price: kline.open_price,
            high_price: kline.high_price,
            low_price: kline.low_price,
            close_price: kline.close_price,
            volume: kline.volume,
        }
    }
}

#[derive(Clone, Copy)]
struct KlineRange {
    high: f64,
    low: f64,
    mid: f64,
}

#[derive(Clone, Copy)]
struct VolumeRange {
    high: f64,
    mid: f64,
}

impl KlineCandlestickPage {
    pub fn new(symbol: String, cx: &mut Context<Self>) -> Self {
        Self::new_with_source(symbol, KlineSource::Spot, cx)
    }

    pub fn new_alpha(symbol: String, cx: &mut Context<Self>) -> Self {
        Self::new_with_source(symbol, KlineSource::Alpha, cx)
    }

    fn new_with_source(symbol: String, source: KlineSource, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            symbol,
            source,
            klines: Vec::new(),
            visible_start: 0,
            visible_count: 120,
            hover_index: None,
            hover_point: None,
            price_chart_bounds: None,
            drag_start_x: None,
            drag_start_visible_start: 0,
            error: None,
            _load_task: Task::ready(()),
        };
        this.reload(cx);
        this
    }

    pub fn set_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        self.set_symbol_with_source(symbol, KlineSource::Spot, cx);
    }

    pub fn set_alpha_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        self.set_symbol_with_source(symbol, KlineSource::Alpha, cx);
    }

    fn set_symbol_with_source(
        &mut self,
        symbol: String,
        source: KlineSource,
        cx: &mut Context<Self>,
    ) {
        self.symbol = symbol;
        self.source = source;
        self.hover_index = None;
        self.hover_point = None;
        self.reload(cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let symbol = self.symbol.clone();
        let source = self.source;
        self.error = None;
        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match source {
                        KlineSource::Spot => {
                            binance_tools::db::spot::load_or_fetch_spot_daily_klines_blocking(
                                BinanceSettings::production(),
                                symbol,
                                120,
                            )
                            .map(|klines| klines.into_iter().map(KlineData::from).collect())
                        }
                        KlineSource::Alpha => {
                            binance_tools::db::alpha::load_or_fetch_alpha_daily_klines_blocking(
                                symbol, 120,
                            )
                            .map(|klines| klines.into_iter().map(KlineData::from).collect())
                        }
                    }
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(klines) => {
                        this.klines = klines;
                        this.visible_count = this.visible_count.clamp(
                            MIN_VISIBLE_KLINES,
                            this.klines.len().max(MIN_VISIBLE_KLINES),
                        );
                        this.visible_start = this
                            .klines
                            .len()
                            .saturating_sub(this.visible_count.min(this.klines.len()));
                        this.drag_start_x = None;
                        this.drag_start_visible_start = this.visible_start;
                        this.hover_index = None;
                        this.hover_point = None;
                        this.error = None;
                    }
                    Err(err) => {
                        this.klines.clear();
                        this.hover_index = None;
                        this.hover_point = None;
                        this.error = Some(err.to_string());
                    }
                }
                cx.notify();
            });
        });
    }

    fn visible_klines(&self) -> Vec<KlineData> {
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

    fn selected_kline<'a>(&'a self, visible: &'a [KlineData]) -> Option<(usize, &'a KlineData)> {
        let index = self
            .hover_index
            .filter(|index| *index < visible.len())
            .unwrap_or_else(|| visible.len().saturating_sub(1));
        visible.get(index).map(|kline| (index, kline))
    }

    fn price_range(&self) -> Option<KlineRange> {
        let visible = self.visible_klines();
        let high = visible
            .iter()
            .map(|kline| kline.high_price)
            .reduce(f64::max)?;
        let low = visible
            .iter()
            .map(|kline| kline.low_price)
            .reduce(f64::min)?;

        Some(KlineRange {
            high,
            low,
            mid: (high + low) / 2.0,
        })
    }

    fn volume_range(&self) -> Option<VolumeRange> {
        let visible = self.visible_klines();
        let high = visible.iter().map(|kline| kline.volume).reduce(f64::max)?;

        Some(VolumeRange {
            high,
            mid: high / 2.0,
        })
    }

    fn kline_change(&self, visible: &[KlineData], index: usize) -> Option<(f64, f64)> {
        let current = visible.get(index)?;
        let previous_close = index
            .checked_sub(1)
            .and_then(|previous| visible.get(previous))
            .map(|kline| kline.close_price)
            .unwrap_or(current.open_price);
        let change = current.close_price - previous_close;
        let percent = if previous_close.abs() > f64::EPSILON {
            change / previous_close * 100.0
        } else {
            0.0
        };

        Some((change, percent))
    }

    fn zoom_in(&mut self, cx: &mut Context<Self>) {
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
        self.hover_index = None;
        self.hover_point = None;
        cx.notify();
    }

    fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let old_count = self.visible_count.min(self.klines.len()).max(1);
        self.visible_count =
            (self.visible_count + ZOOM_STEP).min(self.klines.len().max(MIN_VISIBLE_KLINES));
        let new_count = self.visible_count.min(self.klines.len()).max(1);
        let anchor = self.visible_start + old_count;
        self.visible_start = anchor
            .saturating_sub(new_count)
            .min(self.klines.len().saturating_sub(new_count));
        self.hover_index = None;
        self.hover_point = None;
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
            self.hover_index = None;
            self.hover_point = None;
            cx.notify();
        }
    }

    fn update_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.drag_start_x.is_some() {
            self.pan_to(position, cx);
            return;
        }

        let Some(bounds) = self.price_chart_bounds else {
            return;
        };
        let visible_len = self.visible_klines().len();
        if visible_len == 0 || !bounds.contains(&position) {
            self.clear_hover(cx);
            return;
        }

        let x = (position.x - bounds.left())
            .max(px(0.))
            .min(bounds.size.width);
        let y = (position.y - bounds.top())
            .max(px(0.))
            .min(bounds.size.height);
        let ratio = if bounds.size.width.as_f32() > 0.0 {
            x.as_f32() / bounds.size.width.as_f32()
        } else {
            0.0
        };
        let index = ((ratio * visible_len as f32).floor() as usize).min(visible_len - 1);
        self.hover_index = Some(index);
        self.hover_point = Some(point(x, y));
        cx.notify();
    }

    fn clear_hover(&mut self, cx: &mut Context<Self>) {
        if self.hover_index.is_some() || self.hover_point.is_some() {
            self.hover_index = None;
            self.hover_point = None;
            cx.notify();
        }
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .flex_1()
            .min_h(px(MIN_PRICE_CHART_HEIGHT + VOLUME_CHART_HEIGHT))
            .items_center()
            .justify_center()
            .text_color(palette::muted(cx.theme()))
            .child(format!(
                "暂无 K 线数据，请先查询并缓存 {}",
                self.source.table_name()
            ))
            .into_any_element()
    }

    fn render_market_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible_klines();
        let selected = self.selected_kline(&visible);
        let range = self.price_range();
        let (change, percent) = selected
            .and_then(|(index, _)| self.kline_change(&visible, index))
            .unwrap_or((0.0, 0.0));
        let change_color = market_color(change, cx);
        let mut metrics = Vec::new();

        if let Some((_, kline)) = selected {
            metrics.push(metric(
                "时间",
                full_date(kline.open_time),
                palette::text(cx.theme()),
            ));
            metrics.push(metric(
                "开",
                format_price(kline.open_price),
                palette::text(cx.theme()),
            ));
            metrics.push(metric(
                "高",
                format_price(kline.high_price),
                cx.theme().bullish,
            ));
            metrics.push(metric(
                "低",
                format_price(kline.low_price),
                cx.theme().bearish,
            ));
            metrics.push(metric("收", format_price(kline.close_price), change_color));
            metrics.push(metric("涨跌幅", format!("{percent:.2}%"), change_color));
            metrics.push(metric(
                "成交量",
                format_volume(kline.volume),
                palette::text(cx.theme()),
            ));
        }

        if let Some((index, _)) = selected {
            if let Some(value) = moving_average(&visible, index, 7) {
                metrics.push(metric("MA7", format_price(value), ma7_color()));
            }
            if let Some(value) = moving_average(&visible, index, 25) {
                metrics.push(metric("MA25", format_price(value), ma25_color()));
            }
            if let Some(value) = moving_average(&visible, index, 99) {
                metrics.push(metric("MA99", format_price(value), ma99_color()));
            }
        }

        if let Some(range) = range {
            metrics.push(metric(
                "区间高",
                format_price(range.high),
                cx.theme().bullish,
            ));
            metrics.push(metric(
                "区间低",
                format_price(range.low),
                cx.theme().bearish,
            ));
        }

        h_flex()
            .justify_between()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(chart_grid(cx))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_size(px(12.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_semibold()
                            .text_color(palette::text_strong(cx.theme()))
                            .child(self.symbol.clone()),
                    )
                    .children(metrics),
            )
            .child(
                Button::new("refresh-kline-chart")
                    .outline()
                    .xsmall()
                    .label("刷新")
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
    }

    fn render_price_chart(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(range) = self.price_range() else {
            return self.render_empty(cx);
        };
        let visible = self.visible_klines();
        let tick_margin = (visible.len() / 8).max(1);
        let weak = cx.weak_entity();

        h_flex()
            .flex_1()
            .min_h(px(MIN_PRICE_CHART_HEIGHT))
            .w_full()
            .gap_2()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .child(chart_watermark(self.symbol.trim_end_matches("USDT")))
                    .child(
                        CandlestickChart::new(visible.clone())
                            .x(|kline| short_date(kline.open_time))
                            .open(|kline| kline.open_price)
                            .high(|kline| kline.high_price)
                            .low(|kline| kline.low_price)
                            .close(|kline| kline.close_price)
                            .tick_margin(tick_margin)
                            .body_width_ratio(0.74),
                    )
                    .child(ma_overlay(visible.clone(), range, 7, ma7_color()))
                    .child(ma_overlay(visible.clone(), range, 25, ma25_color()))
                    .child(ma_overlay(visible.clone(), range, 99, ma99_color()))
                    .child(extreme_label(
                        format_price(range.high),
                        palette::text(cx.theme()),
                        LabelPosition::TopRight,
                    ))
                    .child(extreme_label(
                        format_price(range.low),
                        palette::text(cx.theme()),
                        LabelPosition::BottomLeft,
                    ))
                    .child(price_line_label(
                        visible
                            .last()
                            .map(|kline| kline.close_price)
                            .unwrap_or(range.mid),
                        market_color(
                            self.kline_change(&visible, visible.len().saturating_sub(1))
                                .map(|(change, _)| change)
                                .unwrap_or_default(),
                            cx,
                        ),
                    ))
                    .when_some(self.hover_overlay(), |this, overlay| this.child(overlay))
                    .child(
                        div()
                            .id("kline-interaction-layer")
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
                                            _ = weak.update(cx, |this, _| {
                                                this.price_chart_bounds = Some(bounds);
                                            });
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .size_full(),
                            )
                            .on_mouse_move({
                                let weak = weak.clone();
                                move |event, _, cx| {
                                    _ = weak.update(cx, |this, cx| {
                                        this.update_hover(event.position, cx);
                                    });
                                }
                            })
                            .on_mouse_down(MouseButton::Left, {
                                let weak = weak.clone();
                                move |event, _, cx| {
                                    _ = weak.update(cx, |this, _| {
                                        this.begin_drag(event.position);
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
                            })
                            .on_hover({
                                let weak = weak.clone();
                                move |hovered, _, cx| {
                                    if !hovered {
                                        _ = weak.update(cx, |this, cx| this.clear_hover(cx));
                                    }
                                }
                            }),
                    ),
            )
            .child(price_axis(range, cx))
            .into_any_element()
    }

    fn render_volume_chart(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(range) = self.volume_range() else {
            return div().into_any_element();
        };
        let visible = self.visible_klines();
        let tick_margin = (visible.len() / 8).max(1);

        h_flex()
            .h(px(VOLUME_CHART_HEIGHT))
            .w_full()
            .gap_2()
            .border_t_1()
            .border_color(chart_grid(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .child(
                        div()
                            .absolute()
                            .top_2()
                            .left_3()
                            .text_size(px(12.))
                            .text_color(palette::muted(cx.theme()))
                            .child(format!("Vol {}", format_volume(range.high))),
                    )
                    .child(
                        BarChart::new(visible)
                            .x(|kline| short_date(kline.open_time))
                            .y(|kline| kline.volume)
                            .fill(|kline| {
                                if kline.close_price >= kline.open_price {
                                    hsla(0.45, 0.72, 0.52, 1.0)
                                } else {
                                    hsla(0.98, 0.84, 0.61, 1.0)
                                }
                            })
                            .tick_margin(tick_margin),
                    ),
            )
            .child(volume_axis(range, cx))
            .into_any_element()
    }

    fn hover_overlay(&self) -> Option<impl IntoElement> {
        let point = self.hover_point?;
        let visible = self.visible_klines();
        let kline = self.hover_index.and_then(|index| visible.get(index))?;
        let price = self.hover_price(point.y)?;

        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(dashed_vertical(point.x))
                .child(dashed_horizontal(point.y))
                .child(
                    div()
                        .absolute()
                        .left((point.x - px(48.)).max(px(0.)))
                        .bottom_1()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .bg(hsla(0.61, 0.14, 0.30, 1.0))
                        .text_color(hsla(0., 0., 1., 1.))
                        .text_size(px(12.))
                        .child(full_date(kline.open_time)),
                )
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top((point.y - px(12.)).max(px(0.)))
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .bg(hsla(0.61, 0.14, 0.30, 1.0))
                        .text_color(hsla(0., 0., 1., 1.))
                        .text_size(px(12.))
                        .child(format_price(price)),
                ),
        )
    }

    fn hover_price(&self, y: Pixels) -> Option<f64> {
        let bounds = self.price_chart_bounds?;
        let range = self.price_range()?;
        let height = bounds.size.height.as_f32().max(1.0);
        let ratio = (y.as_f32() / height).clamp(0.0, 1.0) as f64;
        Some(range.high - (range.high - range.low) * ratio)
    }

    fn render_chart_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.klines.is_empty() {
            return v_flex()
                .flex_1()
                .h_full()
                .min_h(px(MIN_PRICE_CHART_HEIGHT + VOLUME_CHART_HEIGHT + 72.))
                .rounded(px(6.))
                .border_1()
                .border_color(chart_grid(cx))
                .bg(chart_background(cx))
                .child(self.render_market_header(cx))
                .child(self.render_empty(cx))
                .into_any_element();
        }

        v_flex()
            .flex_1()
            .h_full()
            .min_h(px(MIN_PRICE_CHART_HEIGHT + VOLUME_CHART_HEIGHT + 72.))
            .rounded(px(6.))
            .border_1()
            .border_color(chart_grid(cx))
            .bg(chart_background(cx))
            .overflow_hidden()
            .child(self.render_market_header(cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .p_2()
                    .child(self.render_price_chart(cx))
                    .child(self.render_volume_chart(cx)),
            )
            .into_any_element()
    }
}

impl Render for KlineCandlestickPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .size_full()
            .child(
                h_flex().justify_between().items_center().gap_3().child(
                    v_flex()
                        .gap_1()
                        .child(div().text_size(px(16.)).font_semibold().child(format!(
                            "{} {} 1日K线",
                            self.symbol,
                            self.source.title()
                        )))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette::muted(cx.theme()))
                                .child(format!(
                                    "SQLite {}，最近 {} 根日线",
                                    self.source.table_name(),
                                    self.klines.len()
                                )),
                        ),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("kline-zoom-in")
                            .outline()
                            .xsmall()
                            .label("+")
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_in(cx))),
                    )
                    .child(
                        Button::new("kline-zoom-out")
                            .outline()
                            .xsmall()
                            .label("-")
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_out(cx))),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(cx.theme()))
                            .child(format!(
                                "显示 {} / {} 根",
                                self.visible_count.min(self.klines.len()),
                                self.klines.len()
                            )),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(palette::error_background())
                        .border_1()
                        .border_color(palette::error_border())
                        .text_color(palette::error_text())
                        .line_height(px(18.))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h(px(MIN_PRICE_CHART_HEIGHT + VOLUME_CHART_HEIGHT + 72.))
                    .child(self.render_chart_panel(cx)),
            )
    }
}

#[derive(Clone, Copy)]
enum LabelPosition {
    TopRight,
    BottomLeft,
}

fn metric(label: &'static str, value: impl Into<SharedString>, color: Hsla) -> AnyElement {
    h_flex()
        .gap_1()
        .child(div().text_color(hsla(0.61, 0.14, 0.70, 1.0)).child(label))
        .child(div().text_color(color).font_semibold().child(value.into()))
        .into_any_element()
}

fn chart_watermark(symbol: &str) -> impl IntoElement {
    div()
        .absolute()
        .top(px(118.))
        .left(px(0.))
        .right(px(0.))
        .text_center()
        .text_size(px(44.))
        .font_semibold()
        .text_color(hsla(0.61, 0.12, 0.62, 0.08))
        .child(format!("BINANCE {symbol}"))
}

fn extreme_label(price: String, color: Hsla, position: LabelPosition) -> impl IntoElement {
    div()
        .absolute()
        .when(matches!(position, LabelPosition::TopRight), |this| {
            this.top(px(32.)).right(px(36.))
        })
        .when(matches!(position, LabelPosition::BottomLeft), |this| {
            this.bottom(px(42.)).left(px(36.))
        })
        .px_2()
        .py_1()
        .rounded(px(3.))
        .bg(color.opacity(0.10))
        .text_color(color)
        .text_size(px(12.))
        .font_semibold()
        .child(price)
}

fn price_line_label(price: f64, color: Hsla) -> impl IntoElement {
    h_flex()
        .absolute()
        .right_0()
        .top(px(122.))
        .items_center()
        .child(
            div()
                .h(px(1.))
                .w(px(120.))
                .border_t_1()
                .border_color(color.opacity(0.6)),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .bg(color)
                .text_color(hsla(0., 0., 1., 1.))
                .text_size(px(12.))
                .font_semibold()
                .child(format_price(price)),
        )
}

fn price_axis(range: KlineRange, cx: &mut Context<KlineCandlestickPage>) -> impl IntoElement {
    v_flex()
        .h_full()
        .w(px(96.))
        .justify_between()
        .items_end()
        .text_size(px(12.))
        .text_color(palette::muted(cx.theme()))
        .py_3()
        .child(format_price(range.high))
        .child(format_price(range.mid))
        .child(format_price(range.low))
}

fn volume_axis(range: VolumeRange, cx: &mut Context<KlineCandlestickPage>) -> impl IntoElement {
    v_flex()
        .h_full()
        .w(px(96.))
        .justify_between()
        .items_end()
        .text_size(px(12.))
        .text_color(palette::muted(cx.theme()))
        .py_3()
        .child(format_volume(range.high))
        .child(format_volume(range.mid))
        .child("0")
}

fn market_color(change: f64, cx: &mut Context<KlineCandlestickPage>) -> Hsla {
    if change >= 0.0 {
        cx.theme().bullish
    } else {
        cx.theme().bearish
    }
}

fn chart_background(cx: &mut Context<KlineCandlestickPage>) -> Hsla {
    palette::surface(cx.theme())
}

fn chart_grid(cx: &mut Context<KlineCandlestickPage>) -> Hsla {
    palette::border(cx.theme())
}

fn crosshair_color() -> Hsla {
    hsla(0.61, 0.14, 0.62, 0.88)
}

fn ma7_color() -> Hsla {
    hsla(0.13, 1.0, 0.56, 1.0)
}

fn ma25_color() -> Hsla {
    hsla(0.88, 0.72, 0.58, 1.0)
}

fn ma99_color() -> Hsla {
    hsla(0.72, 0.74, 0.70, 1.0)
}

fn moving_average(data: &[KlineData], index: usize, period: usize) -> Option<f64> {
    if index + 1 < period {
        return None;
    }

    let start = index + 1 - period;
    let total: f64 = data[start..=index]
        .iter()
        .map(|kline| kline.close_price)
        .sum();
    Some(total / period as f64)
}

fn ma_overlay(
    data: Vec<KlineData>,
    range: KlineRange,
    period: usize,
    color: Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if data.len() < period || range.high <= range.low {
                return;
            }

            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32();
            if width <= 0.0 || height <= 0.0 {
                return;
            }

            let mut builder = PathBuilder::stroke(px(1.4));
            let mut started = false;

            for index in 0..data.len() {
                let Some(avg) = moving_average(&data, index, period) else {
                    continue;
                };
                let x = if data.len() <= 1 {
                    width / 2.0
                } else {
                    index as f32 / (data.len() - 1) as f32 * width
                };
                let y = ((range.high - avg) / (range.high - range.low) * height as f64) as f32;
                let point = point(bounds.left() + px(x), bounds.top() + px(y));

                if started {
                    builder.line_to(point);
                } else {
                    builder.move_to(point);
                    started = true;
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

fn dashed_vertical(x: Pixels) -> impl IntoElement {
    div()
        .absolute()
        .left(x)
        .top_0()
        .bottom_0()
        .w(px(1.))
        .children((0..36).map(|index| {
            div()
                .absolute()
                .top(px(index as f32 * 10.))
                .h(px(5.))
                .w(px(1.))
                .bg(crosshair_color())
        }))
}

fn dashed_horizontal(y: Pixels) -> impl IntoElement {
    div()
        .absolute()
        .top(y)
        .left_0()
        .right_0()
        .h(px(1.))
        .children((0..160).map(|index| {
            div()
                .absolute()
                .left(px(index as f32 * 10.))
                .h(px(1.))
                .w(px(5.))
                .bg(crosshair_color())
        }))
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
        format!("{:.3}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.3}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.3}K", value / 1_000.0)
    } else {
        format!("{value:.3}")
    }
}

fn short_date(timestamp_ms: i64) -> String {
    let (_, month, day) = date_parts(timestamp_ms);
    format!("{month:02}/{day:02}")
}

fn full_date(timestamp_ms: i64) -> String {
    let (year, month, day) = date_parts(timestamp_ms);
    format!("{year:04}/{month:02}/{day:02}")
}

fn date_parts(timestamp_ms: i64) -> (i32, u32, u32) {
    let days = timestamp_ms.div_euclid(86_400_000);
    civil_from_days(days)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let day = doy - (153 * mp + 2).div_euclid(5) + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

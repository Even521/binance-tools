use crate::ui::palette;
use binance_tools::binance::alpha::AlphaToken;
use gpui::{EventEmitter, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::{cmp::Ordering, collections::BTreeSet};

const ALL_CHAINS: &str = "全部链";
const MAX_TILES: usize = 100;
const MIN_SHARE_TO_SHOW: f64 = 0.0025;

pub struct AlphaHeatmapPage {
    search_input: Entity<InputState>,
    tokens: Vec<AlphaToken>,
    visible_tokens: Vec<AlphaToken>,
    chains: Vec<String>,
    selected_chain: String,
    weight_mode: AlphaHeatmapWeightMode,
    error: Option<String>,
    loading: bool,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlphaHeatmapWeightMode {
    MarketCap,
    Volume24h,
}

#[derive(Clone, Debug)]
pub enum AlphaHeatmapEvent {
    OpenKline(String),
}

impl AlphaHeatmapWeightMode {
    fn label(self) -> &'static str {
        match self {
            Self::MarketCap => "市值",
            Self::Volume24h => "成交额",
        }
    }

    fn value(self, token: &AlphaToken) -> f64 {
        match self {
            Self::MarketCap => parse_number(&token.market_cap),
            Self::Volume24h => parse_number(&token.volume_24h),
        }
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0)
    }

    fn total_label(self) -> &'static str {
        match self {
            Self::MarketCap => "显示总市值",
            Self::Volume24h => "显示总成交额",
        }
    }

    fn item_label(self) -> &'static str {
        match self {
            Self::MarketCap => "市值",
            Self::Volume24h => "成交额",
        }
    }
}

impl AlphaHeatmapPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索 Alpha ID、Symbol、名称、链")
                .default_value("")
        });
        let _subscriptions =
            vec![cx.subscribe_in(&search_input, window, Self::on_search_input_event)];

        let mut this = Self {
            search_input,
            tokens: Vec::new(),
            visible_tokens: Vec::new(),
            chains: vec![ALL_CHAINS.to_string()],
            selected_chain: ALL_CHAINS.to_string(),
            weight_mode: AlphaHeatmapWeightMode::MarketCap,
            error: None,
            loading: false,
            _load_task: Task::ready(()),
            _subscriptions,
        };
        this.reload(false, cx);
        this
    }

    fn on_search_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.refresh_visible_tokens(cx);
            cx.notify();
        }
    }

    fn reload(&mut self, force_refresh: bool, cx: &mut Context<Self>) {
        self.error = None;
        self.loading = true;
        self.visible_tokens.clear();

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if force_refresh {
                        binance_tools::db::alpha::refresh_alpha_tokens_blocking()
                    } else {
                        binance_tools::db::alpha::load_or_fetch_alpha_tokens_blocking()
                    }
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(tokens) => {
                        this.error = None;
                        this.tokens = tokens;
                        this.chains = collect_token_chains(&this.tokens);
                        if !this.chains.contains(&this.selected_chain) {
                            this.selected_chain = ALL_CHAINS.to_string();
                        }
                        this.refresh_visible_tokens(cx);
                    }
                    Err(err) => {
                        this.error = Some(err.to_string());
                        this.visible_tokens.clear();
                    }
                }
                cx.notify();
            });
        });
    }

    fn set_chain(&mut self, chain: String, cx: &mut Context<Self>) {
        self.selected_chain = chain;
        self.refresh_visible_tokens(cx);
        cx.notify();
    }

    fn set_weight_mode(&mut self, mode: AlphaHeatmapWeightMode, cx: &mut Context<Self>) {
        self.weight_mode = mode;
        self.refresh_visible_tokens(cx);
        cx.notify();
    }

    fn open_kline(&mut self, symbol: String, cx: &mut Context<Self>) {
        cx.emit(AlphaHeatmapEvent::OpenKline(symbol));
    }

    fn refresh_visible_tokens(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let selected_chain = self.selected_chain.clone();
        let weight_mode = self.weight_mode;
        let mut tokens = self
            .tokens
            .iter()
            .filter(|token| {
                (selected_chain == ALL_CHAINS || token_chain_label(token) == selected_chain)
                    && token_matches_query(token, &query)
                    && parse_number(&token.percent_change_24h).is_some()
            })
            .cloned()
            .collect::<Vec<_>>();

        tokens.sort_by(|a, b| {
            weight_mode
                .value(b)
                .partial_cmp(&weight_mode.value(a))
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.alpha_id.cmp(&b.alpha_id))
        });
        let total = total_weight(&tokens, weight_mode);
        tokens.retain(|token| weight_share(weight_mode.value(token), total) >= MIN_SHARE_TO_SHOW);
        tokens.truncate(MAX_TILES);
        self.visible_tokens = tokens;
    }

    fn render_chain_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .flex_wrap()
            .children(self.chains.iter().enumerate().map(|(index, chain)| {
                let chain = chain.clone();
                let selected = chain == self.selected_chain;
                Button::new(("alpha-heatmap-chain", index))
                    .label(chain.clone())
                    .ghost()
                    .xsmall()
                    .when(selected, |button| {
                        button
                            .bg(cx.theme().primary.opacity(0.16))
                            .text_color(palette::text_strong(cx.theme()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_chain(chain.clone(), cx);
                    }))
                    .into_any_element()
            }))
    }

    fn render_weight_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = [
            AlphaHeatmapWeightMode::MarketCap,
            AlphaHeatmapWeightMode::Volume24h,
        ];
        h_flex()
            .gap_0()
            .children(modes.into_iter().enumerate().map(|(index, mode)| {
                let selected = mode == self.weight_mode;
                Button::new(("alpha-heatmap-weight", index))
                    .label(mode.label())
                    .ghost()
                    .xsmall()
                    .rounded(px(0.))
                    .when(index == 0, |button| button.rounded_l(px(4.)))
                    .when(index == modes.len() - 1, |button| button.rounded_r(px(4.)))
                    .when(selected, |button| {
                        button
                            .bg(cx.theme().primary.opacity(0.16))
                            .text_color(palette::text_strong(cx.theme()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_weight_mode(mode, cx);
                    }))
                    .into_any_element()
            }))
    }

    fn render_tile(&self, row_ix: usize, token: &AlphaToken, cx: &mut Context<Self>) -> AnyElement {
        let change = parse_number(&token.percent_change_24h).unwrap_or(0.0);
        let weight = self.weight_mode.value(token);
        let total_weight = total_weight(&self.visible_tokens, self.weight_mode);
        let share = weight_share(weight, total_weight);
        let size = tile_size(row_ix, share);
        let symbol = alpha_usdt_symbol(token);
        let display_symbol = if size.compact {
            token.symbol.clone()
        } else {
            symbol.clone()
        };

        v_flex()
            .id(("alpha-heatmap-tile", row_ix))
            .flex_basis(px(size.width))
            .flex_grow()
            .h(px(size.height))
            .min_w(px(82.))
            .min_h(px(58.))
            .p(px(size.padding))
            .gap_1()
            .rounded(px(7.))
            .bg(change_color(change))
            .border_2()
            .border_color(hsla(0.60, 0.12, 0.15, 1.0))
            .overflow_hidden()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_kline(symbol.clone(), cx);
            }))
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(size.symbol_font))
                            .font_semibold()
                            .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                            .child(display_symbol),
                    )
                    .when(!size.compact, |this| {
                        this.child(
                            div()
                                .truncate()
                                .text_size(px(size.price_font))
                                .text_color(hsla(0.0, 0.0, 1.0, 0.94))
                                .child(format_price(parse_number(&token.price))),
                        )
                    })
                    .child(
                        div()
                            .truncate()
                            .text_size(px(size.change_font))
                            .text_color(hsla(0.0, 0.0, 1.0, 0.94))
                            .child(format!("{change:+.2}%")),
                    ),
            )
            .when(!size.compact, |this| {
                this.child(
                    div()
                        .truncate()
                        .text_size(px(size.name_font))
                        .text_color(hsla(0.0, 0.0, 1.0, 0.72))
                        .child(format!(
                            "{} {} · {:.2}%",
                            self.weight_mode.item_label(),
                            format_money(weight),
                            share * 100.0
                        )),
                )
            })
            .into_any_element()
    }
}

impl EventEmitter<AlphaHeatmapEvent> for AlphaHeatmapPage {}

impl Render for AlphaHeatmapPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible_count = self.visible_tokens.len();
        let total_visible_weight = total_weight(&self.visible_tokens, self.weight_mode);
        let hidden_count = self.tokens.len().saturating_sub(visible_count);
        let up_count = self
            .visible_tokens
            .iter()
            .filter(|token| parse_number(&token.percent_change_24h).unwrap_or(0.0) > 0.0)
            .count();
        let down_count = self
            .visible_tokens
            .iter()
            .filter(|token| parse_number(&token.percent_change_24h).unwrap_or(0.0) < 0.0)
            .count();
        let flat_count = visible_count.saturating_sub(up_count + down_count);

        v_flex()
            .gap_3()
            .size_full()
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded(px(8.))
                    .bg(palette::surface_strong(cx.theme()))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_start()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .min_w(px(260.))
                                    .flex_1()
                                    .child(div().text_size(px(16.)).font_semibold().child("Alpha 热力图"))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(format!(
                                                "按 {} 占比显示 {} 个 Alpha Token，{} {}，已隐藏低占比 {} 个；绿色上涨，红色下跌。",
                                                self.weight_mode.label(),
                                                visible_count,
                                                self.weight_mode.total_label(),
                                                format_money(total_visible_weight),
                                                hidden_count
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(
                                        div()
                                            .w(px(280.))
                                            .max_w(px(340.))
                                            .child(Input::new(&self.search_input).small().cleanable(true)),
                                    )
                                    .child(
                                        Button::new("alpha-heatmap-sync")
                                            .outline()
                                            .xsmall()
                                            .label("同步")
                                            .loading(self.loading)
                                            .disabled(self.loading)
                                            .on_click(cx.listener(|this, _, _, cx| this.reload(false, cx))),
                                    )
                                    .child(
                                        Button::new("alpha-heatmap-refresh")
                                            .primary()
                                            .xsmall()
                                            .label("刷新")
                                            .disabled(self.loading)
                                            .on_click(cx.listener(|this, _, _, cx| this.reload(true, cx))),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .flex_wrap()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(stat_badge(format!("上涨 {} 个", up_count), cx.theme().success.opacity(0.10), cx.theme().success.opacity(0.95)))
                                    .child(stat_badge(format!("下跌 {} 个", down_count), cx.theme().danger.opacity(0.10), cx.theme().danger.opacity(0.95)))
                                    .child(stat_badge(format!("持平 {} 个", flat_count), palette::muted(cx.theme()).opacity(0.10), palette::muted(cx.theme()))),
                            )
                            .child(self.render_weight_tabs(cx)),
                    )
                    .child(self.render_chain_tabs(cx)),
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
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .w_full()
                            .gap(px(0.))
                            .children(heatmap_rows(self.visible_tokens.len()).into_iter().map(
                                |(start, end)| {
                                    h_flex()
                                        .w_full()
                                        .gap(px(0.))
                                        .children((start..end).filter_map(|index| {
                                            self.visible_tokens
                                                .get(index)
                                                .map(|token| self.render_tile(index, token, cx))
                                        }))
                                },
                            )),
                    ),
            )
    }
}

#[derive(Clone, Copy)]
struct TileSize {
    width: f32,
    height: f32,
    symbol_font: f32,
    price_font: f32,
    name_font: f32,
    change_font: f32,
    padding: f32,
    compact: bool,
}

fn tile_size(index: usize, share: f64) -> TileSize {
    if index < 4 {
        TileSize {
            width: if index == 0 { 390.0 } else { 292.0 },
            height: 300.0,
            symbol_font: if index < 2 { 30.0 } else { 28.0 },
            price_font: if index < 2 { 24.0 } else { 22.0 },
            name_font: 12.0,
            change_font: if index < 2 { 24.0 } else { 22.0 },
            padding: if index < 2 { 26.0 } else { 24.0 },
            compact: false,
        }
    } else if index < 8 || share >= 0.025 {
        TileSize {
            width: 230.0,
            height: 140.0,
            symbol_font: 24.0,
            price_font: 18.0,
            name_font: 11.0,
            change_font: 18.0,
            padding: 16.0,
            compact: false,
        }
    } else {
        TileSize {
            width: 92.0,
            height: 92.0,
            symbol_font: 16.0,
            price_font: 0.0,
            name_font: 0.0,
            change_font: 15.0,
            padding: 10.0,
            compact: true,
        }
    }
}

fn heatmap_rows(len: usize) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let mut start = 0;

    for row_size in [4, 4] {
        if start >= len {
            return rows;
        }
        let end = (start + row_size).min(len);
        rows.push((start, end));
        start = end;
    }

    while start < len {
        let end = (start + 10).min(len);
        rows.push((start, end));
        start = end;
    }

    rows
}

fn total_weight(tokens: &[AlphaToken], weight_mode: AlphaHeatmapWeightMode) -> f64 {
    tokens
        .iter()
        .map(|token| weight_mode.value(token))
        .filter(|value| value.is_finite() && *value > 0.0)
        .sum()
}

fn weight_share(weight: f64, total_weight: f64) -> f64 {
    if total_weight > 0.0 {
        (weight / total_weight).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn change_color(change: f64) -> Hsla {
    if change > 0.0 {
        let lightness = (0.52 + (change.abs().min(15.0) / 15.0) * 0.08) as f32;
        hsla(0.42, 0.50, lightness, 1.0)
    } else if change < 0.0 {
        let lightness = (0.61 + (change.abs().min(15.0) / 15.0) * 0.06) as f32;
        hsla(0.98, 0.78, lightness, 1.0)
    } else {
        hsla(0.42, 0.18, 0.56, 1.0)
    }
}

fn collect_token_chains(tokens: &[AlphaToken]) -> Vec<String> {
    let mut chains = BTreeSet::new();
    for token in tokens {
        let label = token_chain_label(token);
        if label != "-" {
            chains.insert(label);
        }
    }

    let mut result = vec![ALL_CHAINS.to_string()];
    result.extend(chains);
    result
}

fn token_chain_label(token: &AlphaToken) -> String {
    match (
        token.chain_name.trim().is_empty(),
        token.chain_id.trim().is_empty(),
    ) {
        (true, true) => "-".to_string(),
        (false, true) => token.chain_name.clone(),
        (true, false) => token.chain_id.clone(),
        (false, false) => format!("{} ({})", token.chain_name, token.chain_id),
    }
}

fn token_matches_query(token: &AlphaToken, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    [
        token.alpha_id.as_str(),
        token.symbol.as_str(),
        token.name.as_str(),
        token.chain_id.as_str(),
        token.chain_name.as_str(),
        token.contract_address.as_str(),
        token.cex_coin_name.as_str(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(query))
}

fn alpha_usdt_symbol(token: &AlphaToken) -> String {
    if token.alpha_id.ends_with("USDT") {
        token.alpha_id.clone()
    } else {
        format!("{}USDT", token.alpha_id)
    }
}

fn parse_number(value: &Option<String>) -> Option<f64> {
    value.as_deref().and_then(|value| value.parse::<f64>().ok())
}

fn stat_badge(label: String, bg: Hsla, fg: Hsla) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .rounded(px(4.))
        .bg(bg)
        .text_size(px(12.))
        .text_color(fg)
        .child(label)
        .into_any_element()
}

fn format_price(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "-".to_string();
    };

    if value >= 100.0 {
        format!("{value:.2}")
    } else if value >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.8}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

fn format_money(value: f64) -> String {
    let abs = value.abs();

    if abs >= 1_000_000_000_000.0 {
        format!("${:.2}T", value / 1_000_000_000_000.0)
    } else if abs >= 1_000_000_000.0 {
        format!("${:.2}B", value / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("${:.2}K", value / 1_000.0)
    } else {
        format!("${value:.2}")
    }
}

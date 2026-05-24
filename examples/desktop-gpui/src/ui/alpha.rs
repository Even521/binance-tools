use crate::ui::palette;
use binance_tools::binance::alpha::{AlphaSymbol, AlphaToken};
use gpui::{EventEmitter, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableState},
    v_flex,
};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
};

pub struct AlphaTokensPage {
    table: Entity<TableState<AlphaTokensTableDelegate>>,
    search_input: Entity<InputState>,
    tokens: Vec<AlphaToken>,
    chains: Vec<String>,
    selected_chain: String,
    selected_category: TokenCategory,
    active_filter_group: TokenFilterGroup,
    error: Option<String>,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
pub enum AlphaTokensEvent {
    OpenKline(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenCategory {
    All,
    PointsPlus,
    SecurityToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenFilterGroup {
    Category,
    Chain,
}

impl TokenCategory {
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部分类",
            Self::PointsPlus => "积分+",
            Self::SecurityToken => "证券代币",
        }
    }
}

impl AlphaTokensPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(AlphaTokensTableDelegate::new(page), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索 Alpha ID、Symbol、名称、链、合约地址")
                .default_value("")
        });
        let _subscriptions =
            vec![cx.subscribe_in(&search_input, window, Self::on_search_input_event)];
        let mut this = Self {
            table,
            search_input,
            tokens: Vec::new(),
            chains: vec!["全部链".to_string()],
            selected_chain: "全部链".to_string(),
            selected_category: TokenCategory::All,
            active_filter_group: TokenFilterGroup::Category,
            error: None,
            _load_task: Task::ready(()),
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    fn open_kline(&mut self, symbol: String, cx: &mut Context<Self>) {
        cx.emit(AlphaTokensEvent::OpenKline(symbol));
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

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(false, cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    fn load(&mut self, force_refresh: bool, cx: &mut Context<Self>) {
        self.error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            table.refresh(cx);
        });

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
                match result {
                    Ok(tokens) => {
                        this.error = None;
                        this.tokens = tokens;
                        this.chains = collect_token_chains(&this.tokens);
                        if !this.chains.contains(&this.selected_chain) {
                            this.selected_chain = "全部链".to_string();
                        }
                        this.refresh_visible_tokens(cx);
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

    fn set_chain(&mut self, chain: String, cx: &mut Context<Self>) {
        self.selected_chain = chain;
        self.active_filter_group = TokenFilterGroup::Chain;
        self.refresh_visible_tokens(cx);
        cx.notify();
    }

    fn set_category(&mut self, category: TokenCategory, cx: &mut Context<Self>) {
        self.selected_category = category;
        self.active_filter_group = TokenFilterGroup::Category;
        self.refresh_visible_tokens(cx);
        cx.notify();
    }

    fn refresh_visible_tokens(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let selected_chain = self.selected_chain.clone();
        let selected_category = self.selected_category;
        let active_filter_group = self.active_filter_group;
        let mut tokens = self
            .tokens
            .iter()
            .filter(|token| {
                (match active_filter_group {
                    TokenFilterGroup::Category => token_matches_category(token, selected_category),
                    TokenFilterGroup::Chain => {
                        selected_chain == "全部链" || token_chain_label(token) == selected_chain
                    }
                }) && token_matches_query(token, &query)
            })
            .cloned()
            .collect::<Vec<_>>();
        tokens.sort_by(|a, b| {
            number_cmp(&b.percent_change_24h, &a.percent_change_24h)
                .then_with(|| a.alpha_id.cmp(&b.alpha_id))
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_tokens(tokens);
            table.refresh(cx);
        });
    }

    fn render_chain_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .flex_wrap()
            .children(self.chains.iter().enumerate().map(|(index, chain)| {
                let chain = chain.clone();
                let selected = self.active_filter_group == TokenFilterGroup::Chain
                    && chain == self.selected_chain;
                Button::new(("alpha-chain", index))
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

    fn render_category_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let categories = [
            TokenCategory::All,
            TokenCategory::PointsPlus,
            TokenCategory::SecurityToken,
        ];

        h_flex()
            .gap_0()
            .flex_wrap()
            .children(categories.into_iter().enumerate().map(|(index, category)| {
                let selected = self.active_filter_group == TokenFilterGroup::Category
                    && category == self.selected_category;
                Button::new(("alpha-category", index))
                    .label(category.label())
                    .ghost()
                    .xsmall()
                    .rounded(px(0.))
                    .when(index == 0, |button| button.rounded_l(px(4.)))
                    .when(index == categories.len() - 1, |button| {
                        button.rounded_r(px(4.))
                    })
                    .when(selected, |button| {
                        button
                            .bg(cx.theme().primary.opacity(0.16))
                            .text_color(palette::text_strong(cx.theme()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_category(category, cx);
                    }))
                    .into_any_element()
            }))
    }
}

impl Render for AlphaTokensPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let token_count = self.table.read(cx).delegate().tokens.len();
        let total_count = self.tokens.len();
        let loading = self.table.read(cx).delegate().loading;
        let visible_tokens = &self.table.read(cx).delegate().tokens;
        let up_count = visible_tokens
            .iter()
            .filter(|token| parse_number(&token.percent_change_24h).unwrap_or(0.0) > 0.0)
            .count();
        let down_count = visible_tokens
            .iter()
            .filter(|token| parse_number(&token.percent_change_24h).unwrap_or(0.0) < 0.0)
            .count();
        let flat_count = token_count.saturating_sub(up_count + down_count);

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
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .font_semibold()
                                            .child("Alpha Token 列表"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(format!(
                                                "按链分类查询 Binance Alpha Token；当前显示 {} 条，总数 {} 条。",
                                                token_count, total_count
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        div().w(px(320.)).max_w(px(420.)).child(
                                            Input::new(&self.search_input)
                                                .small()
                                                .cleanable(true),
                                        ),
                                    )
                                    .child(
                                        Button::new("alpha-token-refresh")
                                            .primary()
                                            .xsmall()
                                            .label("查询 Token")
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.refresh(cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(stat_badge(
                                format!("上涨 {} 个", up_count),
                                cx.theme().success.opacity(0.10),
                                cx.theme().success.opacity(0.95),
                            ))
                            .child(stat_badge(
                                format!("下跌 {} 个", down_count),
                                cx.theme().danger.opacity(0.10),
                                cx.theme().danger.opacity(0.95),
                            ))
                            .child(stat_badge(
                                format!("持平 {} 个", flat_count),
                                palette::muted(cx.theme()).opacity(0.10),
                                palette::muted(cx.theme()),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .flex_wrap()
                            .child(self.render_category_tabs(cx))
                            .child(div().w(px(1.)).h(px(18.)).bg(palette::border(cx.theme())))
                            .child(self.render_chain_tabs(cx)),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(error_banner(error, cx.theme()))
            })
            .child(table_box(&self.table))
    }
}

impl EventEmitter<AlphaTokensEvent> for AlphaTokensPage {}

pub struct AlphaExchangeInfoPage {
    table: Entity<TableState<AlphaSymbolsTableDelegate>>,
    search_input: Entity<InputState>,
    symbols: Vec<AlphaSymbolRow>,
    quote_assets: Vec<String>,
    selected_quote: String,
    timezone: String,
    asset_count: usize,
    error: Option<String>,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl AlphaExchangeInfoPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(AlphaSymbolsTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索 Symbol、Base、Quote、状态")
                .default_value("")
        });
        let _subscriptions =
            vec![cx.subscribe_in(&search_input, window, Self::on_search_input_event)];
        let mut this = Self {
            table,
            search_input,
            symbols: Vec::new(),
            quote_assets: vec!["全部".to_string()],
            selected_quote: "USDT".to_string(),
            timezone: String::new(),
            asset_count: 0,
            error: None,
            _load_task: Task::ready(()),
            _subscriptions,
        };
        this.reload(cx);
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
            self.refresh_visible_symbols(cx);
            cx.notify();
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(false, cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    fn load(&mut self, force_refresh: bool, cx: &mut Context<Self>) {
        self.error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            table.refresh(cx);
        });

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let info = if force_refresh {
                        binance_tools::db::alpha::refresh_alpha_exchange_info_blocking()?
                    } else {
                        binance_tools::db::alpha::load_or_fetch_alpha_exchange_info_blocking()?
                    };
                    let tokens = binance_tools::db::alpha::load_or_fetch_alpha_tokens_blocking()
                        .unwrap_or_default();
                    Ok::<_, anyhow::Error>((info, tokens))
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok((info, tokens)) => {
                        this.error = None;
                        this.timezone = info.timezone;
                        this.asset_count = info.assets.len();
                        let changes = tokens
                            .into_iter()
                            .map(|token| (token.alpha_id, parse_number(&token.percent_change_24h)))
                            .collect::<HashMap<_, _>>();
                        this.symbols = info
                            .symbols
                            .into_iter()
                            .map(|symbol| AlphaSymbolRow {
                                price_change_percent: changes
                                    .get(&symbol.base_asset)
                                    .copied()
                                    .flatten(),
                                symbol,
                            })
                            .collect();
                        this.quote_assets = collect_alpha_quote_assets(&this.symbols);
                        if !this.quote_assets.contains(&this.selected_quote) {
                            this.selected_quote = "全部".to_string();
                        }
                        this.refresh_visible_symbols(cx);
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

    fn set_quote(&mut self, quote: String, cx: &mut Context<Self>) {
        self.selected_quote = quote;
        self.refresh_visible_symbols(cx);
        cx.notify();
    }

    fn refresh_visible_symbols(&mut self, cx: &mut Context<Self>) {
        let selected_quote = self.selected_quote.clone();
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let mut symbols = self
            .symbols
            .iter()
            .filter(|row| {
                (selected_quote == "全部" || row.symbol.quote_asset == selected_quote)
                    && alpha_symbol_row_matches(row, &query)
            })
            .cloned()
            .collect::<Vec<_>>();
        symbols.sort_by(|a, b| a.symbol.symbol.cmp(&b.symbol.symbol));
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_symbols(symbols);
            table.refresh(cx);
        });
    }

    fn render_quote_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .flex_wrap()
            .children(self.quote_assets.iter().enumerate().map(|(index, quote)| {
                let quote = quote.clone();
                let selected = quote == self.selected_quote;
                Button::new(("alpha-exchange-quote", index))
                    .label(quote.clone())
                    .ghost()
                    .xsmall()
                    .when(selected, |button| {
                        button
                            .bg(cx.theme().primary.opacity(0.16))
                            .text_color(palette::text_strong(cx.theme()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_quote(quote.clone(), cx);
                    }))
                    .into_any_element()
            }))
    }
}

impl Render for AlphaExchangeInfoPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let table = self.table.read(cx);
        let visible_symbols = &table.delegate().symbols;
        let symbol_count = visible_symbols.len();
        let up_count = visible_symbols
            .iter()
            .filter(|row| row.price_change_percent.unwrap_or(0.0) > 0.0)
            .count();
        let down_count = visible_symbols
            .iter()
            .filter(|row| row.price_change_percent.unwrap_or(0.0) < 0.0)
            .count();
        let flat_count = symbol_count.saturating_sub(up_count + down_count);
        let loading = table.delegate().loading;

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
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .font_semibold()
                                            .child("Alpha 交易对信息"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(format!(
                                                "查询 Binance Alpha exchange info；当前 {} 个交易对，{} 个资产，时区 {}。",
                                                symbol_count,
                                                self.asset_count,
                                                empty_dash(&self.timezone)
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        div().w(px(280.)).max_w(px(340.)).child(
                                            Input::new(&self.search_input)
                                                .small()
                                                .cleanable(true),
                                        ),
                                    )
                                    .child(
                                        Button::new("alpha-exchange-refresh")
                                            .primary()
                                            .xsmall()
                                            .label("查询交易对")
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.refresh(cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(stat_badge(
                                format!("上涨 {} 个", up_count),
                                cx.theme().success.opacity(0.10),
                                cx.theme().success.opacity(0.95),
                            ))
                            .child(stat_badge(
                                format!("下跌 {} 个", down_count),
                                cx.theme().danger.opacity(0.10),
                                cx.theme().danger.opacity(0.95),
                            ))
                            .child(stat_badge(
                                format!("持平 {} 个", flat_count),
                                palette::muted(cx.theme()).opacity(0.10),
                                palette::muted(cx.theme()),
                            )),
                    )
                    .child(self.render_quote_tabs(cx)),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(error_banner(error, cx.theme()))
            })
            .child(table_box(&self.table))
    }
}

fn error_banner(error: String, _: &gpui_component::Theme) -> AnyElement {
    div()
        .p_3()
        .rounded(px(8.))
        .bg(palette::error_background())
        .border_1()
        .border_color(palette::error_border())
        .text_color(palette::error_text())
        .line_height(px(18.))
        .child(error)
        .into_any_element()
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

fn collect_token_chains(tokens: &[AlphaToken]) -> Vec<String> {
    let mut chains = BTreeSet::new();
    for token in tokens {
        let label = token_chain_label(token);
        if label != "-" {
            chains.insert(label);
        }
    }
    let mut result = vec!["全部链".to_string()];
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
        token.token_id.as_str(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(query))
}

fn token_matches_category(token: &AlphaToken, category: TokenCategory) -> bool {
    match category {
        TokenCategory::All => true,
        TokenCategory::PointsPlus => matches!(token.mul_point, Some(2 | 4)),
        TokenCategory::SecurityToken => token_is_security_token(token),
    }
}

fn token_is_security_token(token: &AlphaToken) -> bool {
    token.stock_state
        && (looks_like_tokenized_security_symbol(&token.symbol)
            || looks_like_tokenized_security_symbol(&token.name)
            || looks_like_tokenized_security_symbol(&token.cex_coin_name))
}

fn looks_like_tokenized_security_symbol(value: &str) -> bool {
    let value = value.trim();
    value.len() > 2
        && value.ends_with("on")
        && value
            .chars()
            .take(value.len().saturating_sub(2))
            .any(|ch| ch.is_ascii_uppercase())
}

fn token_name_with_points(token: &AlphaToken) -> String {
    match token.mul_point {
        Some(point) if point > 1 => format!("{} x{}", token.name, point),
        _ => token.name.clone(),
    }
}

fn alpha_usdt_symbol(token: &AlphaToken) -> String {
    if token.alpha_id.ends_with("USDT") {
        token.alpha_id.clone()
    } else {
        format!("{}USDT", token.alpha_id)
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

fn table_box<D>(table: &Entity<TableState<D>>) -> impl IntoElement
where
    D: TableDelegate + 'static,
{
    v_flex().flex_1().h_full().min_h(px(420.)).w_full().child(
        div().flex_1().size_full().overflow_hidden().child(
            DataTable::new(table)
                .stripe(true)
                .bordered(true)
                .scrollbar_visible(true, true),
        ),
    )
}

#[derive(Clone)]
struct AlphaTokensTableDelegate {
    columns: Vec<Column>,
    tokens: Vec<AlphaToken>,
    loading: bool,
    page: WeakEntity<AlphaTokensPage>,
}

impl AlphaTokensTableDelegate {
    fn new(page: WeakEntity<AlphaTokensPage>) -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "名称")
                    .width(px(214.))
                    .fixed_left()
                    .sortable(),
                Column::new("price", "价格").width(px(108.)).sortable(),
                Column::new("change", "24h涨跌").width(px(82.)).sortable(),
                Column::new("volume", "24h成交量")
                    .width(px(112.))
                    .sortable(),
                Column::new("market_cap", "市值").width(px(112.)).sortable(),
                Column::new("liquidity", "流动性")
                    .width(px(112.))
                    .sortable(),
                Column::new("cex", "已上CEX").width(px(76.)).sortable(),
                Column::new("hot", "热门").width(px(66.)).sortable(),
                Column::new("chain", "链").width(px(118.)).sortable(),
                Column::new("contract", "合约地址").width(px(280.)),
            ],
            tokens: Vec::new(),
            loading: false,
            page,
        }
    }

    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.tokens.clear();
        }
    }

    fn set_tokens(&mut self, mut tokens: Vec<AlphaToken>) {
        tokens.sort_by(|a, b| {
            number_cmp(&b.percent_change_24h, &a.percent_change_24h)
                .then_with(|| a.alpha_id.cmp(&b.alpha_id))
        });
        self.tokens = tokens;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.tokens.clear();
        self.loading = false;
    }

    fn cell(value: impl Into<SharedString>, color: Hsla) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px_1()
            .text_size(px(11.))
            .text_color(color)
            .child(value.into())
            .into_any_element()
    }

    fn bool_cell(value: bool, app_theme: &gpui_component::Theme) -> AnyElement {
        Self::cell(if value { "Yes" } else { "No" }, palette::text(app_theme))
    }

    fn name_cell(
        &self,
        row_ix: usize,
        token: &AlphaToken,
        app_theme: &gpui_component::Theme,
    ) -> AnyElement {
        let page = self.page.clone();
        let kline_symbol = alpha_usdt_symbol(token);

        h_flex()
            .size_full()
            .items_center()
            .justify_between()
            .px_1()
            .gap_1()
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .justify_center()
                    .gap_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(palette::text_strong(app_theme))
                            .child(token.symbol.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(palette::muted_soft(app_theme))
                            .child(token_name_with_points(token)),
                    ),
            )
            .child(
                Button::new(("alpha-token-kline", row_ix))
                    .ghost()
                    .xsmall()
                    .w(px(22.))
                    .h(px(18.))
                    .tooltip("K 线图")
                    .child(candlestick_icon(hsla(0.61, 0.08, 0.55, 1.0)))
                    .on_click(move |_, _, cx| {
                        _ = page.update(cx, |this, cx| {
                            this.open_kline(kline_symbol.clone(), cx);
                        });
                    }),
            )
            .into_any_element()
    }

    fn contract_cell(
        row_ix: usize,
        token: &AlphaToken,
        app_theme: &gpui_component::Theme,
    ) -> AnyElement {
        let contract = token.contract_address.clone();

        h_flex()
            .size_full()
            .items_center()
            .gap_1()
            .px_1()
            .child(
                Button::new(("alpha-contract-copy", row_ix))
                    .ghost()
                    .xsmall()
                    .w(px(22.))
                    .h(px(18.))
                    .tooltip("复制合约地址")
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(app_theme))
                            .child("⧉"),
                    )
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(contract.clone()));
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(palette::muted(app_theme))
                    .child(token.contract_address.clone()),
            )
            .into_any_element()
    }
}

impl TableDelegate for AlphaTokensTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.tokens.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let app_theme = cx.theme();
        let Some(token) = self.tokens.get(row_ix) else {
            return Self::cell("", palette::text(app_theme));
        };
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "symbol" => self.name_cell(row_ix, token, app_theme),
            "chain" => Self::cell(token_chain_label(token), palette::text(app_theme)),
            "price" => Self::cell(format_price_4(&token.price), palette::text(app_theme)),
            "change" => Self::cell(
                format_percent(&token.percent_change_24h),
                change_color(&token.percent_change_24h, app_theme),
            ),
            "volume" => Self::cell(
                format_decimal_2(&token.volume_24h),
                palette::text(app_theme),
            ),
            "market_cap" => Self::cell(
                format_decimal_2(&token.market_cap),
                palette::text(app_theme),
            ),
            "liquidity" => Self::cell(format_decimal_2(&token.liquidity), palette::text(app_theme)),
            "cex" => Self::bool_cell(token.listing_cex, app_theme),
            "hot" => Self::bool_cell(token.hot_tag, app_theme),
            "contract" => Self::contract_cell(row_ix, token, app_theme),
            _ => Self::cell("", palette::text(app_theme)),
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
        self.tokens.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "symbol" => a.symbol.cmp(&b.symbol),
                "chain" => a.chain_name.cmp(&b.chain_name),
                "price" => number_cmp(&a.price, &b.price),
                "change" => number_cmp(&a.percent_change_24h, &b.percent_change_24h),
                "volume" => number_cmp(&a.volume_24h, &b.volume_24h),
                "market_cap" => number_cmp(&a.market_cap, &b.market_cap),
                "liquidity" => number_cmp(&a.liquidity, &b.liquidity),
                "cex" => a.listing_cex.cmp(&b.listing_cex),
                "hot" => a.hot_tag.cmp(&b.hot_tag),
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

#[derive(Clone)]
struct AlphaSymbolRow {
    symbol: AlphaSymbol,
    price_change_percent: Option<f64>,
}

fn alpha_symbol_row_matches(row: &AlphaSymbolRow, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    [
        row.symbol.symbol.as_str(),
        row.symbol.base_asset.as_str(),
        row.symbol.quote_asset.as_str(),
        row.symbol.status.as_str(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(query))
}

fn collect_alpha_quote_assets(rows: &[AlphaSymbolRow]) -> Vec<String> {
    let mut quotes = BTreeSet::new();
    for row in rows {
        if !row.symbol.quote_asset.is_empty() {
            quotes.insert(row.symbol.quote_asset.clone());
        }
    }
    let mut result = vec!["全部".to_string()];
    result.extend(quotes);
    result
}

#[derive(Clone)]
struct AlphaSymbolsTableDelegate {
    columns: Vec<Column>,
    symbols: Vec<AlphaSymbolRow>,
    loading: bool,
}

impl Default for AlphaSymbolsTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "Symbol")
                    .width(px(132.))
                    .fixed_left()
                    .sortable(),
                Column::new("status", "Status").width(px(86.)).sortable(),
                Column::new("base", "Base").width(px(104.)).sortable(),
                Column::new("quote", "Quote").width(px(76.)).sortable(),
                Column::new("price_precision", "Price Precision")
                    .width(px(118.))
                    .sortable(),
                Column::new("quantity_precision", "Qty Precision")
                    .width(px(112.))
                    .sortable(),
                Column::new("base_precision", "Base Precision")
                    .width(px(112.))
                    .sortable(),
                Column::new("quote_precision", "Quote Precision")
                    .width(px(116.))
                    .sortable(),
                Column::new("order_types", "Order Types").width(px(140.)),
                Column::new("filters", "Filters").width(px(320.)),
            ],
            symbols: Vec::new(),
            loading: false,
        }
    }
}

impl AlphaSymbolsTableDelegate {
    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.symbols.clear();
        }
    }

    fn set_symbols(&mut self, mut symbols: Vec<AlphaSymbolRow>) {
        symbols.sort_by(|a, b| a.symbol.symbol.cmp(&b.symbol.symbol));
        self.symbols = symbols;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.symbols.clear();
        self.loading = false;
    }

    fn cell(value: impl Into<SharedString>, color: Hsla) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px_1()
            .text_size(px(11.))
            .text_color(color)
            .child(value.into())
            .into_any_element()
    }
}

impl TableDelegate for AlphaSymbolsTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.symbols.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let app_theme = cx.theme();
        let Some(row) = self.symbols.get(row_ix) else {
            return Self::cell("", palette::text(app_theme));
        };
        let symbol = &row.symbol;
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "symbol" => Self::cell(symbol.symbol.clone(), palette::text_strong(app_theme)),
            "status" => Self::cell(symbol.status.clone(), palette::text(app_theme)),
            "base" => Self::cell(symbol.base_asset.clone(), palette::text(app_theme)),
            "quote" => Self::cell(symbol.quote_asset.clone(), palette::text(app_theme)),
            "price_precision" => {
                Self::cell(option_i64(symbol.price_precision), palette::text(app_theme))
            }
            "quantity_precision" => Self::cell(
                option_i64(symbol.quantity_precision),
                palette::text(app_theme),
            ),
            "base_precision" => Self::cell(
                option_i64(symbol.base_asset_precision),
                palette::text(app_theme),
            ),
            "quote_precision" => {
                Self::cell(option_i64(symbol.quote_precision), palette::text(app_theme))
            }
            "order_types" => Self::cell(symbol.order_types.join(", "), palette::text(app_theme)),
            "filters" => Self::cell(
                symbol
                    .filters
                    .iter()
                    .map(|filter| filter.filter_type.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                palette::muted(app_theme),
            ),
            _ => Self::cell("", palette::text(app_theme)),
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
        self.symbols.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "symbol" => a.symbol.symbol.cmp(&b.symbol.symbol),
                "status" => a.symbol.status.cmp(&b.symbol.status),
                "base" => a.symbol.base_asset.cmp(&b.symbol.base_asset),
                "quote" => a.symbol.quote_asset.cmp(&b.symbol.quote_asset),
                "price_precision" => a.symbol.price_precision.cmp(&b.symbol.price_precision),
                "quantity_precision" => a
                    .symbol
                    .quantity_precision
                    .cmp(&b.symbol.quantity_precision),
                "base_precision" => a
                    .symbol
                    .base_asset_precision
                    .cmp(&b.symbol.base_asset_precision),
                "quote_precision" => a.symbol.quote_precision.cmp(&b.symbol.quote_precision),
                _ => a.symbol.symbol.cmp(&b.symbol.symbol),
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

fn option_text(value: &Option<String>) -> String {
    value
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn format_price_4(value: &Option<String>) -> String {
    value
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| option_text(value))
}

fn format_decimal_2(value: &Option<String>) -> String {
    value
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| option_text(value))
}

fn format_percent(value: &Option<String>) -> String {
    value
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:+.2}%"))
        .unwrap_or_else(|| {
            value
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("{value}%"))
                .unwrap_or_else(|| "-".to_string())
        })
}

fn option_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

fn number_cmp(a: &Option<String>, b: &Option<String>) -> Ordering {
    let a = parse_number(a);
    let b = parse_number(b);
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

fn parse_number(value: &Option<String>) -> Option<f64> {
    value.as_deref().and_then(|value| value.parse::<f64>().ok())
}

fn change_color(value: &Option<String>, app_theme: &gpui_component::Theme) -> Hsla {
    match value.as_deref().and_then(|value| value.parse::<f64>().ok()) {
        Some(value) if value >= 0.0 => app_theme.success.opacity(0.92),
        Some(_) => app_theme.danger.opacity(0.92),
        None => palette::text(app_theme),
    }
}

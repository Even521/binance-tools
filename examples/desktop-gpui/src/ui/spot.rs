use crate::ui::palette;
use binance_tools::binance::{BinanceSettings, spot::SpotSymbolInfo};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableState},
    v_flex,
};
use std::collections::{BTreeSet, HashMap};

pub struct SpotPage {
    settings: BinanceSettings,
    table: Entity<TableState<SpotSymbolsTableDelegate>>,
    search_input: Entity<InputState>,
    symbols: Vec<SpotSymbolRow>,
    quote_assets: Vec<String>,
    selected_quote: String,
    base_asset_count: usize,
    error: Option<String>,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SpotPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(SpotSymbolsTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索 Symbol、Base、Quote")
                .default_value("")
        });
        let _subscriptions =
            vec![cx.subscribe_in(&search_input, window, Self::on_search_input_event)];

        let mut this = Self {
            settings: BinanceSettings::production(),
            table,
            search_input,
            symbols: Vec::new(),
            quote_assets: vec!["全部".to_string()],
            selected_quote: "USDT".to_string(),
            base_asset_count: 0,
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
        let settings = self.settings.clone();
        self.error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            table.refresh(cx);
        });

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let (symbols, base_asset_count) =
                        binance_tools::db::spot::load_or_fetch_spot_symbols_blocking(settings)?;
                    let products =
                        binance_tools::db::market::load_or_fetch_market_products_blocking()?;
                    Ok::<_, anyhow::Error>((symbols, base_asset_count, products))
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok((symbols, base_asset_count, products)) => {
                        this.error = None;
                        this.base_asset_count = base_asset_count;
                        let changes = products
                            .into_iter()
                            .map(|product| (product.symbol, product.price_change_percent))
                            .collect::<HashMap<_, _>>();
                        this.symbols = symbols
                            .into_iter()
                            .map(|symbol| SpotSymbolRow {
                                price_change_percent: changes
                                    .get(&symbol.symbol)
                                    .copied()
                                    .flatten(),
                                symbol,
                            })
                            .collect();
                        this.quote_assets = collect_spot_quote_assets(&this.symbols);
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
                    && spot_row_matches(row, &query)
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
                Button::new(("spot-quote", index))
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

impl Render for SpotPage {
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
                                    .child(div().text_size(px(16.)).font_semibold().child("现货"))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(format!(
                                                "查询 Binance {} 全部现货交易对，当前 {} 条，现货总数 {}",
                                                self.settings.environment(),
                                                symbol_count,
                                                self.base_asset_count
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
                                        Button::new("spot-refresh")
                                            .primary()
                                            .xsmall()
                                            .label("查询现货")
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.reload(cx)
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
                v_flex().flex_1().h_full().min_h(px(420.)).w_full().child(
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

#[derive(Clone)]
struct SpotSymbolRow {
    symbol: SpotSymbolInfo,
    price_change_percent: Option<f64>,
}

fn spot_row_matches(row: &SpotSymbolRow, query: &str) -> bool {
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

fn collect_spot_quote_assets(rows: &[SpotSymbolRow]) -> Vec<String> {
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

#[derive(Clone)]
struct SpotSymbolsTableDelegate {
    columns: Vec<Column>,
    symbols: Vec<SpotSymbolRow>,
    loading: bool,
}

impl Default for SpotSymbolsTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "Symbol")
                    .width(px(110.))
                    .fixed_left()
                    .sortable(),
                Column::new("status", "Status").width(px(86.)).sortable(),
                Column::new("base_asset", "Base").width(px(84.)).sortable(),
                Column::new("quote_asset", "Quote")
                    .width(px(84.))
                    .sortable(),
                Column::new("base_precision", "Base Precision").width(px(112.)),
                Column::new("quote_precision", "Quote Precision").width(px(116.)),
                Column::new("spot_allowed", "Spot").width(px(70.)),
                Column::new("margin_allowed", "Margin").width(px(82.)),
                Column::new("order_types", "Order Types").width(px(300.)),
            ],
            symbols: Vec::new(),
            loading: false,
        }
    }
}

impl SpotSymbolsTableDelegate {
    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.symbols.clear();
        }
    }

    fn set_symbols(&mut self, symbols: Vec<SpotSymbolRow>) {
        self.symbols = symbols;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.symbols.clear();
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

    fn bool_cell(value: bool) -> AnyElement {
        Self::cell(if value { "Yes" } else { "No" })
    }
}

impl TableDelegate for SpotSymbolsTableDelegate {
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
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(symbol) = self.symbols.get(row_ix) else {
            return Self::cell("");
        };
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "symbol" => Self::cell(symbol.symbol.symbol.clone()),
            "status" => Self::cell(symbol.symbol.status.clone()),
            "base_asset" => Self::cell(symbol.symbol.base_asset.clone()),
            "quote_asset" => Self::cell(symbol.symbol.quote_asset.clone()),
            "base_precision" => Self::cell(symbol.symbol.base_asset_precision.to_string()),
            "quote_precision" => Self::cell(symbol.symbol.quote_asset_precision.to_string()),
            "spot_allowed" => Self::bool_cell(symbol.symbol.spot_trading_allowed),
            "margin_allowed" => Self::bool_cell(symbol.symbol.margin_trading_allowed),
            "order_types" => Self::cell(symbol.symbol.order_types.join(", ")),
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

        self.symbols.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "symbol" => a.symbol.symbol.cmp(&b.symbol.symbol),
                "status" => a.symbol.status.cmp(&b.symbol.status),
                "base_asset" => a.symbol.base_asset.cmp(&b.symbol.base_asset),
                "quote_asset" => a.symbol.quote_asset.cmp(&b.symbol.quote_asset),
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

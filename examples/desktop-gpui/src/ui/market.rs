use crate::ui::palette;
use binance_tools::binance::market::MarketProduct;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::Scrollbar,
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableState},
    v_flex,
};
use std::cmp::Ordering;

const QUOTE_ASSETS: &[&str] = &["USDT", "USDC", "FDUSD", "BTC", "BNB", "ETH", "TRY", "EUR"];
const AI_ANALYSIS_LIMIT: usize = 50;

pub enum MarketProductsEvent {
    AnalyzeWithAi {
        prompt: String,
        display_content: String,
        rule_context: AiRuleContext,
    },
    OpenKline(String),
}

#[derive(Clone, Debug)]
pub struct AiRuleContext {
    pub key: String,
    pub label: String,
}

pub struct MarketProductsPage {
    table: Entity<TableState<MarketProductsTableDelegate>>,
    search_input: Entity<InputState>,
    products: Vec<MarketProduct>,
    selected_quote: String,
    error: Option<String>,
    last_loaded_count: usize,
    loading_mode: Option<MarketLoadMode>,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarketLoadMode {
    Sync,
    Refresh,
}

impl MarketProductsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(MarketProductsTableDelegate::new(page), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索交易对、名称、标签")
                .default_value("")
        });
        let _subscriptions =
            vec![cx.subscribe_in(&search_input, window, Self::on_search_input_event)];

        let mut this = Self {
            table,
            search_input,
            products: Vec::new(),
            selected_quote: "USDT".to_string(),
            error: None,
            last_loaded_count: 0,
            loading_mode: None,
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
            self.refresh_visible_products(cx);
            cx.notify();
        }
    }

    fn reload(&mut self, force_refresh: bool, cx: &mut Context<Self>) {
        self.error = None;
        self.loading_mode = Some(if force_refresh {
            MarketLoadMode::Refresh
        } else {
            MarketLoadMode::Sync
        });
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            table.refresh(cx);
        });

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if force_refresh {
                        binance_tools::db::market::refresh_market_products_blocking()
                    } else {
                        binance_tools::db::market::load_or_fetch_market_products_blocking()
                    }
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading_mode = None;
                match result {
                    Ok(products) => {
                        this.error = None;
                        this.last_loaded_count = products.len();
                        this.products = products;
                        this.refresh_visible_products(cx);
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
        self.refresh_visible_products(cx);
        cx.notify();
    }

    fn analyze_with_ai(&mut self, cx: &mut Context<Self>) {
        let prompt = {
            let table = self.table.read(cx);
            let products = &table.delegate().products;
            if products.is_empty() {
                self.error = Some("当前市场没有可分析的数据，请先同步或切换市场。".to_string());
                cx.notify();
                return;
            }

            build_market_analysis_prompt(&self.selected_quote, self.last_loaded_count, products)
        };
        let sample_count = self
            .table
            .read(cx)
            .delegate()
            .products
            .len()
            .min(AI_ANALYSIS_LIMIT);
        let display_content = format!(
            "AI 分析 {} 市场榜单（前 {} 条精简数据）",
            self.selected_quote, sample_count
        );

        cx.emit(MarketProductsEvent::AnalyzeWithAi {
            prompt,
            display_content,
            rule_context: AiRuleContext {
                key: "market_products".to_string(),
                label: "市场榜单".to_string(),
            },
        });
        cx.notify();
    }

    fn open_kline(&mut self, symbol: String, cx: &mut Context<Self>) {
        cx.emit(MarketProductsEvent::OpenKline(symbol));
    }

    fn refresh_visible_products(&mut self, cx: &mut Context<Self>) {
        let selected_quote = self.selected_quote.clone();
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let mut products = self
            .products
            .iter()
            .filter(|product| {
                product.quote_asset == selected_quote
                    && product.is_trading
                    && market_product_matches(product, &query)
            })
            .cloned()
            .collect::<Vec<_>>();
        products.sort_by(|a, b| {
            b.price_change_percent
                .partial_cmp(&a.price_change_percent)
                .unwrap_or(Ordering::Equal)
        });

        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_products(products);
            table.refresh(cx);
        });
    }

    fn render_quote_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .flex_wrap()
            .children(QUOTE_ASSETS.iter().enumerate().map(|(index, quote)| {
                let quote = (*quote).to_string();
                let selected = quote == self.selected_quote;
                Button::new(("market-quote", index))
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

fn market_product_matches(product: &MarketProduct, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let fields = [
        product.symbol.as_str(),
        product.base_asset.as_str(),
        product.quote_asset.as_str(),
        product.asset_name.as_str(),
        product.quote_name.as_str(),
        product.partition.as_str(),
        product.partition_name.as_str(),
        product.status.as_str(),
    ];

    fields
        .iter()
        .any(|field| field.to_lowercase().contains(query))
        || product
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
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

impl Render for MarketProductsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let table = self.table.read(cx);
        let visible_products = &table.delegate().products;
        let visible_count = visible_products.len();
        let up_count = visible_products
            .iter()
            .filter(|product| product.price_change_percent.unwrap_or(0.0) > 0.0)
            .count();
        let down_count = visible_products
            .iter()
            .filter(|product| product.price_change_percent.unwrap_or(0.0) < 0.0)
            .count();
        let surface = palette::surface_strong(cx.theme());
        let border = palette::border(cx.theme());
        let muted = palette::muted(cx.theme());
        let syncing = self.loading_mode == Some(MarketLoadMode::Sync);
        let refreshing = self.loading_mode == Some(MarketLoadMode::Refresh);
        let loading = self.loading_mode.is_some();
        let horizontal_scroll_handle = self.table.read(cx).horizontal_scroll_handle.clone();
        let flat_count = visible_count.saturating_sub(up_count + down_count);

        v_flex()
            .gap_3()
            .size_full()
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded(px(8.))
                    .bg(surface)
                    .border_1()
                    .border_color(border)
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
                                        div().text_size(px(16.)).font_semibold().child("市场榜单"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(muted)
                                            .child(format!(
                                                "来自 Binance Web product 接口；缓存 5 分钟。当前 {} 条，缓存总数 {} 条。",
                                                visible_count, self.last_loaded_count
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(
                                        Button::new("market-ai-analysis")
                                            .outline()
                                            .xsmall()
                                            .label("AI 分析")
                                            .disabled(loading || visible_count == 0)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.analyze_with_ai(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("market-reload")
                                            .outline()
                                            .xsmall()
                                            .label("同步")
                                            .loading(syncing)
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.reload(false, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("market-refresh")
                                            .outline()
                                            .xsmall()
                                            .label("刷新")
                                            .loading(refreshing)
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.reload(true, cx);
                                            })),
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
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded(px(4.))
                                            .bg(cx.theme().success.opacity(0.10))
                                            .text_size(px(12.))
                                            .text_color(cx.theme().success.opacity(0.95))
                                            .child(format!("上涨 {} 个", up_count)),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded(px(4.))
                                            .bg(cx.theme().danger.opacity(0.10))
                                            .text_size(px(12.))
                                            .text_color(cx.theme().danger.opacity(0.95))
                                            .child(format!("下跌 {} 个", down_count)),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded(px(4.))
                                            .bg(muted.opacity(0.10))
                                            .text_size(px(12.))
                                            .text_color(muted)
                                            .child(format!("持平 {} 个", flat_count)),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(280.))
                                    .max_w(px(340.))
                                    .child(Input::new(&self.search_input).small().cleanable(true)),
                            ),
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
                    )
                    .child(
                        div()
                            .ml(px(170.))
                            .h(px(12.))
                            .w_full()
                            .child(Scrollbar::horizontal(&horizontal_scroll_handle)),
                    ),
            )
    }
}

impl EventEmitter<MarketProductsEvent> for MarketProductsPage {}

#[derive(Clone)]
struct MarketProductsTableDelegate {
    columns: Vec<Column>,
    products: Vec<MarketProduct>,
    loading: bool,
    page: WeakEntity<MarketProductsPage>,
}

impl MarketProductsTableDelegate {
    fn new(page: WeakEntity<MarketProductsPage>) -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "名称")
                    .width(px(174.))
                    .fixed_left()
                    .sortable(),
                Column::new("price", "价格").width(px(86.)).sortable(),
                Column::new("change", "24小时").width(px(82.)).sortable(),
                Column::new("high", "24h最高").width(px(86.)).sortable(),
                Column::new("low", "24h最低").width(px(86.)).sortable(),
                Column::new("volume", "24h成交量").width(px(96.)).sortable(),
                Column::new("market_cap", "市值").width(px(96.)).sortable(),
                Column::new("circulating_supply", "流通量")
                    .width(px(92.))
                    .sortable(),
                Column::new("partition", "分区").width(px(84.)).sortable(),
                Column::new("status", "状态").width(px(74.)).sortable(),
                Column::new("tags", "标签").width(px(220.)),
            ],
            products: Vec::new(),
            loading: false,
            page,
        }
    }

    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.products.clear();
        }
    }

    fn set_products(&mut self, products: Vec<MarketProduct>) {
        self.products = products;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.products.clear();
        self.loading = false;
    }

    fn text_cell(value: impl Into<SharedString>, color: Hsla) -> AnyElement {
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

    fn name_cell(
        &self,
        row_ix: usize,
        product: &MarketProduct,
        app_theme: &gpui_component::Theme,
    ) -> AnyElement {
        let page = self.page.clone();
        let symbol = product.symbol.clone();

        v_flex()
            .size_full()
            .justify_center()
            .px_1()
            .gap_0()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(app_theme))
                                    .child(product.base_asset.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(palette::muted(app_theme))
                                    .child(format!("/{}", product.quote_asset)),
                            ),
                    )
                    .child(
                        Button::new(("market-kline", row_ix))
                            .ghost()
                            .xsmall()
                            .w(px(22.))
                            .h(px(18.))
                            .tooltip("K 线图")
                            .child(candlestick_icon(palette::muted(app_theme)))
                            .on_click(move |_, _, cx| {
                                _ = page.update(cx, |this, cx| {
                                    this.open_kline(symbol.clone(), cx);
                                });
                            }),
                    ),
            )
            .child(
                h_flex().gap_1().items_center().child(
                    div()
                        .text_size(px(10.))
                        .text_color(palette::muted_soft(app_theme))
                        .child(product.asset_name.clone()),
                ),
            )
            .into_any_element()
    }

    fn change_cell(product: &MarketProduct, app_theme: &gpui_component::Theme) -> AnyElement {
        let value = product.price_change_percent.unwrap_or(0.0);
        let color = if value >= 0.0 {
            app_theme.success.opacity(0.92)
        } else {
            app_theme.danger.opacity(0.92)
        };
        Self::text_cell(format!("{:+.2}%", value), color)
    }

    fn numeric_value(product: &MarketProduct, key: &str) -> Option<f64> {
        match key {
            "price" => product.last_price,
            "change" => product.price_change_percent,
            "high" => product.high_price,
            "low" => product.low_price,
            "volume" => product.quote_volume,
            "market_cap" => product.market_cap,
            "circulating_supply" => product.circulating_supply,
            _ => None,
        }
    }
}

impl TableDelegate for MarketProductsTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.products.len()
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
        let Some(product) = self.products.get(row_ix) else {
            return Self::text_cell("", palette::text(app_theme));
        };
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "symbol" => self.name_cell(row_ix, product, app_theme),
            "price" => Self::text_cell(format_price(product.last_price), palette::text(app_theme)),
            "change" => Self::change_cell(product, app_theme),
            "high" => Self::text_cell(format_price(product.high_price), palette::text(app_theme)),
            "low" => Self::text_cell(format_price(product.low_price), palette::text(app_theme)),
            "volume" => Self::text_cell(
                format_compact(product.quote_volume),
                palette::text(app_theme),
            ),
            "market_cap" => {
                Self::text_cell(format_compact(product.market_cap), palette::text(app_theme))
            }
            "circulating_supply" => Self::text_cell(
                format_number(product.circulating_supply),
                palette::text(app_theme),
            ),
            "partition" => Self::text_cell(product.partition.clone(), palette::text(app_theme)),
            "status" => Self::text_cell(product.status.clone(), palette::muted(app_theme)),
            "tags" => Self::text_cell(product.tags.join(", "), palette::muted(app_theme)),
            _ => Self::text_cell("", palette::text(app_theme)),
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

        self.products.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "symbol" | "tags" => a.symbol.cmp(&b.symbol),
                "partition" => a.partition.cmp(&b.partition),
                "status" => a.status.cmp(&b.status),
                _ => Self::numeric_value(a, &key)
                    .partial_cmp(&Self::numeric_value(b, &key))
                    .unwrap_or(Ordering::Equal),
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

fn format_compact(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "-".to_string();
    };
    let abs = value.abs();

    if abs >= 1_000_000_000.0 {
        format!("${:.2}B", value / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("${:.2}K", value / 1_000.0)
    } else {
        format!("${value:.2}")
    }
}

fn format_number(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "-".to_string();
    };
    let abs = value.abs();

    if abs >= 1_000_000_000.0 {
        format!("{:.2}B", value / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else {
        format!("{value:.2}")
    }
}

fn build_market_analysis_prompt(
    quote_asset: &str,
    total_cached_count: usize,
    products: &[MarketProduct],
) -> String {
    let sample_count = products.len().min(AI_ANALYSIS_LIMIT);
    let data = products
        .iter()
        .take(AI_ANALYSIS_LIMIT)
        .map(analysis_product_json)
        .collect::<Vec<_>>()
        .join(",\n");

    format!(
        r#"当前市场：{quote_asset}
数据来源：本地 SQLite 缓存的 Binance Web product 数据
缓存规则：数据 5 分钟内有效
数据范围：当前市场按 24h 涨跌幅排序后的前 {sample_count} 条；缓存总数：{total_cached_count} 条

分析数据 JSON：{{
  "quote_asset": {quote_json},
  "limit": {limit},
  "sample_count": {sample_count},
  "products": [
{data}
  ]
}}"#,
        quote_json = json_string(quote_asset),
        limit = AI_ANALYSIS_LIMIT,
    )
}

fn analysis_product_json(product: &MarketProduct) -> String {
    format!(
        r#"    {{
      "symbol": {symbol},
      "base_asset": {base_asset},
      "asset_name": {asset_name},
      "price": {price},
      "change_24h_percent": {change},
      "high_24h": {high},
      "low_24h": {low},
      "quote_volume": {quote_volume},
      "market_cap": {market_cap},
      "circulating_supply": {circulating_supply},
      "tags": {tags}
    }}"#,
        symbol = json_string(&product.symbol),
        base_asset = json_string(&product.base_asset),
        asset_name = json_string(&product.asset_name),
        price = json_number(product.last_price),
        change = json_number(product.price_change_percent),
        high = json_number(product.high_price),
        low = json_number(product.low_price),
        quote_volume = json_number(product.quote_volume),
        market_cap = json_number(product.market_cap),
        circulating_supply = json_number(product.circulating_supply),
        tags = json_string_array(&product.tags),
    )
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push(' '),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn json_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn json_number(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| {
            let text = format!("{value:.8}");
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        })
        .unwrap_or_else(|| "null".to_string())
}

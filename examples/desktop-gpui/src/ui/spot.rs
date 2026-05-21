use crate::ui::palette;
use binance_tools::binance::{BinanceSettings, spot::SpotSymbolInfo};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

pub struct SpotPage {
    settings: BinanceSettings,
    table: Entity<TableState<SpotSymbolsTableDelegate>>,
    base_asset_count: usize,
    error: Option<String>,
    _load_task: Task<()>,
}

impl SpotPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(SpotSymbolsTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });

        let mut this = Self {
            settings: BinanceSettings::production(),
            table,
            base_asset_count: 0,
            error: None,
            _load_task: Task::ready(()),
        };
        this.reload(cx);
        this
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
                    binance_tools::db::spot::load_or_fetch_spot_symbols_blocking(settings)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok((symbols, base_asset_count)) => {
                        this.error = None;
                        this.base_asset_count = base_asset_count;
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_symbols(symbols);
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

impl Render for SpotPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let symbol_count = self.table.read(cx).delegate().symbols.len();

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
                        Button::new("spot-refresh")
                            .primary()
                            .xsmall()
                            .label("查询现货")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
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
struct SpotSymbolsTableDelegate {
    columns: Vec<Column>,
    symbols: Vec<SpotSymbolInfo>,
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

    fn set_symbols(&mut self, symbols: Vec<SpotSymbolInfo>) {
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
            "symbol" => Self::cell(symbol.symbol.clone()),
            "status" => Self::cell(symbol.status.clone()),
            "base_asset" => Self::cell(symbol.base_asset.clone()),
            "quote_asset" => Self::cell(symbol.quote_asset.clone()),
            "base_precision" => Self::cell(symbol.base_asset_precision.to_string()),
            "quote_precision" => Self::cell(symbol.quote_asset_precision.to_string()),
            "spot_allowed" => Self::bool_cell(symbol.spot_trading_allowed),
            "margin_allowed" => Self::bool_cell(symbol.margin_trading_allowed),
            "order_types" => Self::cell(symbol.order_types.join(", ")),
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
                "symbol" => a.symbol.cmp(&b.symbol),
                "status" => a.status.cmp(&b.status),
                "base_asset" => a.base_asset.cmp(&b.base_asset),
                "quote_asset" => a.quote_asset.cmp(&b.quote_asset),
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

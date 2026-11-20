use super::{SpotAiAnalysisPage, SpotAiAnalysisRow};
use crate::ui::palette;
use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    table::{Column, ColumnSort, TableDelegate, TableState},
    v_flex,
};
use std::cmp::Ordering;

#[derive(Clone)]
pub(super) struct SpotAiAnalysisTableDelegate {
    columns: Vec<Column>,
    rows: Vec<SpotAiAnalysisRow>,
    page: WeakEntity<SpotAiAnalysisPage>,
}

impl SpotAiAnalysisTableDelegate {
    pub(super) fn new(page: WeakEntity<SpotAiAnalysisPage>) -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "名称")
                    .width(px(190.))
                    .fixed_left()
                    .sortable(),
                Column::new("change", "24h 涨跌").width(px(112.)).sortable(),
                Column::new("trend", "趋势判断").width(px(96.)).sortable(),
                Column::new("turnover", "24h 成交额")
                    .width(px(140.))
                    .sortable(),
                Column::new("ma120", "120日线位置")
                    .width(px(132.))
                    .sortable(),
                Column::new("score", "综合评分").width(px(90.)).sortable(),
            ],
            rows: Vec::new(),
            page,
        }
    }

    pub(super) fn set_rows(&mut self, rows: Vec<SpotAiAnalysisRow>) {
        self.rows = rows;
    }

    pub(super) fn visible_rows_count(&self) -> usize {
        self.rows.len()
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

    fn select_cell(
        &self,
        row_ix: usize,
        col_ix: usize,
        row: &SpotAiAnalysisRow,
        content: impl IntoElement,
    ) -> AnyElement {
        let page = self.page.clone();
        let symbol = row.product.symbol.clone();
        Button::new(("spot-ai-row-cell", row_ix * self.columns.len() + col_ix))
            .ghost()
            .xsmall()
            .w_full()
            .h_full()
            .child(content)
            .on_click(move |_, _, cx| {
                _ = page.update(cx, |this, cx| this.select_symbol(symbol.clone(), cx));
            })
            .into_any_element()
    }

    fn name_cell(
        &self,
        row_ix: usize,
        _col_ix: usize,
        row: &SpotAiAnalysisRow,
        theme: &gpui_component::Theme,
    ) -> AnyElement {
        let page_for_select = self.page.clone();
        let symbol_for_select = row.product.symbol.clone();
        let page_for_chart = self.page.clone();
        let symbol_for_chart = row.product.symbol.clone();

        h_flex()
            .size_full()
            .items_center()
            .justify_between()
            .gap_1()
            .child(
                Button::new(("spot-ai-row-name", row_ix))
                    .ghost()
                    .xsmall()
                    .flex_1()
                    .h_full()
                    .child(
                        v_flex()
                            .size_full()
                            .justify_center()
                            .px_1()
                            .gap_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_semibold()
                                            .text_color(palette::text_strong(theme))
                                            .child(row.product.base_asset.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(palette::muted(theme))
                                            .child(format!("/{}", row.product.quote_asset)),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .text_size(px(10.))
                                    .text_color(palette::text(theme))
                                    .child(truncate_asset_name(&row.product.asset_name, 9)),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        _ = page_for_select.update(cx, |this, cx| {
                            this.select_symbol(symbol_for_select.clone(), cx);
                        });
                    }),
            )
            .child(
                Button::new(("spot-ai-row-kline", row_ix))
                    .ghost()
                    .xsmall()
                    .w(px(22.))
                    .h(px(20.))
                    .tooltip("查看 K 线图")
                    .child(candlestick_icon(palette::muted(theme)))
                    .on_click(move |_, _, cx| {
                        _ = page_for_chart.update(cx, |_, cx| {
                            cx.emit(super::SpotAiAnalysisEvent::OpenKline(
                                symbol_for_chart.clone(),
                            ));
                        });
                    }),
            )
            .into_any_element()
    }
}

impl TableDelegate for SpotAiAnalysisTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
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
        let theme = cx.theme();
        let Some(row) = self.rows.get(row_ix) else {
            return Self::cell("", palette::text(theme));
        };
        let key = self.columns[col_ix].key.as_ref();
        match key {
            "symbol" => self.name_cell(row_ix, col_ix, row, theme),
            "change" => {
                let value = row.product.price_change_percent.unwrap_or(0.0);
                self.select_cell(
                    row_ix,
                    col_ix,
                    row,
                    Self::cell(
                        format!("{value:+.2}%"),
                        if value >= 0.0 {
                            theme.success
                        } else {
                            theme.danger
                        },
                    ),
                )
            }
            "trend" => self.select_cell(
                row_ix,
                col_ix,
                row,
                Self::cell(row.trend_label.clone(), palette::text(theme)),
            ),
            "score" => self.select_cell(
                row_ix,
                col_ix,
                row,
                Self::cell(format!("{:.0}", row.total_score), palette::text(theme)),
            ),
            "turnover" => self.select_cell(
                row_ix,
                col_ix,
                row,
                Self::cell(
                    format_compact(row.product.quote_volume),
                    palette::text(theme),
                ),
            ),
            "ma120" => {
                let value = row.ma120_difference_percent;
                self.select_cell(
                    row_ix,
                    col_ix,
                    row,
                    Self::cell(
                        format_ma120_location(value),
                        match value {
                            Some(value) if value > 1.0 => theme.success,
                            Some(value) if value < -1.0 => theme.danger,
                            _ => palette::muted(theme),
                        },
                    ),
                )
            }
            _ => Self::cell("", palette::text(theme)),
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
        self.rows.sort_by(|a, b| {
            let order = match key.as_str() {
                "symbol" => a.product.symbol.cmp(&b.product.symbol),
                "change" => a
                    .product
                    .price_change_percent
                    .partial_cmp(&b.product.price_change_percent)
                    .unwrap_or(Ordering::Equal),
                "score" => a
                    .total_score
                    .partial_cmp(&b.total_score)
                    .unwrap_or(Ordering::Equal),
                "turnover" => a
                    .product
                    .quote_volume
                    .partial_cmp(&b.product.quote_volume)
                    .unwrap_or(Ordering::Equal),
                "trend" => a.trend_label.cmp(&b.trend_label),
                "ma120" => a
                    .ma120_difference_percent
                    .partial_cmp(&b.ma120_difference_percent)
                    .unwrap_or(Ordering::Equal),
                _ => Ordering::Equal,
            };
            if descending { order.reverse() } else { order }
        });
    }

    fn loading(&self, _: &App) -> bool {
        false
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
                .left(px(10.))
                .top(px(1.))
                .w(px(1.5))
                .h(px(12.))
                .bg(color),
        )
        .child(
            div()
                .absolute()
                .left(px(8.))
                .top(px(5.))
                .w(px(5.))
                .h(px(5.))
                .border_1()
                .border_color(color)
                .rounded(px(1.5)),
        )
        .into_any_element()
}

fn truncate_asset_name(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let shortened = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn format_ma120_location(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.2}%"))
        .unwrap_or_default()
}

fn format_compact(value: Option<f64>) -> String {
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

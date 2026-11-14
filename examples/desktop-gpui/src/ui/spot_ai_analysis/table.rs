use super::{SpotAiAnalysisPage, SpotAiAnalysisRow};
use crate::ui::palette;
use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    table::{Column, ColumnSort, TableDelegate, TableState},
};
use std::cmp::Ordering;

#[derive(Clone)]
pub(super) struct SpotAiAnalysisTableDelegate {
    columns: Vec<Column>,
    rows: Vec<SpotAiAnalysisRow>,
    loading: bool,
    page: WeakEntity<SpotAiAnalysisPage>,
}

impl SpotAiAnalysisTableDelegate {
    pub(super) fn new(page: WeakEntity<SpotAiAnalysisPage>) -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "交易对").width(px(130.)).fixed_left().sortable(),
                Column::new("change", "24h 涨跌").width(px(90.)).sortable(),
                Column::new("trend", "趋势判断").width(px(100.)).sortable(),
                Column::new("score", "综合评分").width(px(82.)).sortable(),
                Column::new("turnover", "24h 成交额").width(px(110.)).sortable(),
            ],
            rows: Vec::new(),
            loading: false,
            page,
        }
    }

    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading { self.rows.clear(); }
    }

    fn set_rows(&mut self, rows: Vec<SpotAiAnalysisRow>) {
        self.rows = rows;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.rows.clear();
        self.loading = false;
    }

    fn cell(value: impl Into<SharedString>, color: Hsla) -> AnyElement {
        div().size_full().flex().items_center().px_1().text_size(px(11.)).text_color(color).child(value.into()).into_any_element()
    }

    fn symbol_cell(&self, row_ix: usize, row: &SpotAiAnalysisRow, theme: &gpui_component::Theme) -> AnyElement {
        let page = self.page.clone();
        let symbol = row.product.symbol.clone();
        Button::new(("spot-ai-row", row_ix))
            .ghost()
            .xsmall()
            .label(row.product.symbol.clone())
            .text_color(palette::text_strong(theme))
            .on_click(move |_, _, cx| {
                _ = page.update(cx, |this, cx| this.select_symbol(symbol.clone(), cx));
            })
            .into_any_element()
    }
}

impl TableDelegate for SpotAiAnalysisTableDelegate {
    fn columns_count(&self, _: &App) -> usize { self.columns.len() }
    fn rows_count(&self, _: &App) -> usize { self.rows.len() }
    fn column(&self, col_ix: usize, _: &App) -> &Column { &self.columns[col_ix] }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let theme = cx.theme();
        let Some(row) = self.rows.get(row_ix) else { return Self::cell("", palette::text(theme)); };
        match self.columns[col_ix].key.as_ref() {
            "symbol" => self.symbol_cell(row_ix, row, theme),
            "change" => {
                let value = row.product.price_change_percent.unwrap_or(0.0);
                Self::cell(format!("{value:+.2}%"), if value >= 0.0 { theme.success } else { theme.danger })
            }
            "trend" => Self::cell(row.trend_label.clone(), palette::text(theme)),
            "score" => Self::cell(format!("{:.0}", row.total_score), palette::text(theme)),
            "turnover" => Self::cell(format_compact(row.product.quote_volume), palette::text(theme)),
            _ => Self::cell("", palette::text(theme)),
        }
    }

    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, _: &mut Window, _: &mut Context<TableState<Self>>) {
        let descending = matches!(sort, ColumnSort::Descending);
        let key = self.columns[col_ix].key.to_string();
        self.rows.sort_by(|a, b| {
            let order = match key.as_str() {
                "symbol" => a.product.symbol.cmp(&b.product.symbol),
                "change" => a.product.price_change_percent.partial_cmp(&b.product.price_change_percent).unwrap_or(Ordering::Equal),
                "score" => a.total_score.partial_cmp(&b.total_score).unwrap_or(Ordering::Equal),
                "turnover" => a.product.quote_volume.partial_cmp(&b.product.quote_volume).unwrap_or(Ordering::Equal),
                "trend" => a.trend_label.cmp(&b.trend_label),
                _ => Ordering::Equal,
            };
            if descending { order.reverse() } else { order }
        });
    }

    fn loading(&self, _: &App) -> bool { self.loading }
}

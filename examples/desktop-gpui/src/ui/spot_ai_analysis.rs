use crate::ui::palette;
use binance_tools::binance::market::MarketProduct;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableState},
    v_flex,
};
use std::{cmp::Ordering, collections::HashMap};

const SCORE_WEIGHTS: [f64; 6] = [0.25, 0.20, 0.20, 0.15, 0.10, 0.10];

pub enum SpotAiAnalysisEvent {
    AnalyzeWithAi { prompt: String, display_content: String },
}

#[derive(Clone)]
struct SpotAiAnalysisRow {
    product: MarketProduct,
    trend_score: f64,
    volume_score: f64,
    technical_score: f64,
    risk_score: f64,
    sentiment_score: f64,
    liquidity_score: f64,
    total_score: f64,
    trend_label: String,
    basis: String,
}

impl SpotAiAnalysisRow {
    fn score_values(&self) -> [f64; 6] {
        [
            self.trend_score,
            self.volume_score,
            self.technical_score,
            self.risk_score,
            self.sentiment_score,
            self.liquidity_score,
        ]
    }
}

pub struct SpotAiAnalysisPage {
    table: Entity<TableState<SpotAiAnalysisTableDelegate>>,
    search_input: Entity<InputState>,
    rows: Vec<SpotAiAnalysisRow>,
    selected_symbol: Option<String>,
    error: Option<String>,
    loading: bool,
    scanned_count: usize,
    _load_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SpotAiAnalysisPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(SpotAiAnalysisTableDelegate::new(page), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索交易对、基础资产")
                .default_value("")
        });
        let _subscriptions =
            vec![cx.subscribe_in(&search_input, window, Self::on_search_input_event)];

        let mut this = Self {
            table,
            search_input,
            rows: Vec::new(),
            selected_symbol: None,
            error: None,
            loading: false,
            scanned_count: 0,
            _load_task: Task::ready(()),
            _subscriptions,
        };
        this.scan(cx);
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
            self.refresh_visible_rows(cx);
            cx.notify();
        }
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            table.refresh(cx);
        });
        cx.notify();

        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let products =
                        binance_tools::db::market::load_or_fetch_market_products_blocking()?;
                    let cached_ma = binance_tools::db::spot::load_cached_usdt_daily_ma_signals_blocking(30)
                        .unwrap_or_default();
                    Ok::<_, anyhow::Error>(build_analysis_rows(products, &cached_ma))
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(rows) => {
                        this.scanned_count = rows.len();
                        this.selected_symbol = rows.first().map(|row| row.product.symbol.clone());
                        this.rows = rows;
                        this.refresh_visible_rows(cx);
                        app_log::info!(
                            "[现货AI分析] 扫描完成：交易对={}，使用已有日线均线缓存辅助评分",
                            this.scanned_count
                        );
                    }
                    Err(err) => {
                        this.error = Some(format!("{:#}", err));
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_error();
                            table.refresh(cx);
                        });
                        app_log::error!("[现货AI分析] 扫描失败 error={:#}", err);
                    }
                }
                cx.notify();
            });
        });
    }

    fn select_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        self.selected_symbol = Some(symbol);
        cx.notify();
    }

    fn refresh_visible_rows(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let rows = self
            .rows
            .iter()
            .filter(|row| {
                query.is_empty()
                    || row.product.symbol.to_lowercase().contains(&query)
                    || row.product.base_asset.to_lowercase().contains(&query)
                    || row.product.asset_name.to_lowercase().contains(&query)
            })
            .cloned()
            .collect::<Vec<_>>();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_rows(rows);
            table.refresh(cx);
        });
    }

    fn analyze_selected(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self
            .selected_symbol
            .as_ref()
            .and_then(|symbol| self.rows.iter().find(|row| &row.product.symbol == symbol))
        else {
            self.error = Some("请先选择一个交易对。".to_string());
            cx.notify();
            return;
        };

        let prompt = build_analysis_prompt(row);
        cx.emit(SpotAiAnalysisEvent::AnalyzeWithAi {
            prompt,
            display_content: format!("分析 {} 的现货行情与风险", row.product.symbol),
        });
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self
            .selected_symbol
            .as_ref()
            .and_then(|symbol| self.rows.iter().find(|row| &row.product.symbol == symbol))
        else {
            return v_flex()
                .flex_1()
                .justify_center()
                .items_center()
                .child(div().text_color(palette::muted(cx.theme())).child("选择一个交易对查看研究摘要"))
                .into_any_element();
        };

        let theme = cx.theme();
        let dimensions = [
            ("趋势结构", row.trend_score),
            ("成交活跃度", row.volume_score),
            ("技术指标", row.technical_score),
            ("波动风险", row.risk_score),
            ("市场情绪", row.sentiment_score),
            ("流动性代理", row.liquidity_score),
        ];

        v_flex()
            .flex_1()
            .min_w(px(300.))
            .h_full()
            .gap_3()
            .p_3()
            .rounded(px(8.))
            .bg(palette::surface_strong(theme))
            .border_1()
            .border_color(palette::border(theme))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_size(px(15.)).font_semibold().child(row.product.symbol.clone()))
                            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child(row.product.asset_name.clone())),
                    )
                    .child(
                        Button::new("spot-ai-deep-analysis")
                            .primary()
                            .xsmall()
                            .label("AI 深度分析")
                            .on_click(cx.listener(|this, _, _, cx| this.analyze_selected(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child("综合评分"))
                            .child(div().text_size(px(25.)).font_semibold().child(format!("{:.0} / 100", row.total_score))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child("趋势判断"))
                            .child(div().text_size(px(16.)).font_semibold().child(row.trend_label.clone())),
                    ),
            )
            .child(div().h(px(1.)).bg(palette::border(theme)))
            .child(div().text_size(px(12.)).font_semibold().child("行情摘要"))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(detail_stat("24h 涨跌", format_signed_percent(row.product.price_change_percent), theme))
                    .child(detail_stat("最新价格", format_price(row.product.last_price), theme))
                    .child(detail_stat("24h 成交额", format_compact(row.product.quote_volume), theme))
                    .child(detail_stat("24h 振幅", amplitude_percent(&row.product), theme)),
            )
            .child(div().text_size(px(12.)).font_semibold().child("六维研究评分"))
            .children(dimensions.into_iter().map(|(label, score)| {
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(74.)).text_size(px(11.)).text_color(palette::muted(theme)).child(label))
                    .child(
                        div()
                            .flex_1()
                            .h(px(6.))
                            .rounded(px(3.))
                            .bg(theme.muted.opacity(0.18))
                            .child(
                                div()
                                    .h(px(6.))
                                    .w(px(score.clamp(0.0, 100.0) as f32 * 1.6))
                                    .rounded(px(3.))
                                    .bg(theme.primary.opacity(0.78)),
                            ),
                    )
                    .child(div().w(px(28.)).text_size(px(11.)).text_color(palette::text(theme)).child(format!("{:.0}", score)))
            }))
            .child(div().h(px(1.)).bg(palette::border(theme)))
            .child(div().text_size(px(12.)).font_semibold().child("数据依据与限制"))
            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).line_height(px(17.)).child(row.basis.clone()))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(palette::muted(theme))
                    .line_height(px(17.))
                    .child("第一版使用已有 24 小时行情与本地日线均线缓存；未取得的多周期指标、新闻情绪和盘口深度不会伪造，AI 报告会明确标记数据缺口。"),
            )
            .into_any_element()
    }
}

impl EventEmitter<SpotAiAnalysisEvent> for SpotAiAnalysisPage {}

impl Render for SpotAiAnalysisPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let visible_count = self.table.read(cx).delegate().rows.len();
        let selected_count = self.rows.len();
        let loading = self.loading;

        v_flex()
            .size_full()
            .gap_3()
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded(px(8.))
                    .bg(palette::surface_strong(theme))
                    .border_1()
                    .border_color(palette::border(theme))
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(div().text_size(px(16.)).font_semibold().child("AI 分析"))
                                    .child(div().text_size(px(12.)).text_color(palette::muted(theme)).child(format!("现货市场研究 · USDT 交易对 · 已载入 {} 个候选币种", selected_count))),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("spot-ai-scan")
                                            .outline()
                                            .xsmall()
                                            .label("重新扫描")
                                            .loading(loading)
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| this.scan(cx))),
                                    )
                                    .child(
                                        Button::new("spot-ai-run")
                                            .primary()
                                            .xsmall()
                                            .label("分析所选币种")
                                            .disabled(loading || selected_count == 0)
                                            .on_click(cx.listener(|this, _, _, cx| this.analyze_selected(cx))),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child(format!("当前显示 {} 个", visible_count)))
                                    .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child("评分由本地行情规则计算")),
                            )
                            .child(div().w(px(280.)).max_w(px(340.)).child(Input::new(&self.search_input).small().cleanable(true))),
                    ),
            )
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(palette::error_background())
                        .border_1()
                        .border_color(palette::error_border())
                        .text_color(palette::error_text())
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(420.))
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(400.))
                            .h_full()
                            .overflow_hidden()
                            .child(
                                DataTable::new(&self.table)
                                    .stripe(true)
                                    .bordered(true)
                                    .scrollbar_visible(true, true),
                            ),
                    )
                    .child(self.render_detail(cx)),
            )
    }
}

fn build_analysis_rows(
    products: Vec<MarketProduct>,
    cached_ma: &[binance_tools::binance::spot::DailyMaSignal],
) -> Vec<SpotAiAnalysisRow> {
    let usdt = products
        .into_iter()
        .filter(|product| product.quote_asset == "USDT" && product.is_trading && !product.is_etf)
        .collect::<Vec<_>>();
    let mut volumes = usdt.iter().filter_map(|product| product.quote_volume).collect::<Vec<_>>();
    volumes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let ma_by_symbol = cached_ma.iter().map(|signal| (signal.symbol.as_str(), signal)).collect::<HashMap<_, _>>();

    let mut rows = usdt.into_iter().map(|product| {
        let change = product.price_change_percent.unwrap_or(0.0);
        let trend_score = (50.0 + change * 5.0).clamp(0.0, 100.0);
        let quote_volume = product.quote_volume.unwrap_or(0.0);
        let volume_rank = percentile_rank(&volumes, quote_volume);
        let volume_score = (volume_rank * 100.0).clamp(0.0, 100.0);
        let ma_signal = ma_by_symbol.get(product.symbol.as_str()).copied();
        let technical_score = ma_signal
            .map(|signal| (55.0 + signal.difference_percent * 4.0).clamp(0.0, 100.0))
            .unwrap_or(50.0);
        let amplitude = product.high_price.zip(product.low_price)
            .filter(|(_, low)| *low > 0.0)
            .map(|(high, low)| (high - low) / low * 100.0)
            .unwrap_or(0.0);
        let risk_score = (100.0 - amplitude * 4.0).clamp(0.0, 100.0);
        // No reliable news sentiment source is wired into this first iteration.
        let sentiment_score = 50.0;
        let liquidity_score = volume_score;
        let scores = [trend_score, volume_score, technical_score, risk_score, sentiment_score, liquidity_score];
        let total_score = scores.iter().zip(SCORE_WEIGHTS).map(|(score, weight)| score * weight).sum();
        let trend_label = if change >= 3.0 { "趋势偏强" } else if change >= 0.8 { "温和走强" } else if change <= -3.0 { "趋势偏弱" } else if change <= -0.8 { "温和走弱" } else { "区间震荡" }.to_string();
        let basis = match ma_signal {
            Some(signal) => format!("24 小时涨跌幅 {:+.2}%；近 30 日收盘均价偏离 {:+.2}%，样本 {} 根。成交额分数为当前 USDT 候选币种中的相对排名。", change, signal.difference_percent, signal.samples),
            None => format!("24 小时涨跌幅 {:+.2}%；当前没有该币种的完整日线均线缓存。成交额分数为当前 USDT 候选币种中的相对排名。", change),
        };
        SpotAiAnalysisRow {
            product,
            trend_score,
            volume_score,
            technical_score,
            risk_score,
            sentiment_score,
            liquidity_score,
            total_score,
            trend_label,
            basis,
        }
    }).collect::<Vec<_>>();
    rows.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(Ordering::Equal));
    rows
}

fn percentile_rank(sorted: &[f64], value: f64) -> f64 {
    if sorted.is_empty() { return 0.5; }
    let below = sorted.partition_point(|candidate| *candidate <= value);
    below as f64 / sorted.len() as f64
}

fn build_analysis_prompt(row: &SpotAiAnalysisRow) -> String {
    let product = &row.product;
    format!(
        "你是一名严谨的加密货币市场研究分析师。请用中文输出结构清晰、内容充分的研究报告，不要给出保证收益或直接下单指令。只能依据下列数据，不得编造 K 线、指标、支撑阻力、新闻或盘口数据。缺失数据必须明确写“数据不足”。\n\n交易对：{}\n最新价：{}\n24小时涨跌：{}\n24小时最高：{}\n24小时最低：{}\n24小时成交量：{}\n24小时成交额：{}\n日线均线缓存偏离：{}\n六维评分（Rust 预计算）：趋势 {:.1}、成交活跃度 {:.1}、技术指标 {:.1}、波动风险 {:.1}、市场情绪 {:.1}、流动性代理 {:.1}；加权总分 {:.1}/100。\n数据限制：当前未接入 15m/1h/4h 完整指标、新闻情绪和实时盘口深度。成交额排名仅代表本批币种的相对活跃程度，不代表成交量放大或资金净流入。\n\n请按以下章节回答：一、核心结论；二、趋势分析；三、成交量与活跃度；四、技术指标（说明缺失项）；五、风险分析；六、市场情绪（无新闻数据时明确说明）；七、流动性（说明盘口数据缺失）；八、潜在机会；九、主要风险；十、需要进一步确认的条件。对评分偏高或偏低的原因给出解释，不要将评分直接当作买卖信号。",
        product.symbol,
        format_price(product.last_price),
        format_signed_percent(product.price_change_percent),
        format_price(product.high_price),
        format_price(product.low_price),
        format_compact(product.volume),
        format_compact(product.quote_volume),
        row.basis,
        row.trend_score,
        row.volume_score,
        row.technical_score,
        row.risk_score,
        row.sentiment_score,
        row.liquidity_score,
        row.total_score
    )
}

fn detail_stat(label: &'static str, value: String, theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(90.))
        .gap_1()
        .p_2()
        .rounded(px(6.))
        .bg(theme.muted.opacity(0.08))
        .child(div().text_size(px(10.)).text_color(palette::muted(theme)).child(label))
        .child(div().text_size(px(12.)).font_semibold().child(value))
        .into_any_element()
}

fn format_signed_percent(value: Option<f64>) -> String {
    value.map(|value| format!("{value:+.2}%")).unwrap_or_else(|| "-".to_string())
}

fn format_price(value: Option<f64>) -> String {
    let Some(value) = value else { return "-".to_string(); };
    if value >= 100.0 { format!("{value:.2}") } else if value >= 1.0 { format!("{value:.4}") } else { format!("{value:.8}").trim_end_matches('0').trim_end_matches('.').to_string() }
}

fn format_compact(value: Option<f64>) -> String {
    let Some(value) = value else { return "-".to_string(); };
    let abs = value.abs();
    if abs >= 1_000_000_000.0 { format!("{:.2}B", value / 1_000_000_000.0) }
    else if abs >= 1_000_000.0 { format!("{:.2}M", value / 1_000_000.0) }
    else if abs >= 1_000.0 { format!("{:.2}K", value / 1_000.0) }
    else { format!("{value:.2}") }
}

fn amplitude_percent(product: &MarketProduct) -> String {
    product.high_price.zip(product.low_price)
        .filter(|(_, low)| *low > 0.0)
        .map(|(high, low)| format!("{:.2}%", (high - low) / low * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

#[derive(Clone)]
struct SpotAiAnalysisTableDelegate {
    columns: Vec<Column>,
    rows: Vec<SpotAiAnalysisRow>,
    loading: bool,
    page: WeakEntity<SpotAiAnalysisPage>,
}

impl SpotAiAnalysisTableDelegate {
    fn new(page: WeakEntity<SpotAiAnalysisPage>) -> Self {
        Self {
            columns: vec![
                Column::new("symbol", "交易对").width(px(130.)).fixed_left().sortable(),
                Column::new("change", "24h 涨跌").width(px(90.)).sortable(),
                Column::new("trend", "趋势判断").width(px(100.)).sortable(),
                Column::new("score", "AI 评分").width(px(82.)).sortable(),
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

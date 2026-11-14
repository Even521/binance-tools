use crate::ui::palette;
#[path = "table.rs"]
mod table;
use self::table::SpotAiAnalysisTableDelegate;
use binance_tools::binance::market::MarketProduct;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Table as DataTable, TableState},
    v_flex,
};
use spot_ai_analysis::{AnalysisResult, DailyMaSnapshot, MarketSnapshot, analyze_market, build_report_prompt};
use std::collections::HashMap;

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
                        binance_tools::db::market::refresh_market_products_blocking()?;
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
            ("日线均价偏离", row.technical_score),
            ("波动风险", row.risk_score),
            ("市场情绪", row.sentiment_score),
            ("流动性参考", row.liquidity_score),
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
                    .child("第一版使用已有 24 小时行情与本地日线均线缓存；日线均价偏离是技术参考，不代表完整技术指标。成交额排名和振幅仅是流动性参考，不等于盘口深度。未取得的多周期指标、新闻情绪和盘口数据不会伪造，AI 报告会明确标记数据缺口。"),
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
    let inputs = products.iter().map(|p| MarketSnapshot {
        symbol:p.symbol.clone(), base_asset:p.base_asset.clone(), asset_name:p.asset_name.clone(),
        quote_asset:p.quote_asset.clone(), is_trading:p.is_trading, is_etf:p.is_etf,
        price_change_percent:p.price_change_percent, quote_volume:p.quote_volume,
        last_price:p.last_price, high_price:p.high_price, low_price:p.low_price, volume:p.volume,
    }).collect::<Vec<_>>();
    let ma = cached_ma.iter().map(|s| DailyMaSnapshot {
        symbol:s.symbol.clone(), difference_percent:s.difference_percent, samples:s.samples,
    }).collect::<Vec<_>>();
    let scores = analyze_market(&inputs,&ma).into_iter()
        .map(|s|(s.symbol.clone(),s)).collect::<HashMap<_,_>>();
    let mut rows=products.into_iter().filter_map(|product| {
        let s=scores.get(&product.symbol)?;
        Some(SpotAiAnalysisRow {
            product, trend_score:s.trend_score, volume_score:s.volume_score,
            technical_score:s.technical_score, risk_score:s.risk_score,
            sentiment_score:s.sentiment_score, liquidity_score:s.liquidity_score,
            total_score:s.total_score, trend_label:s.trend_label.clone(), basis:s.basis.clone(),
        })
    }).collect::<Vec<_>>();
    rows.sort_by(|a,b|b.total_score.partial_cmp(&a.total_score).unwrap_or(Ordering::Equal));
    rows
}

fn build_analysis_prompt(row:&SpotAiAnalysisRow)->String {
    let market=MarketSnapshot {
        symbol:row.product.symbol.clone(), base_asset:row.product.base_asset.clone(),
        asset_name:row.product.asset_name.clone(), quote_asset:row.product.quote_asset.clone(),
        is_trading:row.product.is_trading, is_etf:row.product.is_etf,
        price_change_percent:row.product.price_change_percent, quote_volume:row.product.quote_volume,
        last_price:row.product.last_price, high_price:row.product.high_price,
        low_price:row.product.low_price, volume:row.product.volume,
    };
    let analysis=AnalysisResult {
        symbol:row.product.symbol.clone(), trend_score:row.trend_score, volume_score:row.volume_score,
        technical_score:row.technical_score, risk_score:row.risk_score,
        sentiment_score:row.sentiment_score, liquidity_score:row.liquidity_score,
        total_score:row.total_score, trend_label:row.trend_label.clone(), basis:row.basis.clone(),
    };
    build_report_prompt(&market,&analysis)
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


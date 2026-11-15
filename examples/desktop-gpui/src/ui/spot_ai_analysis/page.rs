use crate::ui::palette;
#[path = "table.rs"]
mod table;
use self::table::SpotAiAnalysisTableDelegate;
use binance_tools::binance::{BinanceSettings, market::MarketProduct};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    table::{Table as DataTable, TableState},
    tooltip::Tooltip,
    v_flex,
};
use spot_ai_analysis::{AnalysisResult, BacktestCandle, BacktestResult, DailyKlineSnapshot, DailyMaSnapshot, MarketSentiment, MarketSnapshot, analyze_market_with_sentiment, build_report_prompt, run_backtest};
use std::{cmp::Ordering, collections::HashMap, sync::{Mutex, OnceLock}, time::{Duration, Instant}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpotAiView { QuantSelection, Backtest }

#[derive(Clone, Copy, Debug)]
enum SpotLoadMode { Cache, Sync, Refresh }

pub enum SpotAiAnalysisEvent {
    AnalyzeWithAi { prompt: String, display_content: String },
    OpenKline(String),
}

#[derive(Clone)]
struct SpotAiAnalysisRow {
    product: MarketProduct,
    trend_score: f64,
    volume_score: f64,
    technical_score: f64,
    risk_score: f64,
    sentiment_score: f64,
    sentiment_value: f64,
    sentiment_classification: String,
    sentiment_update_time: String,
    liquidity_score: f64,
    total_score: f64,
    data_completeness: f64,
    trend_label: String,
    basis: String,
    ma120_average_price: Option<f64>,
    ma120_difference_percent: Option<f64>,
    ma120_samples: usize,
}

impl SpotAiAnalysisRow {
}

pub struct SpotAiAnalysisPage {
    view: SpotAiView,
    table: Entity<TableState<SpotAiAnalysisTableDelegate>>,
    search_input: Entity<InputState>,
    rows: Vec<SpotAiAnalysisRow>,
    selected_symbol: Option<String>,
    ma120_filter: String,
    ma120_filter_open: bool,
    error: Option<String>,
    loading: bool,
    scanned_count: usize,
    backtest_result: Option<BacktestResult>,
    backtest_loading: bool,
    _backtest_task: Task<()>,
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
            view: SpotAiView::QuantSelection,
            table,
            search_input,
            rows: Vec::new(),
            selected_symbol: None,
            ma120_filter: "全部".to_string(),
            ma120_filter_open: false,
            error: None,
            loading: false,
            scanned_count: 0,
            backtest_result: None,
            backtest_loading: false,
            _backtest_task: Task::ready(()),
            _load_task: Task::ready(()),
            _subscriptions,
        };
        this.load_data(SpotLoadMode::Cache, cx);
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

    fn load_data(&mut self, mode: SpotLoadMode, cx: &mut Context<Self>) {
        if self.loading { return; }
        self.loading = true;
        self.error = None;
        cx.notify();
        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let products = match mode {
                    SpotLoadMode::Cache => {
                        let cached = binance_tools::db::market::load_cached_market_products_blocking()?;
                        if cached.is_empty() { binance_tools::db::market::load_or_fetch_market_products_blocking()? } else { cached }
                    }
                    SpotLoadMode::Sync => binance_tools::db::market::load_or_fetch_market_products_blocking()?,
                    SpotLoadMode::Refresh => binance_tools::db::market::refresh_market_products_blocking()?,
                };
                let cached_ma = binance_tools::db::spot::load_cached_usdt_daily_ma_signals_blocking(30).unwrap_or_default();
                let cached_ma120 = if matches!(mode, SpotLoadMode::Sync) {
                    match binance_tools::db::spot::load_or_fetch_usdt_daily_ma_signals_blocking(BinanceSettings::production(), 120) {
                        Ok(signals) => signals,
                        Err(error) => {
                            app_log::warn!("[现货AI分析] MA120 同步失败，使用本地缓存 error={:#}", error);
                            binance_tools::db::spot::load_cached_usdt_daily_ma_signals_blocking(120).unwrap_or_default()
                        }
                    }
                } else { binance_tools::db::spot::load_cached_usdt_daily_ma_signals_blocking(120).unwrap_or_default() };
                let cached_daily = binance_tools::db::spot::load_cached_usdt_daily_klines_blocking().unwrap_or_default();
                let sentiment = if matches!(mode, SpotLoadMode::Cache) { None } else { fetch_cmc_sentiment().ok() };
                let rows = build_analysis_rows(products, &cached_ma, &cached_ma120, &cached_daily, sentiment.as_ref());
                Ok::<_, anyhow::Error>((rows, sentiment))
            }).await;
            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((rows, sentiment)) => {
                        this.scanned_count = rows.len();
                        if this.selected_symbol.as_ref().map_or(true, |symbol| !rows.iter().any(|row| &row.product.symbol == symbol)) {
                            this.selected_symbol = rows.first().map(|row| row.product.symbol.clone());
                        }
                        this.rows = rows;
                        if let Some(s) = sentiment.as_ref() {
                            app_log::info!("[现货AI分析] 恐惧与贪婪指数={} 分类={} 更新时间={}", s.value, s.classification, s.update_time);
                        }
                        this.refresh_visible_rows(cx);
                        app_log::info!("[现货AI分析] 数据加载完成 mode={:?} rows={}", mode, this.scanned_count);
                    }
                    Err(err) => { this.error = Some(format!("{:#}", err)); app_log::error!("[现货AI分析] 数据加载失败 mode={:?} error={:#}", mode, err); }
                }
                cx.notify();
            });
        });
    }
    fn sync_data(&mut self, cx: &mut Context<Self>) { self.load_data(SpotLoadMode::Sync, cx); }
    fn refresh_data(&mut self, cx: &mut Context<Self>) { self.load_data(SpotLoadMode::Refresh, cx); }

    fn select_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        self.selected_symbol = Some(symbol);
        cx.notify();
    }

    fn set_ma120_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        self.ma120_filter = filter;
        self.ma120_filter_open = false;
        self.refresh_visible_rows(cx);
        cx.notify();
    }

    fn refresh_visible_rows(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let ma120_filter = self.ma120_filter.clone();
        let rows = self
            .rows
            .iter()
            .filter(|row| {
                let matches_query = query.is_empty()
                    || row.product.symbol.to_lowercase().contains(&query)
                    || row.product.base_asset.to_lowercase().contains(&query)
                    || row.product.asset_name.to_lowercase().contains(&query);
                let matches_ma120 = match ma120_filter.as_str() {
                    "线上" => row.ma120_difference_percent.is_some_and(|v| v > 1.0),
                    "趋近" => row.ma120_difference_percent.is_some_and(|v| (-1.0..=1.0).contains(&v)),
                    "线下" => row.ma120_difference_percent.is_some_and(|v| v < -1.0),
                    _ => true,
                };
                matches_query && matches_ma120
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
            (
                "趋势与动量",
                row.trend_score,
                "权重 30%。使用已收盘日线的 1/3/7 日收益率，并结合价格相对 MA30、MA30 相对 MA120 的结构。历史不足时该因子不计入总分。",
            ),
            (
                "成交活跃度",
                row.volume_score,
                "权重 20%。结合 24 小时成交额在候选币中的百分位，以及缓存日线近 20 日平均成交额。无 20 日历史时只使用可用的相对成交额排名。",
            ),
            (
                "入场位置",
                row.technical_score,
                "权重 15%。评估价格相对 MA30 的偏离，以及相对近 20 日高点的位置；偏离过大或追高会降低分数。缺少足够日线历史时不参与总分。",
            ),
            (
                "风险控制",
                row.risk_score,
                "权重 20%。根据 ATR14 相对价格的比例和近 7 日最大回撤评估波动风险。历史数据不足时不参与总分。",
            ),
            (
                "市场情绪",
                row.sentiment_score,
                "权重 5%。使用 CoinMarketCap 恐惧与贪婪指数；请求失败时该因子不计入总分，不使用中性占位冒充有效数据。",
            ),
            (
                "流动性参考",
                row.liquidity_score,
                "权重 10%。当前使用 24 小时 USDT 成交额的相对排名作为流动性代理；尚未接入盘口深度与买卖价差，不能视为真实盘口流动性。",
            ),
        ];

        v_flex()
            .flex_1()
            .w_full()
            .h_full()
            .overflow_y_scrollbar()
            .gap_4()
            .p_4()
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
                            .child(div().text_size(px(18.)).font_bold().text_color(palette::text_strong(theme)).child(row.product.symbol.clone()))
                            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child(row.product.asset_name.clone()))
                            .child(div().text_size(px(10.)).text_color(palette::muted_soft(theme)).child("现货研究 · USDT")),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("spot-ai-deep-analysis")
                                    .primary()
                                    .xsmall()
                                    .label("AI 深度分析")
                                    .on_click(cx.listener(|this, _, _, cx| this.analyze_selected(cx))),
                            ),
                    ),
            )
            .child(div().h(px(1.)).bg(palette::border(theme)))
            .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("行情摘要"))
            .child(
                h_flex()
                    .gap_2()
                    .child(detail_stat_colored("V2 综合评分", format!("{:.0} / 100", row.total_score), score_color(row.total_score, theme), theme))
                    .child(detail_stat("数据完整度", format!("{:.0}%", row.data_completeness), theme))
                    .child(detail_stat_colored("恐惧与贪婪", format!("{:.0} · {}", row.sentiment_value, row.sentiment_classification), score_color(row.sentiment_value, theme), theme))
            )
            .child(div().text_size(px(10.)).text_color(palette::muted(theme)).child(format!("CoinMarketCap · 北京时间 {}", format_beijing_time(&row.sentiment_update_time))))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(detail_stat_colored("24h 涨跌", format_signed_percent(row.product.price_change_percent), change_color(row.product.price_change_percent, theme), theme))
                    .child(detail_stat("最新价格", format_price(row.product.last_price), theme))
                    .child(detail_stat("24h 成交额", format_compact(row.product.quote_volume), theme))
                    .child(detail_stat("24h 振幅", amplitude_percent(&row.product), theme))
                    .child(detail_stat("120日均线价", format_price(row.ma120_average_price), theme))
                    .child(detail_stat_colored(
                        "120日线位置",
                        format_ma120_location(row.ma120_difference_percent),
                        row.ma120_difference_percent
                            .map(|value| change_color(Some(value), theme))
                            .unwrap_or_else(|| palette::muted(theme)),
                        theme,
                    ))
            )
            .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("六维研究评分"))
            .children(dimensions.into_iter().map(|(label, score, explanation)| {
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id(label)
                            .w(px(74.))
                            .text_size(px(11.))
                            .text_color(palette::text(theme))
                            .child(format!("{label} ⓘ"))
                            .tooltip(move |_, cx| {
                                cx.new(|_| Tooltip::new(explanation)).into()
                            }),
                    )
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
                                    .bg(score_color(score, theme)),
                            ),
                    )
                    .child(div().w(px(28.)).text_size(px(11.)).font_semibold().text_color(score_color(score, theme)).child(format!("{:.0}", score)))
            }))
            .child(div().h(px(1.)).bg(palette::border(theme)))
            .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("数据依据与限制"))
            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).line_height(px(20.)).child(row.basis.clone()))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(palette::muted(theme))
                    .line_height(px(17.))
                    .child("使用已有 24 小时行情、30 日均线评分与 120 日均线位置参考；均线偏离是技术参考，不代表完整技术指标。成交额排名和振幅仅是流动性参考，不等于盘口深度。未取得的多周期指标、新闻情绪和盘口数据不会伪造，AI 报告会明确标记数据缺口。"),
            )
            .into_any_element()
    }

    fn set_view(&mut self, view: SpotAiView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    fn render_quant_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let visible_count = self.table.read(cx).delegate().visible_rows_count();
        let selected_count = self.rows.len();

        v_flex()
            .flex_1()
            .min_h(px(420.))
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child(format!("当前显示 {} 个 / 共 {} 个候选币种", visible_count, selected_count)))
                            .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child("1–7 天波段 · 按综合评分排序")),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                v_flex()
                                    .relative()
                                    .child(
                                        Button::new("spot-ai-ma120-filter")
                                            .outline()
                                            .xsmall()
                                            .label(format!("120日线：{} ▾", self.ma120_filter))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.ma120_filter_open = !this.ma120_filter_open;
                                                cx.notify();
                                            })),
                                    )
                                    .when(self.ma120_filter_open, |this| {
                                        this.child(
                                            v_flex()
                                                .absolute()
                                                .top(px(28.))
                                                .right_0()
                                                .w(px(116.))
                                                .p_1()
                                                .gap_1()
                                                .rounded(px(6.))
                                                .bg(palette::surface_strong(theme))
                                                .border_1()
                                                .border_color(palette::border(theme))
                                                .shadow_lg()
                                                .children(["全部", "线上", "趋近", "线下"].into_iter().map(|filter| {
                                                    let selected = self.ma120_filter == filter;
                                                    Button::new(SharedString::from(format!("spot-ai-ma120-option-{filter}")))
                                                        .ghost()
                                                        .xsmall()
                                                        .w_full()
                                                        .label(filter)
                                                        .when(selected, |button| button.bg(theme.primary.opacity(0.14)).text_color(palette::text_strong(theme)))
                                                        .on_click(cx.listener(move |this, _, _, cx| this.set_ma120_filter(filter.to_string(), cx)))
                                                })),
                                        )
                                    }),
                            )
                            .child(div().w(px(280.)).max_w(px(340.)).child(Input::new(&self.search_input).small().cleanable(true))),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(360.))
                    .w_full()
                    .gap_3()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(500.))
                            .h_full()
                            .overflow_hidden()
                            .child(
                                DataTable::new(&self.table)
                                    .stripe(true)
                                    .bordered(true)
                                    .scrollbar_visible(true, true),
                            ),
                    )
                    .child(
                        div()
                            .w(px(410.))
                            .min_w(px(340.))
                            .h_full()
                            .overflow_hidden()
                            .child(self.render_detail(cx)),
                    ),
            )
            .into_any_element()
    }

    fn run_backtest(&mut self, cx: &mut Context<Self>) {
        if self.backtest_loading { return; }
        self.backtest_loading = true;
        self.error = None;
        cx.notify();
        self._backtest_task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move {
                let cached = binance_tools::db::spot::load_cached_usdt_daily_klines_for_backtest_blocking()?;
                let mut candles = Vec::new();
                for (symbol, rows) in cached {
                    if !symbol.ends_with("USDT") { continue; }
                    let base_asset = symbol.trim_end_matches("USDT").to_string();
                    for row in rows {
                        candles.push(BacktestCandle {
                            symbol: symbol.clone(), base_asset: base_asset.clone(),
                            asset_name: base_asset.clone(), open_time: row.open_time,
                            high: row.high_price, low: row.low_price, close: row.close_price,
                            quote_turnover: row.close_price * row.volume,
                        });
                    }
                }
                if candles.is_empty() {
                    anyhow::bail!("本地 SQLite 没有已收盘的 USDT 日线数据，请先在量化选币页点击“同步”。");
                }
                Ok::<_, anyhow::Error>(run_backtest(&candles, 180))
            }).await;
            _ = this.update(cx, |this, cx| {
                this.backtest_loading = false;
                match result {
                    Ok(result) if result.points.is_empty() => {
                        this.backtest_result = Some(result);
                        this.error = Some("可用历史不足：回测需要至少 30 根预热日线，并且至少 2 个连续评估交易日。请先同步更长时间的日线历史。".to_string());
                    }
                    Ok(result) => {
                        app_log::info!("[现货回测] 完成 symbols={} points={} return={:.2}% max_drawdown={:.2}%",
                            result.data_symbols, result.points.len(), result.total_return_percent, result.max_drawdown_percent);
                        this.backtest_result = Some(result);
                    }
                    Err(error) => {
                        this.error = Some(format!("回测失败：{:#}", error));
                        app_log::error!("[现货回测] 执行失败 error={:#}", error);
                    }
                }
                cx.notify();
            });
        });
    }

    fn render_backtest_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let result = self.backtest_result.as_ref();
        let mut view = v_flex()
            .flex_1()
            .w_full()
            .min_h(px(420.))
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .bg(palette::surface_strong(theme))
            .border_1()
            .border_color(palette::border(theme))
            .child(h_flex().items_center().justify_between()
                .child(div().text_size(px(16.)).font_semibold().text_color(palette::text_strong(theme)).child("波段策略回测中心"))
                .child(Button::new("spot-ai-run-backtest").primary().small()
                    .label(if self.backtest_loading { "回测计算中…" } else { "运行最近 180 天回测" })
                    .loading(self.backtest_loading).disabled(self.backtest_loading)
                    .on_click(cx.listener(|this, _, _, cx| this.run_backtest(cx)))))
            .child(div().text_size(px(11.)).text_color(palette::muted(theme))
                .child("V2 六因子横截面排名 · 每日收盘评分、下一交易日执行 · 前 5 名等权 · 单次往返成本估算 0.30% · BTC 买入持有基准"));
        if let Some(result) = result {
            view = view
                .child(h_flex().gap_2()
                    .child(detail_stat_colored("策略净收益", format!("{:+.2}%", result.total_return_percent), change_color(Some(result.total_return_percent), theme), theme))
                    .child(detail_stat_colored("BTC 基准", format!("{:+.2}%", result.benchmark_return_percent), change_color(Some(result.benchmark_return_percent), theme), theme))
                    .child(detail_stat_colored("最大回撤", format!("{:.2}%", result.max_drawdown_percent), theme.danger, theme))
                    .child(detail_stat("胜率", format!("{:.1}%", result.win_rate_percent), theme))
                    .child(detail_stat("Profit Factor", if result.profit_factor.is_finite() { format!("{:.2}", result.profit_factor) } else { "∞".into() }, theme)))
                .child(div().text_size(px(11.)).text_color(palette::muted(theme))
                    .child(format!("区间 {} 至 {} · 数据币种 {} · 评估日 {} · 有持仓日 {}",
                        result.start_date, result.end_date, result.data_symbols, result.trade_days, result.selected_days)))
                .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("账户资金曲线（柱形趋势）"))
                .child(render_backtest_bars(&result.points, false, theme))
                .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("BTC 基准曲线"))
                .child(render_backtest_bars(&result.points, true, theme))
                .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("策略回撤"))
                .child(render_drawdown_bars(&result.points, theme));
        } else {
            view = view
                .child(div().h(px(1.)).bg(palette::border(theme)))
                .child(div().text_size(px(12.)).font_semibold().text_color(palette::text_strong(theme)).child("回测规则"))
                .child(div().text_size(px(12.)).text_color(palette::muted(theme)).line_height(px(21.)).child("回测读取 SQLite 中最多 320 根已收盘日线；前 30 根用于指标预热，最近 180 天用于评估。每个交易日只使用当日及之前的数据计算 V2 分数，完整度低于 70% 的标的不参与排名。"))
                .child(div().text_size(px(12.)).text_color(palette::muted(theme)).line_height(px(21.)).child("每天选取综合评分前 5 名，等权持有至下一根日线；未选出标的时持有现金。成本按买卖手续费与滑点合计 0.30%/次往返估算。"));
        }
        view.into_any_element()
    }

}

impl EventEmitter<SpotAiAnalysisEvent> for SpotAiAnalysisPage {}

impl Render for SpotAiAnalysisPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
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
                                    .child(div().text_size(px(16.)).font_semibold().child("现货量化分析"))
                                    .child(div().text_size(px(12.)).text_color(palette::muted(theme)).child(format!("USDT 现货市场 · 已载入 {} 个候选币种 · 波段周期 1–7 天", selected_count))),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("spot-ai-sync")
                                            .outline()
                                            .xsmall()
                                            .label("同步")
                                            .loading(loading)
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| this.sync_data(cx))),
                                    )
                                    .child(
                                        Button::new("spot-ai-refresh")
                                            .outline()
                                            .xsmall()
                                            .label("刷新")
                                            .disabled(loading)
                                            .on_click(cx.listener(|this, _, _, cx| this.refresh_data(cx))),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("spot-ai-tab-quant")
                                    .outline()
                                    .xsmall()
                                    .label("量化选币")
                                    .when(self.view == SpotAiView::QuantSelection, |button| button.primary())
                                    .on_click(cx.listener(|this, _, _, cx| this.set_view(SpotAiView::QuantSelection, cx))),
                            )
                            .child(
                                Button::new("spot-ai-tab-backtest")
                                    .outline()
                                    .xsmall()
                                    .label("回测中心")
                                    .when(self.view == SpotAiView::Backtest, |button| button.primary())
                                    .on_click(cx.listener(|this, _, _, cx| this.set_view(SpotAiView::Backtest, cx))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(div().text_size(px(11.)).text_color(palette::muted(theme)).child("左侧筛选排名 · 右侧实时查看所选币种研究")),
                            ),
                    )
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
            .child(match self.view {
                SpotAiView::QuantSelection => self.render_quant_view(cx),
                SpotAiView::Backtest => self.render_backtest_view(cx),
            })
    }
}

fn render_backtest_bars(points: &[spot_ai_analysis::BacktestPoint], benchmark: bool, theme: &gpui_component::Theme) -> AnyElement {
    if points.is_empty() { return div().child("暂无回测数据").into_any_element(); }
    let stride = (points.len() / 60).max(1);
    let values = points.iter().step_by(stride).map(|p| if benchmark { p.benchmark } else { p.equity }).collect::<Vec<_>>();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(1.0);
    h_flex().w_full().h(px(150.)).items_end().gap(px(2.)).children(values.into_iter().map(|value| {
        let height = (18.0 + (value - min) / range * 112.0) as f32;
        div().flex_1().min_w(px(1.)).h(px(height)).rounded(px(2.))
            .bg(if benchmark { theme.primary } else { theme.success }.opacity(0.78))
    })).into_any_element()
}

fn render_drawdown_bars(points: &[spot_ai_analysis::BacktestPoint], theme: &gpui_component::Theme) -> AnyElement {
    let stride = (points.len() / 60).max(1);
    h_flex().w_full().h(px(90.)).items_end().gap(px(2.)).children(points.iter().step_by(stride).map(|point| {
        let height = (point.drawdown_percent.abs().min(100.0) / 100.0 * 78.0).max(2.0) as f32;
        div().flex_1().min_w(px(1.)).h(px(height)).rounded_t(px(2.)).bg(theme.danger.opacity(0.78))
    })).into_any_element()
}

fn build_analysis_rows(
    products: Vec<MarketProduct>,
    cached_ma: &[binance_tools::binance::spot::DailyMaSignal],
    cached_ma120: &[binance_tools::binance::spot::DailyMaSignal],
    cached_daily: &HashMap<String, Vec<binance_tools::binance::spot::SpotDailyKline>>,
    sentiment: Option<&MarketSentiment>,
) -> Vec<SpotAiAnalysisRow> {
    let inputs = products.iter().map(|p| MarketSnapshot {
        symbol:p.symbol.clone(), base_asset:p.base_asset.clone(), asset_name:p.asset_name.clone(),
        quote_asset:p.quote_asset.clone(), is_trading:p.is_trading, is_etf:p.is_etf,
        price_change_percent:p.price_change_percent, quote_volume:p.quote_volume,
        last_price:p.last_price, high_price:p.high_price, low_price:p.low_price, volume:p.volume,
        daily_klines: cached_daily.get(&p.symbol).into_iter().flatten().map(|k| DailyKlineSnapshot {
            high:k.high_price, low:k.low_price, close:k.close_price, quote_turnover:k.close_price*k.volume,
        }).collect(),
    }).collect::<Vec<_>>();
    let ma = cached_ma.iter().map(|s| DailyMaSnapshot {
        symbol:s.symbol.clone(), difference_percent:s.difference_percent, samples:s.samples,
    }).collect::<Vec<_>>();
    let ma120 = cached_ma120.iter()
        .map(|signal| (signal.symbol.as_str(), signal))
        .collect::<HashMap<_, _>>();
    let scores = analyze_market_with_sentiment(&inputs,&ma,sentiment).into_iter()
        .map(|s|(s.symbol.clone(),s)).collect::<HashMap<_,_>>();
    let mut rows=products.into_iter().filter_map(|product| {
        let s=scores.get(&product.symbol)?;
        let ma120_signal = ma120
            .get(product.symbol.as_str())
            .copied()
            .filter(|signal| signal.samples >= 120);        let ma120_average_price = ma120_signal.map(|signal| signal.average_price);
        let ma120_samples = ma120_signal.map(|signal| signal.samples).unwrap_or(0);
        let ma120_difference_percent = ma120_signal
            .filter(|signal| signal.samples >= 120)
            .and_then(|signal| product.last_price.zip(Some(signal.average_price)))
            .filter(|(_, average)| *average > 0.0)
            .map(|(price, average)| (price - average) / average * 100.0);
        let ma120_basis = ma120_difference_percent
            .map(|difference| format!("120 日均线：均线价 {}，当前价偏离 {:+.2}%，有效收盘样本 {} 根。", format_price(ma120_average_price), difference, ma120_samples))
            .unwrap_or_else(|| "120 日均线：数据不足，尚无完整的 120 根已收盘日线缓存。".to_string());
        Some(SpotAiAnalysisRow {
            product, trend_score:s.trend_score, volume_score:s.volume_score,
            technical_score:s.technical_score, risk_score:s.risk_score,
            sentiment_score:s.sentiment_score, sentiment_value:s.sentiment_value,
            sentiment_classification:s.sentiment_classification.clone(), sentiment_update_time:s.sentiment_update_time.clone(),
            liquidity_score:s.liquidity_score,
            total_score:s.total_score, data_completeness:s.data_completeness, trend_label:s.trend_label.clone(),
            basis:format!("{} {}", s.basis, ma120_basis),
            ma120_average_price, ma120_difference_percent, ma120_samples,
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
        low_price:row.product.low_price, volume:row.product.volume, daily_klines: Vec::new(),
    };
    let analysis=AnalysisResult {
        symbol:row.product.symbol.clone(), trend_score:row.trend_score, volume_score:row.volume_score,
        technical_score:row.technical_score, risk_score:row.risk_score,
        sentiment_score:row.sentiment_score, sentiment_value:row.sentiment_value,
        sentiment_classification:row.sentiment_classification.clone(), sentiment_update_time:row.sentiment_update_time.clone(),
        liquidity_score:row.liquidity_score,
        total_score:row.total_score, data_completeness:row.data_completeness, trend_label:row.trend_label.clone(), basis:row.basis.clone(),
    };
    build_report_prompt(&market,&analysis)
}

fn detail_stat(label: &'static str, value: String, theme: &gpui_component::Theme) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(0.))
        .gap_1()
        .p_2()
        .rounded(px(6.))
        .bg(theme.muted.opacity(0.08))
        .child(div().w_full().text_size(px(10.)).text_color(palette::muted(theme)).child(label))
        .child(div().w_full().text_size(px(11.)).font_semibold().child(value))
        .into_any_element()
}


fn detail_stat_colored(
    label: &'static str,
    value: String,
    color: gpui::Hsla,
    theme: &gpui_component::Theme,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(0.))
        .gap_1()
        .p_2()
        .rounded(px(6.))
        .bg(color.opacity(0.08))
        .border_1()
        .border_color(color.opacity(0.18))
        .child(div().w_full().text_size(px(10.)).text_color(palette::muted(theme)).child(label))
        .child(div().w_full().text_size(px(11.)).font_semibold().text_color(color).child(value))
        .into_any_element()
}

fn score_color(score: f64, theme: &gpui_component::Theme) -> gpui::Hsla {
    if score >= 65.0 {
        theme.success.opacity(0.95)
    } else if score <= 35.0 {
        theme.danger.opacity(0.95)
    } else {
        palette::text_strong(theme)
    }
}

fn change_color(value: Option<f64>, theme: &gpui_component::Theme) -> gpui::Hsla {
    match value {
        Some(value) if value > 0.0 => theme.success.opacity(0.95),
        Some(value) if value < 0.0 => theme.danger.opacity(0.95),
        _ => palette::muted(theme),
    }
}

fn trend_color(label: &str, theme: &gpui_component::Theme) -> gpui::Hsla {
    if label.contains("偏强") || label.contains("走强") {
        theme.success.opacity(0.95)
    } else if label.contains("偏弱") || label.contains("走弱") {
        theme.danger.opacity(0.95)
    } else {
        palette::text_strong(theme)
    }
}

fn format_ma120_location(value: Option<f64>) -> String {
    value.map(|value| format!("{value:+.2}%")).unwrap_or_default()
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
    else if abs >= 1_000.0 { format!("{:.2}K", value / 1_000.0) }    else { format!("{value:.2}") }
}

fn amplitude_percent(product: &MarketProduct) -> String {
    product.high_price.zip(product.low_price)
        .filter(|(_, low)| *low > 0.0)
        .map(|(high, low)| format!("{:.2}%", (high - low) / low * 100.0))
        .unwrap_or_else(|| "-".to_string())
}



/// CMC Keyless Public API：无需 API Key，接口缓存/更新频率为 15 分钟。
/// 使用 blocking reqwest：GPUI 的 background_spawn 不保证处于 Tokio Runtime 中。
fn fetch_cmc_sentiment() -> anyhow::Result<MarketSentiment> {
    static CACHE: OnceLock<Mutex<Option<(Instant, MarketSentiment)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let cached = cache.lock().ok().and_then(|guard| guard.clone());
    if let Some((cached_at, value)) = cached.as_ref() {
        if cached_at.elapsed() < Duration::from_secs(15 * 60) {
            return Ok(value.clone());
        }
    }

    let fetched = (|| {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;
        let response = client
            .get("https://pro-api.coinmarketcap.com/public-api/v3/fear-and-greed/latest")
            .send()?
            .error_for_status()?;
        let payload: serde_json::Value = response.json()?;
        let data = payload
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("CMC 响应缺少 data 字段"))?;
        let value = data
            .get("value")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| anyhow::anyhow!("CMC 响应缺少有效的 value"))?;
        if !(0.0..=100.0).contains(&value) {
            anyhow::bail!("CMC 恐惧贪婪指数超出 0～100 范围：{value}");
        }
        let raw_classification = data
            .get("value_classification")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown");
        let classification = match raw_classification {
            "Extreme Fear" => "极度恐惧",
            "Fear" => "恐惧",
            "Neutral" => "中性",
            "Greed" => "贪婪",
            "Extreme Greed" => "极度贪婪",
            "Unknown" | "unknown" => "未知",
            other => other,
        }
        .to_string();
        let update_time = data
            .get("update_time")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok::<_, anyhow::Error>(MarketSentiment {
            value,
            classification,
            update_time,
        })
    })();

    match fetched {
        Ok(sentiment) => {
            if let Ok(mut guard) = cache.lock() {
                *guard = Some((Instant::now(), sentiment.clone()));
            }
            Ok(sentiment)
        }
        Err(error) => {
            if let Some((_, mut stale)) = cached {
                stale.classification.push_str("（缓存数据）");
                app_log::warn!(
                    "[现货AI分析] CoinMarketCap 请求失败，沿用上次缓存指数={} 更新时间={} error={:#}",
                    stale.value,
                    stale.update_time,
                    error
                );
                Ok(stale)
            } else {
                Err(error)
            }
        }
    }
}
fn format_beijing_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|time| chrono::FixedOffset::east_opt(8 * 60 * 60).map(|offset| time.with_timezone(&offset)))
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| if value.is_empty() { "暂无数据".to_string() } else { value.to_string() })
}
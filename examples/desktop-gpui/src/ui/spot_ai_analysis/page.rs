use crate::ui::palette;
#[path = "table.rs"]
mod table;
use self::table::SpotAiAnalysisTableDelegate;
use binance_tools::binance::market::MarketProduct;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Table as DataTable, TableState},
    tooltip::Tooltip,
    v_flex,
};
use spot_ai_analysis::{AnalysisResult, DailyMaSnapshot, MarketSentiment, MarketSnapshot, analyze_market_with_sentiment, build_report_prompt};
use std::{cmp::Ordering, collections::HashMap, sync::{Mutex, OnceLock}, time::{Duration, Instant}};

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
    sentiment_value: f64,
    sentiment_classification: String,
    sentiment_update_time: String,
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
                    let sentiment = fetch_cmc_sentiment().await.ok();
                    Ok::<_, anyhow::Error>((build_analysis_rows(products, &cached_ma, sentiment.as_ref()), sentiment))
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((rows, sentiment)) => {
                        this.scanned_count = rows.len();
                        this.selected_symbol = rows.first().map(|row| row.product.symbol.clone());
                        this.rows = rows;
                        if let Some(s) = sentiment.as_ref() {
                            app_log::info!("[现货AI分析] CoinMarketCap 恐惧贪婪指数={} 分类={} 更新时间={}", s.value, s.classification, s.update_time);
                        } else {
                            app_log::warn!("[现货AI分析] CoinMarketCap 恐惧贪婪指数获取失败，当前使用 50 分中性占位");
                        }
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
            (
                "趋势结构",
                row.trend_score,
                "权重 25%。依据 24 小时涨跌幅计算：50 + 涨跌幅(%) × 5，结果限制在 0～100 分。涨幅为 0 时得 50 分；涨跌幅每变化 1 个百分点，分数变化 5 分。它反映短期价格方向，不代表趋势一定持续。",
            ),
            (
                "成交活跃度",
                row.volume_score,
                "权重 20%。取该币种 24 小时 USDT 成交额，在本次符合条件的 USDT 现货候选币种中计算百分位排名。排名越靠前得分越高；这是相对排名，不代表资金净流入。",
            ),
            (
                "日线均价偏离",
                row.technical_score,
                "权重 20%。依据本地缓存的近 30 日日线收盘均价偏离：55 + 偏离幅度(%) × 4，结果限制在 0～100 分。偏离为正表示价格高于均价，偏离为负表示低于均价。缺少完整均线缓存时暂按 50 分。",
            ),
            (
                "波动风险",
                row.risk_score,
                "权重 15%。先计算 24 小时振幅：(24h 最高价 - 24h 最低价) ÷ 24h 最低价 × 100%；再按 100 - 振幅(%) × 4 计分，并限制在 0～100 分。振幅越大，风险参考分越低。",
            ),
            (
                "市场情绪",
                row.sentiment_score,
                "权重 10%。使用 CoinMarketCap 全市场恐惧与贪婪指数原始值（0～100）作为情绪分。该指标反映整个加密市场，不代表当前交易对的独立情绪。若请求失败则使用 50 分中性占位。",
            ),
            (
                "流动性参考",
                row.liquidity_score,
                "权重 10%。由成交活跃度百分位排名 × 70% + 振幅参考分 × 30% 组成；振幅参考分为 100 - 振幅(%) × 2，并限制在 0～100 分。当前没有盘口深度、买卖价差数据，因此不能等同于真实盘口流动性。",
            ),
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
            .child(detail_stat("市场恐惧贪婪指数", format!("{:.0} · {}", row.sentiment_value, row.sentiment_classification), theme))
            .child(div().text_size(px(10.)).text_color(palette::muted(theme)).child(format!("CoinMarketCap · 北京时间 {}", format_beijing_time(&row.sentiment_update_time))))
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
            .children(dimensions.into_iter().map(|(label, score, explanation)| {
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(74.))
                            .text_size(px(11.))
                            .text_color(palette::muted(theme))
                            .child(format!("{label} ⓘ"))
                            .tooltip(move |_, cx| {
                                cx.new(|_| Tooltip::new(explanation))
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
        let visible_count = self.table.read(cx).delegate().visible_rows_count();
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
    sentiment: Option<&MarketSentiment>,
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
    let scores = analyze_market_with_sentiment(&inputs,&ma,sentiment).into_iter()
        .map(|s|(s.symbol.clone(),s)).collect::<HashMap<_,_>>();
    let mut rows=products.into_iter().filter_map(|product| {
        let s=scores.get(&product.symbol)?;
        Some(SpotAiAnalysisRow {
            product, trend_score:s.trend_score, volume_score:s.volume_score,
            technical_score:s.technical_score, risk_score:s.risk_score,
            sentiment_score:s.sentiment_score, sentiment_value:s.sentiment_value,
            sentiment_classification:s.sentiment_classification.clone(), sentiment_update_time:s.sentiment_update_time.clone(),
            liquidity_score:s.liquidity_score,
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
        sentiment_score:row.sentiment_score, sentiment_value:row.sentiment_value,
        sentiment_classification:row.sentiment_classification.clone(), sentiment_update_time:row.sentiment_update_time.clone(),
        liquidity_score:row.liquidity_score,
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



/// CMC Keyless Public API：无需 API Key，接口缓存/更新频率为 15 分钟。
async fn fetch_cmc_sentiment() -> anyhow::Result<MarketSentiment> {
    static CACHE: OnceLock<Mutex<Option<(Instant, MarketSentiment)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let cached = cache.lock().ok().and_then(|guard| guard.clone());
    if let Some((cached_at, value)) = cached.as_ref() {
        if cached_at.elapsed() < Duration::from_secs(15 * 60) {
            return Ok(value.clone());
        }
    }

    let fetched = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;
        let response = client
            .get("https://pro-api.coinmarketcap.com/public-api/v3/fear-and-greed/latest")
            .send()
            .await?
            .error_for_status()?;
        let payload: serde_json::Value = response.json().await?;
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
    }
    .await;

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

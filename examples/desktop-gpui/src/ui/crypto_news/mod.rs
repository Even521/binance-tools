use crate::ui::palette;
use binance_tools::{
    ai::{AiSettings, ChatMessage, send_news_rewrite_chat_blocking},
    db::{
        crypto_news::{
            NewsItemEntity, insert_news_items, update_news_rewrite,
        },
        open_default_connection,
    },
};
use crypto_rss::{Collector, FeedConfig};
use gpui::{prelude::FluentBuilder, *};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    table::{Column, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

#[derive(Debug, Clone)]
pub struct DbNewsItem {
    pub id: i64,
    pub source: String,
    pub title: String,
    pub summary: Option<String>,
    pub chinese_summary: Option<String>,
    pub published_at: Option<String>,
    pub collected_at: String,
    pub hot_rank: Option<usize>,
}

const NEWS_PAGE_SIZE: i64 = 15;
const NEWS_VISIBLE_ROWS: usize = 10;
const NEWS_PREFETCH_ROWS: usize = 2;
const NEWS_ROW_HEIGHT: f32 = 32.;

pub struct CryptoNewsPage {
    table: Entity<TableState<CryptoNewsTableDelegate>>,
    search_input: Entity<InputState>,
    total_count: usize,
    scroll_distance: Pixels,
    is_loading_more: bool,
    _search_subscription: Subscription,
    _table_subscription: Subscription,
    news_items: Vec<DbNewsItem>,
    status: Option<String>,
    error: Option<String>,
    is_syncing: bool,
    sync_guard: Arc<AtomicBool>,
    rewrite_open: bool,
    rewrite_title: String,
    rewrite_source: String,
    rewrite_result: Option<String>,
    rewrite_error: Option<String>,
    is_rewriting: bool,
    _rewrite_task: Task<()>,
}

struct CryptoNewsRewriteRequested {
    id: i64,
    title: String,
    content: String,
}

impl EventEmitter<CryptoNewsRewriteRequested> for TableState<CryptoNewsTableDelegate> {}

impl CryptoNewsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(CryptoNewsTableDelegate::new(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索新闻 ID")
                .default_value("")
        });
        let search_subscription =
            cx.subscribe_in(&search_input, window, Self::on_search_input_event);

        let table_subscription =
            cx.subscribe_in(&table, window, Self::on_rewrite_requested);
        let mut instance = Self {
            table,
            search_input,
            total_count: 0,
            scroll_distance: px(0.),
            is_loading_more: false,
            _search_subscription: search_subscription,
            _table_subscription: table_subscription,
            news_items: Vec::new(),
            status: None,
            error: None,
            is_syncing: false,
            sync_guard: Arc::new(AtomicBool::new(false)),
            rewrite_open: false,
            rewrite_title: String::new(),
            rewrite_source: String::new(),
            rewrite_result: None,
            rewrite_error: None,
            is_rewriting: false,
            _rewrite_task: Task::ready(()),
        };
        instance.reload_news(cx);
        instance
    }

    fn refresh_table_page(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_string();
        self.scroll_distance = px(0.);
        self.is_loading_more = false;
        match Self::fetch_from_db(0, &query) {
            Ok((items, total_count)) => {
                self.news_items = items;
                self.total_count = total_count;
                let visible_items = self.news_items.clone();
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_news(visible_items);
                    table.refresh(cx);
                });
            }
            Err(error) => self.error = Some(format!("加载新闻失败：{error:#}")),
        }
    }

    fn load_more_news(&mut self, cx: &mut Context<Self>) {
        if self.is_loading_more || self.news_items.len() >= self.total_count {
            return;
        }

        self.is_loading_more = true;
        let query = self.search_input.read(cx).value().trim().to_string();
        let offset = self.news_items.len();
        match Self::fetch_from_db(offset, &query) {
            Ok((mut items, total_count)) => {
                self.total_count = total_count;
                self.news_items.append(&mut items);
                let visible_items = self.news_items.clone();
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_news(visible_items);
                    table.refresh(cx);
                });
            }
            Err(error) => {
                self.error = Some(format!("加载更多新闻失败：{error:#}"));
            }
        }
        self.is_loading_more = false;
        cx.notify();
    }

    fn on_search_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.refresh_table_page(cx);
            cx.notify();
        }
    }

    fn reload_news(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.refresh_table_page(cx);
    }

    fn on_rewrite_requested(
        &mut self,
        _table: &Entity<TableState<CryptoNewsTableDelegate>>,
        event: &CryptoNewsRewriteRequested,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_rewriting {
            return;
        }

        self.rewrite_open = true;
        self.rewrite_title = event.title.clone();
        self.rewrite_source = event.content.clone();
        self.rewrite_result = None;
        self.rewrite_error = None;
        self.is_rewriting = true;
        cx.notify();

        let news_id = event.id;
        let title = event.title.clone();
        let source = event.content.clone();
        self._rewrite_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let settings = AiSettings::load_default()?;
                    let selection = settings.selected_model().clone();
                    const REWRITE_SYSTEM_PROMPT: &str = "你是一名专业的加密货币 X 博主，负责将币圈新闻改写成适合 X 发布的中文短帖。改写规则：1. 字数严格控制在 100 字以内，包含标点和空格。2. 开头直接突出最重要的新闻事件，避免铺垫。3. 保留核心人物、机构、项目、事件和关键数据。4. 使用自然、简洁、有传播力的中文，避免新闻通稿腔。5. 可以适当加入分析或点评，但必须有原文依据，不得编造观点。6. 不夸大事实，不制造恐慌，不使用标题党表达。7. 区分事实、传闻和预测，不得将传闻写成事实。8. 避免重复、空话和无意义的感叹。9. 不强制添加话题标签、表情符号或引导关注。10. 不要每条新闻使用相同句式，保持表达自然。只输出一条可以直接发布到 X 的中文文案，不输出标题、改写说明、字数统计或其他解释。超过 100 字时必须重新压缩并保证语句完整，不能截断；不得添加原文没有的事实、数字或信息。";
                    let messages = [
                        ChatMessage::system(REWRITE_SYSTEM_PROMPT),
                        ChatMessage::user(format!(
                            "请将下面这条新闻改写成一条可直接发布到 X 的中文短帖，严格遵守系统规则。\n\n新闻标题：{title}\n新闻内容：{source}"
                        )),
                    ];
                    let response = send_news_rewrite_chat_blocking(
                        &settings,
                        selection.clone(),
                        &messages,
                        Duration::from_secs(180),
                    )?;
                    let mut rewritten = response
                        .content
                        .trim()
                        .trim_matches('"')
                        .trim_matches('“')
                        .trim_matches('”')
                        .trim()
                        .to_string();

                    // 超过 100 字时让模型重新压缩，避免直接截断导致句子残缺。
                    if rewritten.chars().count() > 100 {
                        let retry_messages = [
                            ChatMessage::system(REWRITE_SYSTEM_PROMPT),
                            ChatMessage::user(format!(
                                "请将下面这条已经改写的 X 文案进一步压缩到 100 字以内（包含标点和空格）。保留核心事实和关键数据，保证语句完整。只输出最终文案，不要解释。\n\n原始新闻标题：{title}\n原始新闻内容：{source}\n\n当前文案：{rewritten}"
                            )),
                        ];
                        let retry = send_news_rewrite_chat_blocking(
                            &settings,
                            selection,
                            &retry_messages,
                            Duration::from_secs(180),
                        )?;
                        rewritten = retry.content
                            .trim()
                            .trim_matches('"')
                            .trim_matches('“')
                            .trim_matches('”')
                            .trim()
                            .to_string();
                    }

                    anyhow::ensure!(!rewritten.is_empty(), "AI 返回的改写内容为空");
                    anyhow::ensure!(
                        rewritten.chars().count() <= 100,
                        "AI 改写内容仍超过 100 字，未写入数据库，请重试"
                    );

                    let connection = open_default_connection()?;
                    update_news_rewrite(&connection, news_id, &rewritten)?;
                    Ok::<String, anyhow::Error>(rewritten)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.is_rewriting = false;
                match result {
                    Ok(rewritten) => {
                        this.rewrite_result = Some(rewritten.clone());
                        this.rewrite_error = None;
                        if let Some(item) = this.news_items.iter_mut().find(|item| item.id == news_id) {
                            item.chinese_summary = Some(rewritten);
                        }
                        let items = this.news_items.clone();
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_news(items);
                            table.refresh(cx);
                        });
                    }
                    Err(error) => {
                        this.rewrite_error = Some(format!("AI 改写失败：{error:#}"));
                    }
                }
                cx.notify();
            });
        });
    }

    fn fetch_from_db(offset: usize, query: &str) -> anyhow::Result<(Vec<DbNewsItem>, usize)> {
        let conn = open_default_connection()?;
        // 在 SQLite 中先过滤中文标题和 ID，再执行 COUNT / LIMIT / OFFSET，
        // 避免把整张新闻表加载到 UI 后再分页。
        const FILTER: &str = "title GLOB '*[㐀-鿿豈-﫿]*' AND (?1 = '' OR CAST(id AS TEXT) = ?1)";
        let total_count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM crypto_news WHERE {FILTER}"),
            [query],
            |row| row.get(0),
        )?;
        let offset = offset.min(i64::MAX as usize) as i64;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, source, title, summary, chinese_summary, published_at, collected_at
             FROM crypto_news WHERE {FILTER}
             ORDER BY datetime(COALESCE(published_at, collected_at)) DESC, id DESC
             LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = stmt.query_map((query, NEWS_PAGE_SIZE, offset), |row| {
            Ok(DbNewsItem {
                id: row.get(0)?,
                source: row.get(1)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                chinese_summary: row.get(4)?,
                published_at: row.get(5)?,
                collected_at: row.get(6)?,
                hot_rank: None,
            })
        })?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok((items, total_count.max(0) as usize))
    }

    fn on_sync_click(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // UI 状态用于禁用按钮；原子锁用于兜底拦截连续/重复事件，
        // 确保同一页面生命周期内最多只有一个 RSS 同步任务。
        if self.is_syncing
            || self
                .sync_guard
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        self.is_syncing = true;
        self.status = Some("正在同步 RSS 新闻（请确保网络畅通）...".to_string());
        self.error = None;
        cx.notify();

        let sync_guard = self.sync_guard.clone();
        cx.spawn(async move |this, cx| {
            // RSS 网络请求和 SQLite 写入都放在独立线程执行。
            // 通过 oneshot 异步等待结果，不能在 GPUI 任务里调用 JoinHandle::join()，
            // 否则会阻塞界面线程，导致点击同步时窗口卡顿。
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();

            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<usize> {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;

                    rt.block_on(async {
                        let feeds = vec![
                            FeedConfig {
                                name: "Odaily 快讯".to_string(),
                                url: "https://rss.odaily.news/rss/newsflash".to_string(),
                            },
                            FeedConfig {
                                name: "BlockBeats 快讯".to_string(),
                                url: "https://api.theblockbeats.news/v2/rss/newsflash".to_string(),
                            },
                            FeedConfig {
                                name: "ChainCatcher 快讯".to_string(),
                                url: "https://www.chaincatcher.com/rss/clist".to_string(),
                            },
                            FeedConfig {
                                name: "PANews".to_string(),
                                url: "https://rss.panewslab.com/zh/tvsq/rss".to_string(),
                            },
                        ];
                        let collector = Collector::new()?;
                        let (items, reports) = collector.collect_all(&feeds).await;
                        for rep in &reports {
                            if let Some(e) = &rep.error {
                                app_log::news_warn!("加密新闻 RSS 源同步失败：{}：{}", rep.source, e);
                            }
                        }

                        // 只保留快讯：摘要正文超过 500 个字符的内容直接丢弃，
                        // 避免 RSS 混入长篇专栏/文章；空摘要的短快讯仍允许入库。
                        const MAX_SUMMARY_CHARS: usize = 500;
                        let entities: Vec<NewsItemEntity> = items
                            .into_iter()
                            .filter(|item| {
                                item.summary
                                    .as_deref()
                                    .map(|summary| summary.chars().count() <= MAX_SUMMARY_CHARS)
                                    .unwrap_or(true)
                            })
                            .map(|i| NewsItemEntity {
                                source: i.source,
                                title: i.title,
                                url: i.url,
                                summary: i.summary,
                                published_at: i.published_at.map(|dt| dt.to_rfc3339()),
                                collected_at: i.collected_at.to_rfc3339(),
                            })
                            .collect();

                        let conn = open_default_connection()?;
                        insert_news_items(&conn, &entities)
                    })
                })();

                let _ = result_tx.send(result);
            });

            let result = result_rx.await;

            _ = this.update(cx, |this, cx| {
                this.is_syncing = false;
                sync_guard.store(false, Ordering::Release);
                match result {
                    Ok(Ok(count)) => {
                        this.reload_news(cx);
                        let hot_count = this.rank_hot_news(cx);
                        this.status = Some(format!(
                            "同步完成！新增 {count} 条，已自动识别并置顶前 {hot_count} 条热点新闻。"
                        ));
                    }
                    Ok(Err(err)) => {
                        app_log::news_error!("加密新闻 RSS 同步失败：{err:?}");
                        this.error = Some(format!("同步失败: {err}"));
                        this.status = None;
                    }
                    Err(_) => {
                        app_log::news_error!("加密新闻 RSS 同步线程异常退出");
                        this.error = Some("同步线程异常退出".to_string());
                        this.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rank_hot_news(&mut self, cx: &mut Context<Self>) -> usize {
        let mut scored: Vec<(i64, DbNewsItem)> = self
            .news_items
            .drain(..)
            .map(|mut item| {
                let score = Self::hot_score(&item);
                item.hot_rank = None;
                (score, item)
            })
            .collect();

        scored.sort_by(|(score_a, item_a), (score_b, item_b)| {
            score_b.cmp(score_a).then_with(|| {
                item_b.published_at.as_deref().unwrap_or(&item_b.collected_at)
                    .cmp(item_a.published_at.as_deref().unwrap_or(&item_a.collected_at))
            })
        });

        let hot_count = scored.len().min(5);
        for (index, (_, item)) in scored.iter_mut().take(hot_count).enumerate() {
            item.hot_rank = Some(index + 1);
        }
        scored[hot_count..].sort_by(|(_, a), (_, b)| {
            b.published_at.as_deref().unwrap_or(&b.collected_at)
                .cmp(a.published_at.as_deref().unwrap_or(&a.collected_at))
        });

        self.news_items = scored.into_iter().map(|(_, item)| item).collect();
        self.refresh_table_page(cx);
        hot_count
    }

    fn hot_score(item: &DbNewsItem) -> i64 {
        let text = format!(
            "{} {}",
            item.title,
            item.summary.as_deref().unwrap_or_default()
        )
        .to_lowercase();

        let weighted_terms = [
            ("比特币", 5), ("btc", 5), ("以太坊", 5), ("ethereum", 5), ("eth", 4),
            ("美联储", 5), ("降息", 5), ("加息", 4), ("etf", 5), ("sec", 4),
            ("监管", 4), ("黑客", 5), ("被盗", 5), ("漏洞", 4), ("清算", 5),
            ("暴涨", 4), ("暴跌", 4), ("大涨", 3), ("大跌", 3), ("突破", 3),
            ("历史新高", 5), ("创纪录", 4), ("资金流入", 4), ("资金流出", 4),
            ("交易所", 3), ("币安", 4), ("上币", 3), ("下架", 3), ("稳定币", 3),
            ("空投", 2), ("主网", 2), ("融资", 3), ("收购", 3), ("诉讼", 4),
            ("破产", 5), ("暂停提现", 5), ("清退", 3), ("政策", 3),
        ];

        let mut score = weighted_terms
            .iter()
            .filter(|(term, _)| text.contains(term))
            .map(|(_, weight)| *weight)
            .sum::<i64>();

        // 标题中的热点信号比摘要中的普通提及更重要。
        let title = item.title.to_lowercase();
        for (term, weight) in [
            ("突发", 5), ("重磅", 4), ("紧急", 5), ("首次", 2), ("正式", 2),
            ("宣布", 2), ("巨鲸", 4), ("大额", 3), ("清算", 3),
        ] {
            if title.contains(term) {
                score += weight;
            }
        }

        // 新鲜度加权：24 小时内优先，避免旧新闻因关键词多而长期置顶。
        let timestamp = item.published_at.as_deref().unwrap_or(&item.collected_at);
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) {
            let age_hours = (app_log::beijing_now() - parsed.with_timezone(&app_log::beijing_timezone()))
                .num_hours()
                .max(0);
            score += match age_hours {
                0..=2 => 8,
                3..=6 => 6,
                7..=12 => 4,
                13..=24 => 2,
                25..=48 => 1,
                _ => 0,
            };
        }

        score
    }


}

#[derive(Clone)]
struct CryptoNewsTableDelegate {
    columns: Vec<Column>,
    news: Vec<DbNewsItem>,
}

impl CryptoNewsTableDelegate {
    fn new() -> Self {
        Self {
            columns: vec![
                Column::new("id", "ID").width(px(76.)).sortable(),
                Column::new("source", "来源").width(px(120.)).sortable(),
                Column::new("title", "标题").width(px(360.)).sortable(),
                Column::new("content", "内容").width(px(520.)),
                Column::new("published_at", "发布时间")
                    .width(px(180.))
                    .sortable(),
            ],
            news: Vec::new(),
        }
    }

    fn set_news(&mut self, news: Vec<DbNewsItem>) {
        self.news = news;
    }

    fn text_cell(text: impl Into<String>, color: Hsla) -> AnyElement {
        div()
            .text_size(px(12.))
            .text_color(color)
            .child(text.into())
            .into_any_element()
    }

    // RSS 时间统一按北京时间（UTC+8）显示；无法解析的原始值保持原样。
    fn format_beijing_time(value: &str) -> String {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|time| {
                time.with_timezone(
                    &app_log::beijing_timezone(),
                )
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
            })
            .unwrap_or_else(|_| value.to_string())
    }
}

impl TableDelegate for CryptoNewsTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.news.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let Some(item) = self.news.get(row_ix) else {
            return Self::text_cell("", palette::text(theme));
        };
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "id" => Self::text_cell(item.id.to_string(), palette::muted(theme)),
            "source" => Self::text_cell(item.source.clone(), palette::text(theme)),
            "title" => {
                let title = match item.hot_rank {
                    Some(rank) => format!("🔥 TOP {rank}  {}", item.title),
                    None => item.title.clone(),
                };
                Self::text_cell(title, palette::text(theme))
            },
            "content" => {
                let content = item
                    .chinese_summary
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .or(item.summary.as_deref())
                    .unwrap_or("暂无内容")
                    .to_string();
                let copy_content = content.clone();
                let rewrite_news_id = item.id;
                let rewrite_news_title = item.title.clone();
                let rewrite_content = content.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new(("crypto-news-ai-rewrite", row_ix))
                            .ghost()
                            .xsmall()
                            .w(px(28.))
                            .h(px(18.))
                            .tooltip("AI 改写为 X 文案（100 字以内）")
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_semibold()
                                    .text_color(palette::muted(theme))
                                    .child("AI"),
                            )
                            .on_click(cx.listener(move |_state, _event, _window, cx| {
                                cx.emit(CryptoNewsRewriteRequested {
                                    id: rewrite_news_id,
                                    title: rewrite_news_title.clone(),
                                    content: rewrite_content.clone(),
                                });
                            })),
                    )
                    // 复制按钮紧跟在 AI 改写按钮后面。
                    .child(
                        Button::new(("crypto-news-copy", row_ix))
                            .ghost()
                            .xsmall()
                            .w(px(22.))
                            .h(px(18.))
                            .tooltip("复制新闻内容")
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(theme))
                                    .child("⧉"),
                            )
                            .on_click(cx.listener(move |_state, _event, _window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    copy_content.clone(),
                                ));
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(palette::text(theme))
                            .child(content),
                    )
                    .into_any_element()
            },
            "published_at" => {
                let timestamp = item.published_at.as_deref().unwrap_or(&item.collected_at);
                Self::text_cell(
                    Self::format_beijing_time(timestamp),
                    palette::text(theme),
                )
            },
            _ => Self::text_cell("", palette::text(theme)),
        }
    }
}

impl Render for CryptoNewsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let surface = palette::surface_strong(theme);
        let border = palette::border(theme);
        let muted = palette::muted(theme);
        let text_strong = palette::text_strong(theme);
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
                                        div()
                                            .text_size(px(16.))
                                            .font_semibold()
                                            .text_color(text_strong)
                                            .child("加密货币快讯 (Crypto RSS)"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(muted)
                                            .child(format!(
                                                "仅收集中文加密货币 RSS 快讯，自动过滤英文标题并去重入库。共 {} 条资讯。",
                                                self.total_count
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        div().w(px(260.)).max_w(px(320.)).child(
                                            Input::new(&self.search_input).small().cleanable(true),
                                        ),
                                    )
                                    .child(
                                        Button::new("sync_news_btn")
                                            .outline()
                                            .xsmall()
                                            .label(if self.is_syncing { "正在同步..." } else { "立即同步 RSS" })
                                            .disabled(self.is_syncing)
                                            .on_click(cx.listener(Self::on_sync_click)),
                                    ),
                            ),
                    )
                    .when_some(self.status.clone(), |parent, status| {
                        parent.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.success)
                                .child(status),
                        )
                    })
                    .when_some(self.error.clone(), |parent, err| {
                        parent.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.danger)
                                .child(err),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .h(px(220.))
                            .min_h(px(220.))
                            .min_w_0()
                            .child(
                                div()
                                    .flex_1()
                                    .size_full()
                                    .overflow_hidden()
                                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                                        let delta = event.delta.pixel_delta(window.line_height());
                                        if delta.y < px(0.) {
                                            this.scroll_distance -= delta.y;
                                            let rows_before_prefetch = this
                                                .news_items
                                                .len()
                                                .saturating_sub(NEWS_VISIBLE_ROWS + NEWS_PREFETCH_ROWS);
                                            let load_threshold =
                                                px(rows_before_prefetch as f32 * NEWS_ROW_HEIGHT);
                                            if this.scroll_distance >= load_threshold {
                                                this.load_more_news(cx);
                                            }
                                        }
                                    }))
                                    .child(
                                        DataTable::new(&self.table)
                                            .stripe(true)
                                            .bordered(true)
                                            .scrollbar_visible(true, true),
                                    ),
                            ),
                    )
                    .when(self.rewrite_open, |row| {
                        row.child(
                            v_flex()
                                .w(px(360.))
                                .min_w(px(320.))
                                .h_full()
                                .gap_3()
                                .p_3()
                                .rounded(px(8.))
                                .bg(surface)
                                .border_1()
                                .border_color(border)
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .font_semibold()
                                                .text_color(text_strong)
                                                .child("AI 改写"),
                                        )
                                        .child(
                                            Button::new("close_crypto_news_rewrite")
                                                .ghost()
                                                .xsmall()
                                                .label("关闭")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.rewrite_open = false;
                                                    cx.notify();
                                                })),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(muted)
                                                .child("原始新闻"),
                                        )
                                        .child(
                                            div()
                                                .p_2()
                                                .rounded(px(6.))
                                                .bg(theme.background)
                                                .text_size(px(12.))
                                                .text_color(palette::text(theme))
                                                .child(self.rewrite_source.clone()),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(muted)
                                                .child("当前新闻"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .font_semibold()
                                                .text_color(text_strong)
                                                .child(self.rewrite_title.clone()),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_h_0()
                                        .gap_1()
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(muted)
                                                        .child("X 改写结果"),
                                                )
                                                .child(
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            div()
                                                                .text_size(px(10.))
                                                                .text_color(muted)
                                                                .child("最多 100 字"),
                                                        )
                                                        .child(
                                                            Button::new("copy_news_rewrite_result")
                                                                .ghost()
                                                                .xsmall()
                                                                .w(px(24.))
                                                                .h(px(22.))
                                                                .tooltip("复制 AI 改写结果")
                                                                .disabled(
                                                                    self.is_rewriting
                                                                        || self.rewrite_result
                                                                            .as_ref()
                                                                            .map(|value| value.trim().is_empty())
                                                                            .unwrap_or(true),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(px(13.))
                                                                        .text_color(muted)
                                                                        .child("⧉"),
                                                                )
                                                                .on_click(cx.listener(|this, _, _, cx| {
                                                                    if let Some(result) = this
                                                                        .rewrite_result
                                                                        .as_ref()
                                                                        .filter(|value| !value.trim().is_empty())
                                                                    {
                                                                        cx.write_to_clipboard(
                                                                            ClipboardItem::new_string(result.clone()),
                                                                        );
                                                                    }
                                                                })),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_h(px(140.))
                                                .p_2()
                                                .rounded(px(6.))
                                                .bg(theme.background)
                                                .text_size(px(13.))
                                                .text_color(palette::text(theme))
                                                .child(if self.is_rewriting {
                                                    "正在等待 AI 返回改写结果...".to_string()
                                                } else {
                                                    self.rewrite_result.clone().unwrap_or_else(|| {
                                                        "改写结果将在这里显示".to_string()
                                                    })
                                                }),
                                        )
                                        .when_some(self.rewrite_error.clone(), |panel, error| {
                                            panel.child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(theme.danger)
                                                    .child(error),
                                            )
                                        }),
                                ),
                        )
                    }),
            )
    }
}

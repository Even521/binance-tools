use crate::ui::palette;
use binance_tools::{
    ai::{AiSettings, ChatMessage, send_news_rewrite_chat_blocking},
    db::{
        entertainment_news::{
            NewsItemEntity, delete_all_news, insert_news_items, update_news_rewrite,
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

pub struct EntertainmentNewsPage {
    table: Entity<TableState<EntertainmentNewsTableDelegate>>,
    search_input: Entity<InputState>,
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
    clear_confirm: bool,
}

struct EntertainmentNewsRewriteRequested {
    id: i64,
    title: String,
    content: String,
}

impl EventEmitter<EntertainmentNewsRewriteRequested> for TableState<EntertainmentNewsTableDelegate> {}

impl EntertainmentNewsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(EntertainmentNewsTableDelegate::new(), window, cx)
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
            clear_confirm: false,
        };
        instance.reload_news(cx);
        instance
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
        match Self::fetch_from_db() {
            Ok(items) => {
                self.news_items = items;
                self.error = None;
                self.refresh_table_page(cx);
            }
            Err(err) => {
                self.error = Some(format!("加载新闻失败: {err}"));
            }
        }
    }

    fn on_rewrite_requested(
        &mut self,
        _table: &Entity<TableState<EntertainmentNewsTableDelegate>>,
        event: &EntertainmentNewsRewriteRequested,
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
                    const REWRITE_SYSTEM_PROMPT: &str = "你是一名熟悉中文 X 平台表达习惯的内容编辑。请把输入内容改写成自然、简洁、有网感、像真实用户会发在 X 上的中文短帖。要求：1. 保留原文核心事实、人物、时间和关键细节，不改变原意，不补充原文没有的信息。2. 使用 X 上常见的自然口语表达和轻松节奏，避免新闻通稿腔、机器翻译腔和生硬营销腔。3. 直接呈现事件或重点，不要在末尾添加结论、总结、感想、点评、升华或价值判断。4. 不要添加任何表情符号或 emoji。5. 不要擅自添加标题、话题标签、引导关注、互动提问或无关铺垫。6. 不夸大、不造谣；原文属于传闻时，保留传闻属性，不得写成已证实事实。7. 尽量保留原文有价值的信息，只优化措辞、节奏和可读性，不要为了吸引眼球歪曲内容。8. 严格控制在 100 字以内（包含标点和空格），保证语句自然完整。只输出改写后的正文，不输出说明。";
                    let messages = [
                        ChatMessage::system(REWRITE_SYSTEM_PROMPT),
                        ChatMessage::user(format!(
                            "请只优化下面内容的表达方式，使它更符合中文 X 用户常见的发帖风格；不要添加结论、表情符号或原文没有的信息。严格遵守系统规则。\n\n内容标题：{title}\n内容正文：{source}"
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
                        this.refresh_table_page(cx);
                    }
                    Err(error) => {
                        this.rewrite_error = Some(format!("AI 改写失败：{error:#}"));
                    }
                }
                cx.notify();
            });
        });
    }

    // 与 Alpha Token 列表保持一致：完整数据交给 DataTable，由表格自身滚动浏览。
    fn refresh_table_page(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_string();
        let visible_items = if query.is_empty() {
            self.news_items.clone()
        } else {
            self.news_items.iter().filter(|item| item.id.to_string() == query).cloned().collect()
        };
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_news(visible_items);
            table.refresh(cx);
        });
    }

    fn on_clear_all_click(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_syncing {
            return;
        }
        if !self.clear_confirm {
            self.clear_confirm = true;
            self.status = Some("再次点击‘确认清空’将永久删除全部吃瓜娱乐数据。".to_string());
            cx.notify();
            return;
        }
        match (|| -> anyhow::Result<usize> {
            let conn = open_default_connection()?;
            delete_all_news(&conn)
        })() {
            Ok(deleted) => {
                self.news_items.clear();
                self.clear_confirm = false;
                self.rewrite_open = false;
                self.status = Some(format!("已清空娱乐新闻数据，共删除 {deleted} 条。"));
                self.error = None;
                self.refresh_table_page(cx);
            }
            Err(err) => {
                self.clear_confirm = false;
                self.error = Some(format!("清空数据失败：{err}"));
            }
        }
        cx.notify();
    }

    fn fetch_from_db() -> anyhow::Result<Vec<DbNewsItem>> {
        let conn = open_default_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, title, summary, chinese_summary, published_at, collected_at FROM entertainment_news ORDER BY datetime(COALESCE(published_at, collected_at)) DESC LIMIT 1000"
        )?;
        let rows = stmt.query_map([], |row| {
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
        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
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
                let result = (|| -> anyhow::Result<(usize, usize, usize, String)> {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;

                    rt.block_on(async {
                        let feeds = vec![
                            FeedConfig {
                                name: "新浪娱乐要闻".to_string(),
                                url: "https://rss.sina.com.cn/ent/hot_roll.xml".to_string(),
                            },
                            FeedConfig {
                                name: "新浪明星全接触".to_string(),
                                url: "https://rss.sina.com.cn/ent/star/focus7.xml".to_string(),
                            },
                            FeedConfig {
                                name: "热摸爽综合热点".to_string(),
                                url: "https://remoshuang.com/feed.xml".to_string(),
                            },
                            FeedConfig {
                                name: "新浪内地明星".to_string(),
                                url: "https://rss.sina.com.cn/ent/inner.xml".to_string(),
                            },
                            FeedConfig {
                                name: "中国新闻网文娱".to_string(),
                                url: "https://www.chinanews.com.cn/rss/culture.xml".to_string(),
                            },
                        ];
                        let collector = Collector::new()?;
                        let (items, reports) = collector.collect_all(&feeds).await;
                        let fetched_total = reports.iter().map(|report| report.fetched).sum::<usize>();
                        let report_text = reports
                            .iter()
                            .map(|report| match &report.error {
                                Some(error) => format!("{}：失败（{}）", report.source, error),
                                None => format!("{}：获取 {} 条", report.source, report.fetched),
                            })
                            .collect::<Vec<_>>()
                            .join("；");
                        for report in &reports {
                            if let Some(error) = &report.error {
                                app_log::news_warn!("娱乐新闻 RSS 源同步失败：{}：{}", report.source, error);
                            }
                        }

                        // 统计过滤前后数量，避免所有内容被长度条件过滤后页面仍误报同步成功。
                        const MAX_SUMMARY_CHARS: usize = 300;
                        let entities: Vec<NewsItemEntity> = items
                            .into_iter()
                            .filter(|item| {
                                item.title.chars().count() <= MAX_SUMMARY_CHARS
                                    && item
                                        .summary
                                        .as_deref()
                                        .map(|summary| summary.chars().count() <= MAX_SUMMARY_CHARS)
                                        .unwrap_or(true)
                            })
                            .map(|item| NewsItemEntity {
                                source: item.source,
                                title: item.title,
                                url: item.url,
                                summary: item.summary,
                                published_at: item.published_at.map(|dt| dt.to_rfc3339()),
                                collected_at: item.collected_at.to_rfc3339(),
                            })
                            .collect();
                        let accepted_count = entities.len();

                        let conn = open_default_connection()?;
                        let inserted_count = insert_news_items(&conn, &entities)?;
                        Ok((inserted_count, fetched_total, accepted_count, report_text))
                    })
                })();

                let _ = result_tx.send(result);
            });

            let result = result_rx.await;

            _ = this.update(cx, |this, cx| {
                this.is_syncing = false;
                sync_guard.store(false, Ordering::Release);
                match result {
                    Ok(Ok((count, fetched, accepted, report))) => {
                        this.reload_news(cx);
                        let hot_count = this.rank_hot_news(cx);
                        this.status = Some(format!(
                            "同步结束：RSS 获取 {fetched} 条，长度筛选后 {accepted} 条，数据库新增 {count} 条；当前列表 {} 条。热点置顶 {hot_count} 条。订阅源：{}",
                            this.news_items.len(),
                            report
                        ));
                    }
                    Ok(Err(err)) => {
                        app_log::news_error!("娱乐新闻 RSS 同步失败：{err:?}");
                        this.error = Some(format!("同步失败: {err}"));
                        this.status = None;
                    }
                    Err(_) => {
                        app_log::news_error!("娱乐新闻 RSS 同步线程异常退出");
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
            ("官宣", 5), ("恋情", 5), ("分手", 5), ("结婚", 4), ("离婚", 5),
            ("出轨", 5), ("回应", 4), ("声明", 5), ("道歉", 4), ("塌房", 5),
            ("争议", 4), ("爆料", 4), ("辟谣", 5), ("工作室", 3), ("经纪人", 2),
            ("演员", 2), ("明星", 2), ("艺人", 2), ("综艺", 2), ("电视剧", 2),
            ("电影", 2), ("票房", 3), ("定档", 3), ("杀青", 2), ("开机", 2),
            ("红毯", 3), ("演唱会", 2), ("粉丝", 2), ("网红", 2), ("直播", 2),
        ];

        let mut score = weighted_terms
            .iter()
            .filter(|(term, _)| text.contains(term))
            .map(|(_, weight)| *weight)
            .sum::<i64>();

        // 标题中的热点信号比摘要中的普通提及更重要。
        let title = item.title.to_lowercase();
        for (term, weight) in [
            ("突发", 5), ("重磅", 4), ("紧急", 5), ("官宣", 5), ("回应", 3), ("吃瓜", 5), ("瓜", 2), ("爆料", 5), ("热搜", 4), ("离谱", 3), ("反转", 4), ("笑死", 3), ("搞笑", 3), ("段子", 3),
            ("声明", 4), ("首次", 2), ("正式", 2), ("曝光", 4), ("热搜", 4),
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
struct EntertainmentNewsTableDelegate {
    columns: Vec<Column>,
    news: Vec<DbNewsItem>,
}

impl EntertainmentNewsTableDelegate {
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

impl TableDelegate for EntertainmentNewsTableDelegate {
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
                        Button::new(("entertainment-news-ai-rewrite", row_ix))
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
                                cx.emit(EntertainmentNewsRewriteRequested {
                                    id: rewrite_news_id,
                                    title: rewrite_news_title.clone(),
                                    content: rewrite_content.clone(),
                                });
                            })),
                    )
                    // 复制按钮紧跟在 AI 改写按钮后面。
                    .child(
                        Button::new(("entertainment-news-copy", row_ix))
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

impl Render for EntertainmentNewsPage {
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
                                            .child("吃瓜娱乐 · 搞笑段子 (Entertainment RSS)"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(muted)
                                            .child(format!(
                                                "主打明星八卦、吃瓜爆料、网红动态和搞笑段子；过滤超过 300 字的内容并去重。当前显示 {} 条。",
                                                self.news_items.len()
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
                                    )
                                    .child(
                                        Button::new("clear_entertainment_news_btn")
                                            .outline()
                                            .xsmall()
                                            .label(if self.clear_confirm { "确认清空" } else { "清空数据" })
                                            .disabled(self.is_syncing || self.news_items.is_empty())
                                            .on_click(cx.listener(Self::on_clear_all_click)),
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
                            .h_full()
                            .min_h(px(420.))
                            .min_w_0()
                            .child(
                                div().flex_1().size_full().overflow_hidden().child(
                                    DataTable::new(&self.table)
                                        .stripe(true)
                                        .bordered(true)
                                        .scrollbar_visible(true, true),
                                ),
                            )

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
                                            Button::new("close_entertainment_news_rewrite")
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
                                                .justify_between()
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(muted)
                                                        .child("X 改写结果"),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(10.))
                                                        .text_color(muted)
                                                        .child("最多 100 字"),
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

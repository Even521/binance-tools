use crate::ui::palette;
use binance_tools::db::{
    crypto_news::NewsItemEntity, crypto_news::insert_news_items,
    open_default_connection,
};
use crypto_rss::{Collector, FeedConfig};
use gpui::{prelude::FluentBuilder, *};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    table::{Column, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

#[derive(Debug, Clone)]
pub struct DbNewsItem {
    pub id: i64,
    pub source: String,
    pub title: String,
    pub chinese_title: Option<String>,
    pub url: Option<String>,
    pub summary: Option<String>,
    pub chinese_summary: Option<String>,
    pub published_at: Option<String>,
    pub collected_at: String,
    pub hot_rank: Option<usize>,
}

pub struct CryptoNewsPage {
    table: Entity<TableState<CryptoNewsTableDelegate>>,
    news_items: Vec<DbNewsItem>,
    status: Option<String>,
    error: Option<String>,
    is_syncing: bool,
    sync_guard: Arc<AtomicBool>,
}

impl CryptoNewsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(CryptoNewsTableDelegate::new(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });

        let mut instance = Self {
            table,
            news_items: Vec::new(),
            status: None,
            error: None,
            is_syncing: false,
            sync_guard: Arc::new(AtomicBool::new(false)),
        };
        instance.reload_news(cx);
        instance
    }

    fn reload_news(&mut self, cx: &mut Context<Self>) {
        match Self::fetch_from_db() {
            Ok(items) => {
                self.news_items = items;
                self.error = None;
                let items_clone = self.news_items.clone();
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_news(items_clone);
                    table.refresh(cx);
                });
            }
            Err(err) => {
                self.error = Some(format!("加载新闻失败: {err}"));
            }
        }
    }

    fn fetch_from_db() -> anyhow::Result<Vec<DbNewsItem>> {
        let conn = open_default_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, title, chinese_title, url, summary, chinese_summary, published_at, collected_at FROM crypto_news ORDER BY datetime(COALESCE(published_at, collected_at)) DESC LIMIT 1000"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DbNewsItem {
                id: row.get(0)?,
                source: row.get(1)?,
                title: row.get(2)?,
                chinese_title: row.get(3)?,
                url: row.get(4)?,
                summary: row.get(5)?,
                chinese_summary: row.get(6)?,
                published_at: row.get(7)?,
                collected_at: row.get(8)?,
                hot_rank: None,
            })
        })?;
        let mut list = Vec::new();
        for row in rows {
            let item = row?;
            if item.title.chars().any(|ch| matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)) {
                list.push(item);
            }
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
                                eprintln!("[CryptoNews] Feed {} error: {}", rep.source, e);
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
                        eprintln!("[CryptoNews] Sync error: {err:?}");
                        this.error = Some(format!("同步失败: {err}"));
                        this.status = None;
                    }
                    Err(_) => {
                        eprintln!("[CryptoNews] Sync thread stopped unexpectedly");
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
        let items_clone = self.news_items.clone();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_news(items_clone);
            table.refresh(cx);
        });
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
            let age_hours = (chrono::Utc::now() - parsed.with_timezone(&chrono::Utc))
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
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(palette::text(theme))
                            .child(content),
                    )
                    .child(
                        Button::new(format!("copy_news_content_{row_ix}"))
                            .outline()
                            .xsmall()
                            .label("复制")
                            .on_click(cx.listener(move |_state, _event, _window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    copy_content.clone(),
                                ));
                            })),
                    )
            },
            "published_at" => Self::text_cell(
                item.published_at.clone().unwrap_or_else(|| item.collected_at.clone()),
                palette::text(theme),
            ),
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
                                                "仅收集中文加密货币 RSS 快讯，自动过滤英文标题并去重入库。当前显示 {} 条资讯。",
                                                self.news_items.len()
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
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
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(8.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(border)
                    .overflow_hidden()
                    .child(DataTable::new(&self.table)),
            )
    }
}

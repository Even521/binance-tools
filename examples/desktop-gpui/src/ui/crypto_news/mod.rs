use crate::ui::palette;
use binance_tools::ai::{ChatMessage, send_chat_blocking};
use binance_tools::db::{
    crypto_news::NewsItemEntity, crypto_news::insert_news_items,
    crypto_news::update_news_translation, open_default_connection,
};
use crypto_rss::{Collector, FeedConfig};
use gpui::{prelude::FluentBuilder, *};
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
}

pub struct CryptoNewsPage {
    table: Entity<TableState<CryptoNewsTableDelegate>>,
    selected_news_id: Option<i64>,
    news_items: Vec<DbNewsItem>,
    status: Option<String>,
    error: Option<String>,
    is_syncing: bool,
    is_translating: bool,
}

impl CryptoNewsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(CryptoNewsTableDelegate::new(page), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });

        let mut instance = Self {
            table,
            selected_news_id: None,
            news_items: Vec::new(),
            status: None,
            error: None,
            is_syncing: false,
            is_translating: false,
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
            "SELECT id, source, title, chinese_title, url, summary, chinese_summary, published_at, collected_at FROM crypto_news ORDER BY datetime(COALESCE(published_at, collected_at)) DESC LIMIT 200"
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
            })
        })?;
        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    fn on_sync_click(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_syncing {
            return;
        }
        self.is_syncing = true;
        self.status = Some("正在同步 RSS 新闻（请确保网络畅通）...".to_string());
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    let feeds = vec![
                        FeedConfig {
                            name: "Coindesk".to_string(),
                            url: "https://www.coindesk.com/arc/outboundfeeds/rss/".to_string(),
                        },
                        FeedConfig {
                            name: "CoinTelegraph".to_string(),
                            url: "https://cointelegraph.com/rss".to_string(),
                        },
                    ];
                    let collector = Collector::new()?;
                    let (items, reports) = collector.collect_all(&feeds).await;
                    for rep in &reports {
                        if let Some(e) = &rep.error {
                            eprintln!("[CryptoNews] Feed {} error: {}", rep.source, e);
                        }
                    }
                    let entities: Vec<NewsItemEntity> = items
                        .into_iter()
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
                    let inserted = insert_news_items(&conn, &entities)?;
                    Ok::<usize, anyhow::Error>(inserted)
                })
            })
            .join();

            _ = this.update(cx, |this, cx| {
                this.is_syncing = false;
                match result {
                    Ok(Ok(count)) => {
                        this.status = Some(format!("同步完成！成功入库 {count} 条新资讯"));
                        this.reload_news(cx);
                    }
                    Ok(Err(err)) => {
                        eprintln!("[CryptoNews] Sync error: {err:?}");
                        this.error = Some(format!("同步失败: {err}"));
                        this.status = None;
                    }
                    Err(_) => {
                        eprintln!("[CryptoNews] Sync thread panicked");
                        this.error = Some("同步线程崩溃".to_string());
                        this.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn translate_selected_news(&mut self, item_id: i64, cx: &mut Context<Self>) {
        if self.is_translating {
            return;
        }
        let Some(item) = self.news_items.iter().find(|i| i.id == item_id).cloned() else {
            return;
        };

        self.is_translating = true;
        self.status = Some("正在使用 AI 翻译当前快讯...".to_string());
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = std::thread::spawn(move || {
                let title = item.title.clone();
                let summary = item.summary.clone().unwrap_or_default();
                let prompt = format!(
                    "请将以下加密货币快讯的标题和摘要翻译为专业、流畅的中文。\n\
                    【重要规则】：\n\
                    1. 专有名词、代币符号（如 BTC、ETH、SOL、USDT）、话题标签（如 #Bitcoin、#Ethereum、#DeFi）必须严格保留英文原样，绝对不要翻译成中文。\n\
                    2. 请按以下格式返回结果，不要包含多余的废话：\n\
                    [CHINESE_TITLE]: 翻译后的中文标题\n\
                    [CHINESE_SUMMARY]: 翻译后的中文摘要/正文\n\n\
                    英文标题：{}\n\
                    英文摘要：{}",
                    title, summary
                );

                let messages = vec![ChatMessage::user(prompt)];
                let response = send_chat_blocking(&messages)?;
                let reply = response.content.trim();

                let mut cn_title = title.clone();
                let mut cn_summary = summary.clone();

                for line in reply.lines() {
                    if let Some(rest) = line.strip_prefix("[CHINESE_TITLE]:") {
                        cn_title = rest.trim().to_string();
                    } else if let Some(rest) = line.strip_prefix("[CHINESE_SUMMARY]:") {
                        cn_summary = rest.trim().to_string();
                    }
                }

                let conn = open_default_connection()?;
                update_news_translation(&conn, item_id, &cn_title, &cn_summary)?;
                Ok::<(String, String), anyhow::Error>((cn_title, cn_summary))
            })
            .join();

            _ = this.update(cx, |this, cx| {
                this.is_translating = false;
                match result {
                    Ok(Ok((cn_title, cn_summary))) => {
                        this.status = Some("AI 翻译完成并已保存！".to_string());
                        if let Some(news) = this.news_items.iter_mut().find(|i| i.id == item_id) {
                            news.chinese_title = Some(cn_title);
                            news.chinese_summary = Some(cn_summary);
                        }
                        let items_clone = this.news_items.clone();
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_news(items_clone);
                            table.refresh(cx);
                        });
                    }
                    Ok(Err(err)) => {
                        eprintln!("[CryptoNews] Translate error: {err:?}");
                        this.error = Some(format!("AI 翻译失败 (请检查 AI Provider 设置): {err}"));
                        this.status = None;
                    }
                    Err(_) => {
                        this.error = Some("AI 翻译线程崩溃".to_string());
                        this.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[derive(Clone)]
struct CryptoNewsTableDelegate {
    columns: Vec<Column>,
    news: Vec<DbNewsItem>,
    page: WeakEntity<CryptoNewsPage>,
}

impl CryptoNewsTableDelegate {
    fn new(page: WeakEntity<CryptoNewsPage>) -> Self {
        Self {
            columns: vec![
                Column::new("source", "来源").width(px(140.)).sortable(),
                Column::new("title", "标题").width(px(520.)).sortable(),
                Column::new("published_at", "发布时间")
                    .width(px(180.))
                    .sortable(),
            ],
            news: Vec::new(),
            page,
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
        let item_id = item.id;
        let page = self.page.clone();

        let display_title = item
            .chinese_title
            .clone()
            .unwrap_or_else(|| item.title.clone());

        match key {
            "source" => Button::new(("news-src", row_ix as u64))
                .ghost()
                .xsmall()
                .label(item.source.clone())
                .on_click(move |_, _, cx| {
                    _ = page.update(cx, |page, cx| {
                        page.selected_news_id = Some(item_id);
                        cx.notify();
                    });
                })
                .into_any_element(),
            "title" => Button::new(("news-title", row_ix as u64))
                .ghost()
                .xsmall()
                .label(display_title)
                .on_click(move |_, _, cx| {
                    _ = page.update(cx, |page, cx| {
                        page.selected_news_id = Some(item_id);
                        cx.notify();
                    });
                })
                .into_any_element(),
            "published_at" => Button::new(("news-time", row_ix as u64))
                .ghost()
                .xsmall()
                .label(
                    item.published_at
                        .clone()
                        .unwrap_or_else(|| item.collected_at.clone()),
                )
                .on_click(move |_, _, cx| {
                    _ = page.update(cx, |page, cx| {
                        page.selected_news_id = Some(item_id);
                        cx.notify();
                    });
                })
                .into_any_element(),
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
        let selected_item = self
            .selected_news_id
            .and_then(|id| self.news_items.iter().find(|i| i.id == id))
            .cloned();

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
                                                "实时收集主流加密 RSS 快讯，支持去重入库与 AI 中文翻译。当前共存储 {} 条资讯。",
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
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(
                        // Official gpui_component::table::Table
                        v_flex()
                            .flex_1()
                            .h_full()
                            .rounded(px(8.))
                            .bg(theme.background)
                            .border_1()
                            .border_color(border)
                            .overflow_hidden()
                            .child(
                                DataTable::new(&self.table),
                            ),
                    )
                    .when_some(selected_item, |parent, item| {
                        let item_id = item.id;
                        let has_translation = item.chinese_title.is_some() || item.chinese_summary.is_some();
                        let display_title = item.chinese_title.clone().unwrap_or_else(|| item.title.clone());
                        let display_summary = item.chinese_summary.clone()
                            .or(item.summary.clone())
                            .unwrap_or_else(|| "暂无详细正文/摘要。".to_string());

                        parent.child(
                            // Detail panel
                            v_flex()
                                .w(px(460.))
                                .h_full()
                                .rounded(px(8.))
                                .bg(theme.background)
                                .border_1()
                                .border_color(border)
                                .p_4()
                                .gap_3()
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .font_bold()
                                                .text_color(theme.accent)
                                                .child(item.source),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Button::new("translate_news_btn")
                                                        .primary()
                                                        .small()
                                                        .label(if self.is_translating {
                                                            "翻译中..."
                                                        } else if has_translation {
                                                            "重新翻译"
                                                        } else {
                                                            "🤖 AI 智能翻译"
                                                        })
                                                        .disabled(self.is_translating)
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            this.translate_selected_news(item_id, cx);
                                                        })),
                                                )
                                                .child(
                                                    Button::new("close_detail")
                                                        .ghost()
                                                        .small()
                                                        .label("关闭")
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.selected_news_id = None;
                                                            cx.notify();
                                                        })),
                                                    ),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_bold()
                                        .text_color(text_strong)
                                        .child(display_title),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(muted)
                                        .child(format!("发布时间: {}", item.published_at.unwrap_or(item.collected_at))),
                                )
                                .when_some(item.url, |p, url| {
                                    p.child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(theme.accent)
                                            .child(format!("原文链接: {url}")),
                                    )
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scrollbar()
                                        .text_size(px(13.))
                                        .text_color(palette::text(theme))
                                        .child(display_summary),
                                ),
                        )
                    }),
            )
    }
}

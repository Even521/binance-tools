use crate::ui::palette;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::Button,
    h_flex,
    table::{Column, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

use binance_tools::db::x_posting::XPublishedPost;

pub(super) struct PostHistoryPage {
    table: Entity<TableState<PostHistoryTableDelegate>>,
    error: Option<String>,
    status: Option<String>,
}

#[derive(Clone)]
struct PostHistoryTableDelegate {
    columns: Vec<Column>,
    records: Vec<XPublishedPost>,
}

impl Default for PostHistoryTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("published_at", "发布时间（北京时间）").width(px(150.)),
                Column::new("status", "发帖状态").width(px(100.)),
                Column::new("comment_status", "评论状态").width(px(100.)),
                Column::new("content", "发布正文").width(px(420.)),
                Column::new("url", "X 帖子链接").width(px(240.)),
                Column::new("news_id", "新闻 ID").width(px(100.)),
            ],
            records: Vec::new(),
        }
    }
}

impl PostHistoryTableDelegate {
    fn set_records(&mut self, records: Vec<XPublishedPost>) {
        self.records = records;
    }
}

impl TableDelegate for PostHistoryTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.records.len()
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
        let Some(record) = self.records.get(row_ix) else {
            return div().size_full().into_any_element();
        };
        let key = self.columns[col_ix].key.as_ref();
        let value = match key {
            "published_at" => app_log::format_beijing_timestamp(
                record.published_at.as_deref().unwrap_or(&record.created_at),
            ),
            "status" => match record.status.as_str() {
                "published" => "已发布".to_string(),
                "pending" => "待发布".to_string(),
                "submitting" => "发布中".to_string(),
                "unknown" => "结果待确认".to_string(),
                "failed" => "失败".to_string(),
                other => other.to_string(),
            },
            "comment_status" => match record.comment_status.as_str() {
                "pending" => "待评论".to_string(),
                "commenting" => "评论中".to_string(),
                "completed" => "已评论".to_string(),
                "unknown" => "评论结果待确认".to_string(),
                "failed" => "评论失败".to_string(),
                other => other.to_string(),
            },
            "content" => record.content.clone(),
            "url" => record.x_post_url.clone().unwrap_or_default(),
            "news_id" => record.news_id.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let color = match record.status.as_str() {
            "published" => theme.success,
            "unknown" | "submitting" => palette::muted(theme),
            "failed" => palette::error_text(),
            _ => palette::text(theme),
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .px_2()
            .text_size(px(11.))
            .text_color(if key == "status" { color } else { palette::text(theme) })
            .truncate()
            .child(value)
            .into_any_element()
    }
}

impl PostHistoryPage {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(PostHistoryTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let mut page = Self {
            table,
            error: None,
            status: None,
        };
        page.reload(cx);
        page
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.status = None;
        match binance_tools::db::x_posting::list_published_posts_blocking(500) {
            Ok(records) => self.table.update(cx, |table, cx| {
                table.delegate_mut().set_records(records);
                table.refresh(cx);
            }),
            Err(error) => self.error = Some(format!("读取 X 发帖历史失败：{error:#}")),
        }
        cx.notify();
    }
}

impl Render for PostHistoryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let table = self.table.read(cx);
        let row_count = table.delegate().records.len();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(px(6.))
                    .bg(palette::surface_strong(theme))
                    .border_1()
                    .border_color(palette::border(theme))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_size(px(16.)).font_semibold().child("发帖历史"))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(theme))
                                            .child(format!("本地保存 {row_count} 条记录；包含待发布、结果待确认和已发布状态")),
                                    ),
                            )
                            .child(
                                Button::new("x-assistant-post-history-refresh")
                                    .outline()
                                    .small()
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
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
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette::error_text())
                        .child(error),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).bordered(true)),
            )
            .into_any_element()
    }
}

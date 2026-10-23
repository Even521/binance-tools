use crate::ui::palette;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::Button,
    h_flex,
    scroll::ScrollableElement,
    table::{Column, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

use binance_tools::db::x_assistant::XAssistantReplyRecord;

pub(super) struct ReplyListPage {
    table: Entity<TableState<ReplyRecordsTableDelegate>>,
    stats: binance_tools::db::x_assistant::XAssistantReplyStats,
    error: Option<String>,
}

#[derive(Clone)]
struct ReplyRecordsTableDelegate {
    columns: Vec<Column>,
    records: Vec<XAssistantReplyRecord>,
}

impl Default for ReplyRecordsTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("created_at", "运行时间").width(px(160.)),
                Column::new("author", "发帖人").width(px(150.)),
                Column::new("account", "账号").width(px(150.)),
                Column::new("posted_at", "发帖时间").width(px(100.)),
                Column::new("post", "帖子正文").width(px(280.)),
                Column::new("reply", "生成回复").width(px(220.)),
                Column::new("completed", "最后一步完成").width(px(120.)),
            ],
            records: Vec::new(),
        }
    }
}

impl ReplyRecordsTableDelegate {
    fn set_records(&mut self, records: Vec<XAssistantReplyRecord>) {
        self.records = records;
    }
}

impl TableDelegate for ReplyRecordsTableDelegate {
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
            "created_at" => record.created_at.clone(),
            "author" => record.author_name.clone(),
            "account" => record.account_handle.clone(),
            "posted_at" => record.posted_at_text.clone(),
            "post" => record.post_text.clone(),
            "reply" => record.reply_text.clone(),
            "completed" => if record.completed_last_step {
                "是"
            } else {
                "否"
            }
            .to_string(),
            _ => String::new(),
        };
        let color = if key == "completed" && record.completed_last_step {
            theme.success
        } else {
            palette::text(theme)
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .px_2()
            .text_size(px(11.))
            .text_color(color)
            .truncate()
            .child(value)
            .into_any_element()
    }
}

impl ReplyListPage {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(ReplyRecordsTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let mut page = Self {
            table,
            stats: Default::default(),
            error: None,
        };
        page.reload(cx);
        page
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        match binance_tools::db::x_assistant::reply_stats_blocking() {
            Ok(stats) => self.stats = stats,
            Err(error) => self.error = Some(format!("读取回复统计失败：{error:#}")),
        }
        match binance_tools::db::x_assistant::list_reply_records_blocking() {
            Ok(records) => self.table.update(cx, |table, cx| {
                table.delegate_mut().set_records(records);
                table.refresh(cx);
            }),
            Err(error) => self.error = Some(format!("读取回复记录失败：{error:#}")),
        }
        cx.notify();
    }
}

impl Render for ReplyListPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let table = self.table.read(cx);
        let records = &table.delegate().records;
        let row_count = records.len();
        let selected_record = table
            .selected_row()
            .and_then(|row| records.get(row))
            .cloned();
        let has_selected_record = selected_record.is_some();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                v_flex()
                    .gap_3()
                    .p_3()
                    .rounded(px(6.))
                    .bg(palette::surface_strong(theme))
                    .border_1()
                    .border_color(palette::border(theme))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div().text_size(px(16.)).font_semibold().child("回复列表"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(theme))
                                            .child(format!("共 {row_count} 条回复记录")),
                                    ),
                            )
                            .child(
                                Button::new("x-assistant-reply-list-refresh")
                                    .outline()
                                    .small()
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(palette::border(theme))
                            .bg(palette::surface(theme))
                            .px_3()
                            .py_2()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(palette::muted(theme))
                                            .child("近 1 小时"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(20.))
                                            .font_semibold()
                                            .child(format!("{} 条", self.stats.last_hour)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(palette::border(theme))
                            .bg(palette::surface(theme))
                            .px_3()
                            .py_2()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(palette::muted(theme))
                                            .child("今日"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(20.))
                                            .font_semibold()
                                            .child(format!("{} 条", self.stats.today)),
                                    ),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .rounded(px(6.))
                        .bg(palette::error_background())
                        .px_3()
                        .py_2()
                        .text_color(palette::error_text())
                        .child(error),
                )
            })
            .child(
                v_flex().flex_1().h_full().min_h(px(180.)).w_full().child(
                    div().flex_1().size_full().overflow_hidden().child(
                        DataTable::new(&self.table)
                            .stripe(true)
                            .bordered(true)
                            .scrollbar_visible(true, true),
                    ),
                ),
            )
            .when_some(selected_record, |parent, record| {
                parent.child(
                    v_flex()
                        .h(px(260.))
                        .min_h(px(180.))
                        .gap_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette::border(theme))
                        .bg(palette::surface_strong(theme))
                        .p_3()
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .font_semibold()
                                        .child("帖子正文（完整内容）"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(palette::muted(theme))
                                        .child(format!(
                                            "{} 字符 · {}",
                                            record.post_text.chars().count(),
                                            record.account_handle
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .text_size(px(13.))
                                .child(record.post_text),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette::muted(theme))
                                .child(format!("生成回复：{}", record.reply_text)),
                        ),
                )
            })
            .when(!has_selected_record, |parent| {
                parent.child(
                    div()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette::border(theme))
                        .p_3()
                        .text_size(px(12.))
                        .text_color(palette::muted(theme))
                        .child("点击一行记录，在此查看完整帖子正文和生成回复。"),
                )
            })
    }
}

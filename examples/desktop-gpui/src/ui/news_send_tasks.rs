use crate::ui::palette;
use binance_tools::db::news_send_tasks::{NewsSendTask, PLATFORM_BINANCE, PLATFORM_X};
use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    table::{Column, Table as DataTable, TableDelegate, TableState},
    v_flex,
};

pub struct NewsSendTasksPage {
    table: Entity<TableState<NewsSendTasksTableDelegate>>,
    target_mode: String,
    status: Option<String>,
    error: Option<String>,
}

#[derive(Clone)]
struct NewsSendTasksTableDelegate {
    columns: Vec<Column>,
    tasks: Vec<NewsSendTask>,
}

impl Default for NewsSendTasksTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("news_title", "热点新闻").width(px(310.)).sortable(),
                Column::new("platform", "发送平台").width(px(130.)).sortable(),
                Column::new("status", "发送状态").width(px(110.)).sortable(),
                Column::new("content", "发送内容").width(px(390.)),
                Column::new("created_at", "任务时间").width(px(170.)).sortable(),
                Column::new("last_error", "结果 / 错误").width(px(240.)),
            ],
            tasks: Vec::new(),
        }
    }
}

impl NewsSendTasksTableDelegate {
    fn set_tasks(&mut self, tasks: Vec<NewsSendTask>) {
        self.tasks = tasks;
    }
}

impl TableDelegate for NewsSendTasksTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.tasks.len()
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
        let Some(task) = self.tasks.get(row_ix) else {
            return div().size_full().into_any_element();
        };
        let key = self.columns[col_ix].key.as_ref();
        let value = match key {
            "news_title" => task.news_title.clone(),
            "platform" => match task.platform.as_str() {
                PLATFORM_X => "X / Twitter".to_string(),
                PLATFORM_BINANCE => "币安广场".to_string(),
                _ => task.platform.clone(),
            },
            "status" => match task.status.as_str() {
                "pending" => "待发送",
                "sending" => "发送中",
                "sent" => "已发送",
                "failed" => "发送失败",
                "skipped" => "已跳过",
                "unknown" => "结果待核实",
                _ => "未知",
            }
            .to_string(),
            "content" => task.content.clone(),
            "created_at" => task.created_at.clone(),
            "last_error" => task.last_error.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let color = match task.status.as_str() {
            "sent" => theme.success,
            "failed" | "unknown" => theme.danger,
            _ => palette::text(theme),
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

impl NewsSendTasksPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(NewsSendTasksTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let mut page = Self {
            table,
            target_mode: "all".to_string(),
            status: None,
            error: None,
        };
        page.reload(cx);
        page
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        match binance_tools::db::news_send_tasks::load_target_mode_blocking() {
            Ok(mode) => self.target_mode = mode,
            Err(error) => self.error = Some(format!("读取发送平台设置失败：{error:#}")),
        }
        match binance_tools::db::news_send_tasks::list_news_send_tasks_blocking(1000) {
            Ok(tasks) => {
                self.status = Some(format!("当前显示 {} 条平台发送任务", tasks.len()));
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_tasks(tasks);
                    table.refresh(cx);
                });
            }
            Err(error) => self.error = Some(format!("加载发送任务失败：{error:#}")),
        }
        cx.notify();
    }

    fn set_target_mode(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        match binance_tools::db::news_send_tasks::save_target_mode_blocking(mode) {
            Ok(()) => {
                self.status = Some(match mode {
                    "x" => "已设置为只发送到 X".to_string(),
                    "binance" => "已设置为只发送到币安广场".to_string(),
                    _ => "已设置为发送到所有已启用平台".to_string(),
                });
                self.reload(cx);
            }
            Err(error) => self.error = Some(format!("保存发送平台设置失败：{error:#}")),
        }
        cx.notify();
    }
}

impl Render for NewsSendTasksPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let surface = palette::surface_strong(theme);
        let border = palette::border(theme);
        let muted = palette::muted(theme);
        v_flex()
            .size_full()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_size(px(20.)).font_semibold().child("新闻发送任务"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(muted)
                                    .child("选择自动发帖的平台；币安广场需先配置 API Key，全部模式会并行发送。"),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("news-send-target-all")
                                    .when(self.target_mode == "all", |button| button.primary())
                                    .when(self.target_mode != "all", |button| button.outline())
                                    .small()
                                    .label("全部")
                                    .on_click(cx.listener(|this, _, _, cx| this.set_target_mode("all", cx))),
                            )
                            .child(
                                Button::new("news-send-target-x")
                                    .when(self.target_mode == "x", |button| button.primary())
                                    .when(self.target_mode != "x", |button| button.outline())
                                    .small()
                                    .label("X")
                                    .on_click(cx.listener(|this, _, _, cx| this.set_target_mode("x", cx))),
                            )
                            .child(
                                Button::new("news-send-target-binance")
                                    .when(self.target_mode == "binance", |button| button.primary())
                                    .when(self.target_mode != "binance", |button| button.outline())
                                    .small()
                                    .label("币安")
                                    .on_click(cx.listener(|this, _, _, cx| this.set_target_mode("binance", cx))),
                            )
                            .child(
                                Button::new("refresh-news-send-tasks")
                                    .outline()
                                    .small()
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    ),
            )
            .when_some(self.status.clone(), |parent, status| {
                parent.child(div().text_size(px(12.)).text_color(theme.success).child(status))
            })
            .when_some(self.error.clone(), |parent, error| {
                parent.child(div().text_size(px(12.)).text_color(theme.danger).child(error))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .rounded(px(8.))
                    .bg(surface)
                    .border_1()
                    .border_color(border)
                    .child(
                        DataTable::new(&self.table)
                            .stripe(true)
                            .bordered(true)
                            .scrollbar_visible(true, true),
                    ),
            )
    }
}

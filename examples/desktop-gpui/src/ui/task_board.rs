use crate::ui::palette;
use binance_tools::db::task_board::{
    ToolBoardTask, create_tool_board_task_blocking, delete_tool_board_task_blocking,
    list_tool_board_tasks_blocking, set_tool_board_task_completed_blocking,
    update_tool_board_task_blocking,
};
use chrono::{Duration as ChronoDuration, Local, NaiveDateTime};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::time::Duration;

pub struct TaskBoardPage {
    tasks: Vec<ToolBoardTask>,
    title_input: Entity<InputState>,
    note_input: Entity<InputState>,
    due_input: Entity<InputState>,
    status: Option<String>,
    error: Option<String>,
    editor_open: bool,
    editing_task_id: Option<i64>,
    detail_task_id: Option<i64>,
    _tick_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl TaskBoardPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("任务标题")
                .default_value("")
        });
        let note_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("备注，可选")
                .default_value("")
        });
        let due_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("提醒时间，例如 2026-05-26 18:30")
                .default_value(default_due_text())
        });
        let _subscriptions = vec![
            cx.subscribe_in(&title_input, window, Self::on_input_event),
            cx.subscribe_in(&note_input, window, Self::on_input_event),
            cx.subscribe_in(&due_input, window, Self::on_input_event),
        ];
        let _tick_task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_secs(30)).await;
                _ = this.update(cx, |_, cx| cx.notify());
            }
        });

        let mut this = Self {
            tasks: Vec::new(),
            title_input,
            note_input,
            due_input,
            status: None,
            error: None,
            editor_open: false,
            editing_task_id: None,
            detail_task_id: None,
            _tick_task,
            _subscriptions,
        };
        this.reload();
        this
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.status = None;
            self.error = None;
            cx.notify();
        }
    }

    fn reload(&mut self) {
        match list_tool_board_tasks_blocking() {
            Ok(tasks) => {
                self.tasks = tasks;
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn open_create_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_open = true;
        self.editing_task_id = None;
        self.detail_task_id = None;
        self.error = None;
        self.status = None;
        self.title_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.note_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.due_input.update(cx, |input, cx| {
            input.set_value(default_due_text(), window, cx)
        });
        cx.notify();
    }

    fn open_detail(&mut self, task_id: i64, cx: &mut Context<Self>) {
        self.detail_task_id = Some(task_id);
        self.editor_open = false;
        self.editing_task_id = None;
        self.error = None;
        self.status = None;
        cx.notify();
    }

    fn open_edit_editor(&mut self, task_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.iter().find(|task| task.id == task_id).cloned() else {
            self.error = Some("任务不存在".to_string());
            cx.notify();
            return;
        };
        self.editor_open = true;
        self.editing_task_id = Some(task_id);
        self.detail_task_id = None;
        self.error = None;
        self.status = None;
        self.title_input
            .update(cx, |input, cx| input.set_value(task.title, window, cx));
        self.note_input
            .update(cx, |input, cx| input.set_value(task.note, window, cx));
        self.due_input
            .update(cx, |input, cx| input.set_value(task.due_at, window, cx));
        cx.notify();
    }

    fn close_dialogs(&mut self, cx: &mut Context<Self>) {
        self.editor_open = false;
        self.editing_task_id = None;
        self.detail_task_id = None;
        self.error = None;
        cx.notify();
    }

    fn save_editor_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title_input.read(cx).text().to_string();
        let note = self.note_input.read(cx).text().to_string();
        let due_text = self.due_input.read(cx).text().to_string();
        let due_at = match normalize_due_at(&due_text) {
            Ok(value) => value,
            Err(err) => {
                self.error = Some(err);
                self.status = None;
                cx.notify();
                return;
            }
        };

        if title.trim().is_empty() {
            self.error = Some("任务标题不能为空".to_string());
            self.status = None;
            cx.notify();
            return;
        }

        let result = if let Some(task_id) = self.editing_task_id {
            update_tool_board_task_blocking(task_id, title, note, due_at).map(|_| task_id)
        } else {
            create_tool_board_task_blocking(title, note, due_at)
        };

        match result {
            Ok(task_id) => {
                self.reload();
                self.status = Some(
                    if self.editing_task_id.is_some() {
                        "任务已更新"
                    } else {
                        "任务已添加"
                    }
                    .to_string(),
                );
                self.error = None;
                self.editor_open = false;
                self.editing_task_id = None;
                self.detail_task_id = Some(task_id);
                self.title_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.note_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.due_input.update(cx, |input, cx| {
                    input.set_value(default_due_text(), window, cx)
                });
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn toggle_completed(&mut self, task_id: i64, completed: bool, cx: &mut Context<Self>) {
        match set_tool_board_task_completed_blocking(task_id, completed) {
            Ok(()) => {
                self.reload();
                self.status = Some(
                    if completed {
                        "任务已完成"
                    } else {
                        "任务已恢复"
                    }
                    .to_string(),
                );
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn delete_task(&mut self, task_id: i64, cx: &mut Context<Self>) {
        match delete_tool_board_task_blocking(task_id) {
            Ok(()) => {
                self.reload();
                self.status = Some("任务已删除".to_string());
                self.error = None;
                if self.detail_task_id == Some(task_id) {
                    self.detail_task_id = None;
                }
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn due_tasks_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task_due_state(task) == DueState::Due)
            .count()
    }

    fn pending_count(&self) -> usize {
        self.tasks.iter().filter(|task| !task.completed).count()
    }

    fn completed_count(&self) -> usize {
        self.tasks.iter().filter(|task| task.completed).count()
    }

    fn selected_detail_task(&self) -> Option<ToolBoardTask> {
        self.detail_task_id
            .and_then(|task_id| self.tasks.iter().find(|task| task.id == task_id).cloned())
    }

    fn render_labeled_input(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child(label),
            )
            .child(
                div()
                    .min_h(px(32.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .px_2()
                    .py_1()
                    .child(Input::new(input).appearance(false)),
            )
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let message = self.error.clone().or_else(|| self.status.clone());
        let is_error = self.error.is_some();

        div()
            .min_h(px(28.))
            .when_some(message, |parent, message| {
                parent.child(
                    div()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if is_error {
                            palette::error_border()
                        } else {
                            app_theme.success.opacity(0.35)
                        })
                        .bg(if is_error {
                            palette::error_background()
                        } else {
                            app_theme.success.opacity(0.10)
                        })
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(if is_error {
                            palette::error_text()
                        } else {
                            palette::text(app_theme)
                        })
                        .child(message),
                )
            })
            .into_any_element()
    }

    fn render_task_card(&self, task: ToolBoardTask, cx: &mut Context<Self>) -> AnyElement {
        let due_state = task_due_state(&task);
        let is_due = due_state == DueState::Due;
        let is_soon = due_state == DueState::Soon;
        let task_id = task.id;
        let completed = task.completed;
        let note = task.note.trim().to_string();
        let note_empty = note.is_empty();
        let due_badge = self.render_due_badge(due_state, completed, cx);
        let app_theme = cx.theme();

        v_flex()
            .w(px(280.))
            .min_h(px(150.))
            .gap_3()
            .rounded(px(6.))
            .border_1()
            .cursor_pointer()
            .border_color(if is_due {
                palette::error_border()
            } else if is_soon {
                app_theme.warning.opacity(0.42)
            } else {
                palette::border(app_theme)
            })
            .bg(if completed {
                app_theme.group_box.opacity(0.35)
            } else if is_due {
                palette::error_background()
            } else if is_soon {
                app_theme.warning.opacity(0.10)
            } else {
                app_theme.background
            })
            .p_3()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.open_detail(task_id, cx);
                }),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(15.))
                            .font_semibold()
                            .text_color(if completed {
                                palette::muted(app_theme)
                            } else {
                                palette::text_strong(app_theme)
                            })
                            .child(task.title.clone()),
                    )
                    .child(due_badge),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child(format!("提醒时间：{}", task.due_at)),
            )
            .when(!note_empty, |parent| {
                parent.child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .text_color(palette::text(app_theme))
                        .child(summary_text(&note, 96)),
                )
            })
            .when(note_empty, |parent| parent.child(div().flex_1()))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(("complete-task", task_id as u64))
                            .outline()
                            .xsmall()
                            .icon(if completed {
                                IconName::Undo2
                            } else {
                                IconName::Check
                            })
                            .label(if completed { "恢复" } else { "完成" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_completed(task_id, !completed, cx);
                            })),
                    )
                    .child(
                        Button::new(("delete-task", task_id as u64))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Delete).size_4())
                            .tooltip("删除任务")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.delete_task(task_id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_due_badge(
        &self,
        due_state: DueState,
        completed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        let (label, bg, fg) = if completed {
            (
                "已完成",
                app_theme.success.opacity(0.12),
                app_theme.success.opacity(0.92),
            )
        } else {
            match due_state {
                DueState::Due => ("到期", palette::error_background(), palette::error_text()),
                DueState::Soon => (
                    "临近",
                    app_theme.warning.opacity(0.16),
                    app_theme.warning.opacity(0.92),
                ),
                DueState::Future => (
                    "待办",
                    app_theme.muted.opacity(0.20),
                    palette::muted(app_theme),
                ),
            }
        };

        div()
            .flex_none()
            .rounded(px(4.))
            .px_2()
            .py_1()
            .bg(bg)
            .text_size(px(11.))
            .text_color(fg)
            .child(label)
            .into_any_element()
    }

    fn render_editor_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let title_input = self.render_labeled_input("标题", &self.title_input, cx);
        let due_input = self.render_labeled_input("提醒时间", &self.due_input, cx);
        let note_input = self.render_labeled_input("备注", &self.note_input, cx);
        let status = self.render_status(cx);
        let app_theme = cx.theme();
        let editing = self.editing_task_id.is_some();

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(gpui::black().opacity(0.18))
            .child(
                v_flex()
                    .absolute()
                    .top(px(86.))
                    .left(px(220.))
                    .right(px(220.))
                    .min_w(px(560.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .shadow_md()
                    .bg(app_theme.background)
                    .child(
                        h_flex()
                            .items_center()
                            .h(px(42.))
                            .px_3()
                            .border_b_1()
                            .border_color(palette::border(app_theme))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(16.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(app_theme))
                                    .child(if editing {
                                        "修改任务"
                                    } else {
                                        "新增任务"
                                    }),
                            )
                            .child(
                                Button::new("close-task-editor")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Close).size_4())
                                    .tooltip("关闭")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_dialogs(cx);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_3()
                            .p_4()
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(div().flex_1().child(title_input))
                                    .child(div().w(px(240.)).child(due_input)),
                            )
                            .child(note_input)
                            .child(status)
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("cancel-task-editor")
                                            .outline()
                                            .small()
                                            .label("取消")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.close_dialogs(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("save-task-editor")
                                            .primary()
                                            .small()
                                            .label(if editing {
                                                "保存修改"
                                            } else {
                                                "添加任务"
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save_editor_task(window, cx);
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_detail_dialog(&self, task: ToolBoardTask, cx: &mut Context<Self>) -> AnyElement {
        let task_id = task.id;
        let completed = task.completed;
        let due_badge = self.render_due_badge(task_due_state(&task), completed, cx);
        let app_theme = cx.theme();

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(gpui::black().opacity(0.18))
            .child(
                v_flex()
                    .absolute()
                    .top(px(72.))
                    .left(px(180.))
                    .right(px(180.))
                    .bottom(px(72.))
                    .min_w(px(620.))
                    .min_h(px(420.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .shadow_md()
                    .bg(app_theme.background)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .h(px(44.))
                            .px_3()
                            .border_b_1()
                            .border_color(palette::border(app_theme))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(17.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(app_theme))
                                    .child(task.title.clone()),
                            )
                            .child(due_badge)
                            .child(
                                Button::new("close-task-detail")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Close).size_4())
                                    .tooltip("关闭")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_dialogs(cx);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .overflow_y_scrollbar()
                            .gap_3()
                            .p_4()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(app_theme))
                                    .child(format!("提醒时间：{}", task.due_at)),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(palette::muted(app_theme))
                                    .child(format!(
                                        "创建：{}    更新：{}",
                                        task.created_at, task.updated_at
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(palette::border(app_theme))
                                    .bg(app_theme.group_box.opacity(0.18))
                                    .p_3()
                                    .text_size(px(14.))
                                    .text_color(palette::text(app_theme))
                                    .child(if task.note.trim().is_empty() {
                                        "没有备注".to_string()
                                    } else {
                                        task.note.clone()
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .border_t_1()
                            .border_color(palette::border(app_theme))
                            .p_3()
                            .child(
                                Button::new("delete-task-detail")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Delete).size_4())
                                    .label("删除")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.delete_task(task_id, cx);
                                    })),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("toggle-task-detail")
                                            .outline()
                                            .small()
                                            .icon(if completed {
                                                IconName::Undo2
                                            } else {
                                                IconName::Check
                                            })
                                            .label(if completed { "恢复" } else { "完成" })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle_completed(task_id, !completed, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("edit-task-detail")
                                            .primary()
                                            .small()
                                            .icon(IconName::Settings2)
                                            .label("修改")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_edit_editor(task_id, window, cx);
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for TaskBoardPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let due_count = self.due_tasks_count();
        let pending_count = self.pending_count();
        let completed_count = self.completed_count();
        let status = self.render_status(cx);
        let cards = self
            .tasks
            .clone()
            .into_iter()
            .map(|task| self.render_task_card(task, cx))
            .collect::<Vec<_>>();
        let has_tasks = !cards.is_empty();
        let detail_task = self.selected_detail_task();
        let editor_dialog = self.editor_open.then(|| self.render_editor_dialog(cx));
        let detail_dialog = detail_task
            .filter(|_| !self.editor_open)
            .map(|task| self.render_detail_dialog(task, cx));
        let app_theme = cx.theme();

        v_flex()
            .size_full()
            .relative()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .p_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(app_theme))
                                    .child("任务看板"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(app_theme))
                                    .child("卡片式记录任务；提醒时间到了会在看板中高亮。"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(summary_pill(
                                "到期",
                                due_count,
                                palette::error_background(),
                                palette::error_text(),
                            ))
                            .child(summary_pill(
                                "待办",
                                pending_count,
                                app_theme.muted.opacity(0.22),
                                palette::text(app_theme),
                            ))
                            .child(summary_pill(
                                "完成",
                                completed_count,
                                app_theme.success.opacity(0.12),
                                app_theme.success.opacity(0.92),
                            )),
                    )
                    .child(
                        Button::new("open-create-board-task")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label("新增")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_editor(window, cx);
                            })),
                    ),
            )
            .when(due_count > 0, |parent| {
                parent.child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(palette::error_border())
                        .bg(palette::error_background())
                        .px_3()
                        .py_2()
                        .text_color(palette::error_text())
                        .child(Icon::new(IconName::Bell).size_4())
                        .child(format!("有 {due_count} 个任务已经到提醒时间")),
                )
            })
            .child(status)
            .child(
                div().flex_1().overflow_y_scrollbar().child(
                    h_flex()
                        .items_start()
                        .gap_3()
                        .flex_wrap()
                        .children(cards)
                        .when(!has_tasks, |parent| {
                            parent.child(
                                div()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(palette::border(app_theme))
                                    .bg(app_theme.background)
                                    .p_6()
                                    .text_size(px(13.))
                                    .text_color(palette::muted(app_theme))
                                    .child("还没有任务"),
                            )
                        }),
                ),
            )
            .when_some(detail_dialog, |parent, dialog| parent.child(dialog))
            .when_some(editor_dialog, |parent, dialog| parent.child(dialog))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DueState {
    Due,
    Soon,
    Future,
}

fn task_due_state(task: &ToolBoardTask) -> DueState {
    if task.completed {
        return DueState::Future;
    }
    let now = Local::now().naive_local();
    let due_at = parse_due_at(&task.due_at).unwrap_or(now);
    if due_at <= now {
        DueState::Due
    } else if due_at - now <= ChronoDuration::minutes(30) {
        DueState::Soon
    } else {
        DueState::Future
    }
}

fn normalize_due_at(value: &str) -> Result<String, String> {
    let value = value.trim();
    let due_at = parse_due_at(value).ok_or_else(|| {
        "提醒时间格式错误，请使用 YYYY-MM-DD HH:MM 或 YYYY-MM-DD HH:MM:SS".to_string()
    })?;
    Ok(due_at.format("%Y-%m-%d %H:%M:%S").to_string())
}

fn parse_due_at(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M"))
        .ok()
}

fn default_due_text() -> String {
    (Local::now() + ChronoDuration::hours(1))
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn summary_pill(label: &str, count: usize, bg: Hsla, fg: Hsla) -> AnyElement {
    h_flex()
        .items_center()
        .gap_1()
        .rounded(px(4.))
        .px_2()
        .py_1()
        .bg(bg)
        .text_color(fg)
        .child(div().text_size(px(12.)).child(label.to_string()))
        .child(
            div()
                .text_size(px(13.))
                .font_semibold()
                .child(count.to_string()),
        )
        .into_any_element()
}

fn summary_text(value: &str, max_chars: usize) -> String {
    let compact = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let mut output = compact.chars().take(max_chars).collect::<String>();
        output.push_str("...");
        output
    }
}

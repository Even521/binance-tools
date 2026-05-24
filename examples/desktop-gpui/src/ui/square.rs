use crate::ui::palette;
use binance_tools::{
    db::square::{
        BinanceSquareSendLog, BinanceSquareTask, SQUARE_TASK_STATUS_DRAFT,
        SQUARE_TASK_STATUS_FAILED, SQUARE_TASK_STATUS_PENDING, SQUARE_TASK_STATUS_SENT,
        SQUARE_TASK_STATUS_SKIPPED,
    },
    square::{SquareAiGenerationSummary, SquareAutomationSummary, SquareTaskRunSummary},
};
use chrono::{Duration as ChronoDuration, Local, NaiveDateTime, TimeZone, Timelike};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    switch::Switch,
    table::{Column, ColumnSort, Table as DataTable, TableDelegate, TableEvent, TableState},
    v_flex,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct SquareKeySettingsPage {
    api_key_input: Entity<InputState>,
    status: Option<String>,
    _task: Task<()>,
}

impl SquareKeySettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let api_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入币安广场 API Key")
                .default_value("")
        });

        let mut this = Self {
            api_key_input,
            status: None,
            _task: Task::ready(()),
        };
        this.load_key(cx);
        this
    }

    fn load_key(&mut self, cx: &mut Context<Self>) {
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::load_square_api_key_blocking()
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(key)) => {
                        let masked_key = mask_key(&key.api_key);
                        this.status = Some(format!(
                            "已加载 {}，更新时间 {}",
                            masked_key, key.updated_at
                        ));
                    }
                    Ok(None) => {
                        this.status = Some("未设置 Key".to_string());
                    }
                    Err(err) => {
                        this.status = Some(err.to_string());
                    }
                }
                cx.notify();
            });
        });
    }

    fn save_key(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let api_key = self.api_key_input.read(cx).value().to_string();
        self.status = Some("保存中".to_string());

        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::save_square_api_key_blocking(api_key)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(()) => "Key 已保存到 SQLite".to_string(),
                    Err(err) => err.to_string(),
                });
                cx.notify();
            });
        });
    }
}

fn mask_key(api_key: &str) -> String {
    if api_key.len() <= 8 {
        "********".to_string()
    } else {
        format!("{}****{}", &api_key[..4], &api_key[api_key.len() - 4..])
    }
}

impl Render for SquareKeySettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .size_full()
            .child(
                v_flex()
                    .gap_1()
                    .p_4()
                    .rounded(px(8.))
                    .bg(palette::surface_strong(cx.theme()))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_semibold()
                            .child("币安广场 Key 设置"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(cx.theme()))
                            .child("API Key 保存到本地 SQLite，用于发送币安广场消息任务。"),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .max_w(px(520.))
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(Input::new(&self.api_key_input))
                    .child(
                        h_flex().gap_2().child(
                            Button::new("save-square-key")
                                .primary()
                                .xsmall()
                                .label("保存 Key")
                                .on_click(cx.listener(Self::save_key)),
                        ),
                    ),
            )
            .when_some(self.status.clone(), |this, status| {
                this.child(
                    div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(palette::surface_strong(cx.theme()))
                        .text_size(px(12.))
                        .child(status),
                )
            })
    }
}

pub struct SquareTasksPage {
    table: Entity<TableState<SquareTasksTableDelegate>>,
    name_input: Entity<InputState>,
    message_input: Entity<InputState>,
    scheduled_at_input: Entity<InputState>,
    selected_task_id: Option<i64>,
    selected_task_status: Option<String>,
    status_picker_open: bool,
    pending_delete_task_id: Option<i64>,
    ai_analysis_enabled: bool,
    ai_next_run_at: Option<String>,
    ai_generating: bool,
    automation_collapsed: bool,
    editor_collapsed: bool,
    scheduler_running: bool,
    scheduler_next_run_at: Option<String>,
    scheduler_enabled: Arc<AtomicBool>,
    status: Option<String>,
    _task: Task<()>,
    _settings_task: Task<()>,
    _ai_generation_task: Task<()>,
    _scheduler_task: Task<()>,
    _countdown_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SquareTasksPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(SquareTasksTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let _subscriptions = vec![cx.subscribe_in(&table, window, Self::on_table_event)];
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("任务名称")
                .default_value("币安广场定时消息")
        });
        let message_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(5)
                .placeholder("输入要发送到币安广场的正文；AI 草稿可在表格中选中后回填修改")
        });
        let scheduled_at_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("预计发送时间，留空表示当前时间")
                .default_value("")
        });

        let mut this = Self {
            table,
            name_input,
            message_input,
            scheduled_at_input,
            selected_task_id: None,
            selected_task_status: None,
            status_picker_open: false,
            pending_delete_task_id: None,
            ai_analysis_enabled: false,
            ai_next_run_at: None,
            ai_generating: false,
            automation_collapsed: false,
            editor_collapsed: false,
            scheduler_running: false,
            scheduler_next_run_at: None,
            scheduler_enabled: Arc::new(AtomicBool::new(false)),
            status: None,
            _task: Task::ready(()),
            _settings_task: Task::ready(()),
            _ai_generation_task: Task::ready(()),
            _scheduler_task: Task::ready(()),
            _countdown_task: Task::ready(()),
            _subscriptions,
        };
        this.load_tasks(cx);
        this.load_ai_settings(cx);
        this.start_countdown_ticker(cx);
        this
    }

    fn load_ai_settings(&mut self, cx: &mut Context<Self>) {
        self._settings_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::load_square_ai_settings_blocking()
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(settings) => {
                        this.ai_analysis_enabled = settings.enabled;
                        this.ai_next_run_at = settings.next_run_at;
                    }
                    Err(err) => {
                        this.status = Some(err.to_string());
                    }
                }
                cx.notify();
            });
        });
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_tasks(cx);
        self.load_ai_settings(cx);
    }

    fn load_tasks(&mut self, cx: &mut Context<Self>) {
        match binance_tools::db::square::list_square_tasks_blocking() {
            Ok(tasks) => {
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_tasks(tasks);
                    table.refresh(cx);
                });
            }
            Err(err) => {
                self.status = Some(err.to_string());
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().set_error();
                    table.refresh(cx);
                });
            }
        }
        cx.notify();
    }

    fn save_task(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_task_id.is_some() {
            self.status = Some("当前已选中任务；请使用“保存修改”，或先取消选择再新增".to_string());
            cx.notify();
            return;
        }
        let name = self.name_input.read(cx).value().to_string();
        let message = self.message_input.read(cx).value().to_string();
        let scheduled_at = self.scheduled_at_input.read(cx).value().trim().to_string();
        if message.trim().is_empty() {
            self.status = Some("任务内容不能为空".to_string());
            cx.notify();
            return;
        }

        self.status = Some("正在保存任务".to_string());

        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::save_square_task_blocking(
                        name,
                        message,
                        (!scheduled_at.is_empty()).then_some(scheduled_at),
                    )
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(_) => "任务已保存".to_string(),
                    Err(err) => err.to_string(),
                });
                cx.notify();
                this.load_tasks(cx);
            });
        });
    }

    fn update_selected_task(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self.selected_task_id else {
            self.status = Some("请先在任务表格中选中一条任务".to_string());
            cx.notify();
            return;
        };
        let name = self.name_input.read(cx).value().to_string();
        let message = self.message_input.read(cx).value().to_string();
        let scheduled_at = self.scheduled_at_input.read(cx).value().trim().to_string();
        if message.trim().is_empty() {
            self.status = Some("任务内容不能为空".to_string());
            cx.notify();
            return;
        }

        let title = extract_task_title(&message);
        self.status = Some("正在更新任务".to_string());
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::update_square_task_blocking(
                        task_id,
                        title,
                        name,
                        message,
                        (!scheduled_at.is_empty()).then_some(scheduled_at),
                    )
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(()) => "任务已更新".to_string(),
                    Err(err) => err.to_string(),
                });
                cx.notify();
                this.load_tasks(cx);
            });
        });
    }

    fn set_selected_task_status(
        &mut self,
        next_status: &'static str,
        status_label: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(task_id) = self.selected_task_id else {
            self.status = Some("请先在任务表格中选中一条任务".to_string());
            cx.notify();
            return;
        };

        self.status_picker_open = false;
        self.pending_delete_task_id = None;
        self.status = Some(format!("正在修改状态为 {status_label}"));
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::mark_square_task_status_blocking(
                        task_id,
                        next_status,
                    )
                })
                .await;

            _ = this.update(cx, |this, cx| {
                if result.is_ok() {
                    this.selected_task_status = Some(next_status.to_string());
                }
                this.status = Some(match result {
                    Ok(()) => format!("任务状态已改为 {status_label}"),
                    Err(err) => err.to_string(),
                });
                cx.notify();
                this.load_tasks(cx);
            });
        });
    }

    fn toggle_status_picker(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_task_id.is_none() {
            self.status = Some("请先在任务表格中选中一条任务".to_string());
            cx.notify();
            return;
        }
        self.status_picker_open = !self.status_picker_open;
        cx.notify();
    }

    fn delete_selected_task(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task_id) = self.selected_task_id else {
            self.status = Some("请先在任务表格中选中一条任务".to_string());
            cx.notify();
            return;
        };

        if self.pending_delete_task_id != Some(task_id) {
            self.pending_delete_task_id = Some(task_id);
            self.status = Some(format!("再次点击“确认删除”才会删除任务 #{task_id}"));
            cx.notify();
            return;
        }

        self.status = Some(format!("正在删除任务 #{task_id}"));
        match binance_tools::db::square::delete_square_task_blocking(task_id) {
            Ok(()) => {
                self.clear_selected_task(window, cx);
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().remove_task(task_id);
                    table.refresh(cx);
                });
                self.status = Some(format!("任务 #{task_id} 已删除"));
                self.load_tasks(cx);
            }
            Err(err) => {
                self.status = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn clear_selected_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_task_id = None;
        self.selected_task_status = None;
        self.status_picker_open = false;
        self.pending_delete_task_id = None;
        self.name_input.update(cx, |input, cx| {
            input.set_value("币安广场定时消息".to_string(), window, cx)
        });
        self.message_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.scheduled_at_input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
    }

    fn clear_selected_task_from_click(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_selected_task(window, cx);
        self.status = Some("已取消选择，可以新增任务".to_string());
        cx.notify();
    }

    fn set_scheduled_at(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.scheduled_at_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
        cx.notify();
    }

    fn set_scheduled_now(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.set_scheduled_at(format_local_datetime(Local::now()), window, cx);
    }

    fn set_scheduled_after_30m(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scheduled_at(
            format_local_datetime(Local::now() + ChronoDuration::minutes(30)),
            window,
            cx,
        );
    }

    fn set_scheduled_after_1h(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scheduled_at(
            format_local_datetime(Local::now() + ChronoDuration::hours(1)),
            window,
            cx,
        );
    }

    fn set_scheduled_tomorrow_morning(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tomorrow = Local::now() + ChronoDuration::days(1);
        let value = tomorrow
            .with_hour(9)
            .and_then(|value| value.with_minute(0))
            .and_then(|value| value.with_second(0))
            .unwrap_or(tomorrow);
        self.set_scheduled_at(format_local_datetime(value), window, cx);
    }

    fn on_table_event(
        &mut self,
        _: &Entity<TableState<SquareTasksTableDelegate>>,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TableEvent::SelectRow(row_ix) = event else {
            return;
        };
        let task = self.table.read(cx).delegate().task_at(*row_ix).cloned();
        let Some(task) = task else {
            return;
        };

        self.selected_task_id = Some(task.id);
        self.selected_task_status = Some(task.send_status.clone());
        self.status_picker_open = false;
        self.pending_delete_task_id = None;
        self.name_input.update(cx, |input, cx| {
            input.set_value(task.name.clone(), window, cx)
        });
        self.message_input.update(cx, |input, cx| {
            input.set_value(task.message.clone(), window, cx)
        });
        self.scheduled_at_input.update(cx, |input, cx| {
            input.set_value(task.scheduled_at.clone(), window, cx)
        });
        self.status = Some(format!(
            "已选中任务 #{}，状态 {}",
            task.id, task.send_status
        ));
        cx.notify();
    }

    fn toggle_ai_analysis_from_switch(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.set_ai_analysis_enabled(enabled, cx);
    }

    fn set_ai_analysis_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.ai_analysis_enabled {
            return;
        }
        self.ai_analysis_enabled = enabled;
        self.status = Some(if enabled {
            "正在开启 AI 分析任务".to_string()
        } else {
            "正在关闭 AI 分析任务".to_string()
        });

        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::save_square_ai_settings_blocking(enabled)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(()) if enabled => "AI 分析已开启：每 1 小时生成一条任务".to_string(),
                    Ok(()) => "AI 分析已关闭".to_string(),
                    Err(err) => err.to_string(),
                });
                cx.notify();
                this.load_ai_settings(cx);
            });
        });
        cx.notify();
    }

    fn run_ai_analysis_now(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.ai_generating {
            return;
        }
        self.ai_generating = true;
        self.status = Some("正在执行 AI 分析并生成任务".to_string());
        cx.notify();

        self._ai_generation_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::square::run_square_ai_generation_now_blocking()
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.ai_generating = false;
                this.status = Some(match result {
                    Ok(summary) => format_ai_summary(&summary),
                    Err(err) => err.to_string(),
                });
                cx.notify();
                this.load_ai_settings(cx);
                this.load_tasks(cx);
            });
        });
    }

    fn run_due_tasks(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.status = Some("正在执行到期任务".to_string());

        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { binance_tools::square::run_due_square_tasks_blocking() },
                )
                .await;

            _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(summary) => format_run_summary(&summary),
                    Err(err) => err.to_string(),
                });
                cx.notify();
                this.load_tasks(cx);
            });
        });
    }

    fn send_now(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let message = self.message_input.read(cx).value().to_string();
        if message.trim().is_empty() {
            self.status = Some("发送内容不能为空".to_string());
            cx.notify();
            return;
        }
        let selected_task_id = self.selected_task_id;

        self.status = Some("正在即时发送".to_string());
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(task_id) = selected_task_id {
                        binance_tools::square::send_square_task_message_now_blocking(
                            task_id, message,
                        )
                    } else {
                        binance_tools::square::send_square_message_now_blocking(message)
                    }
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.status = Some(match &result {
                    Ok(result) => format!(
                        "即时发送完成：{}{}",
                        result.status.as_str(),
                        result
                            .error_message
                            .as_ref()
                            .map(|value| format!("，{value}"))
                            .unwrap_or_default()
                    ),
                    Err(err) => err.to_string(),
                });
                if let Some(task_id) = selected_task_id {
                    this.selected_task_status = Some(match &result {
                        Ok(result) => match &result.status {
                            binance_tools::square::SquareSendStatus::Success => {
                                SQUARE_TASK_STATUS_SENT.to_string()
                            }
                            binance_tools::square::SquareSendStatus::Skipped => {
                                binance_tools::db::square::SQUARE_TASK_STATUS_SKIPPED.to_string()
                            }
                            binance_tools::square::SquareSendStatus::Failed
                            | binance_tools::square::SquareSendStatus::DailyLimit
                            | binance_tools::square::SquareSendStatus::KeyExpired => {
                                SQUARE_TASK_STATUS_FAILED.to_string()
                            }
                        },
                        Err(_) => SQUARE_TASK_STATUS_FAILED.to_string(),
                    });
                    this.status = this
                        .status
                        .take()
                        .map(|status| format!("任务 #{task_id} {status}"));
                }
                cx.notify();
                this.load_tasks(cx);
            });
        });
    }

    fn start_scheduler_inner(&mut self, cx: &mut Context<Self>) {
        if self.scheduler_running {
            return;
        }

        self.scheduler_running = true;
        self.scheduler_next_run_at = Some(format_local_datetime(
            Local::now() + ChronoDuration::minutes(30),
        ));
        self.scheduler_enabled.store(true, Ordering::SeqCst);
        self.status = Some("调度器已启动，每 30 分钟检查一次 AI 分析和到期任务".to_string());
        let scheduler_enabled = self.scheduler_enabled.clone();
        let executor = cx.background_executor().clone();

        self._scheduler_task = cx.spawn(async move |this, cx| {
            while scheduler_enabled.load(Ordering::SeqCst) {
                executor.timer(Duration::from_secs(30 * 60)).await;
                if !scheduler_enabled.load(Ordering::SeqCst) {
                    break;
                }

                let result = cx
                    .background_spawn(async move {
                        binance_tools::square::run_square_automation_blocking()
                    })
                    .await;

                _ = this.update(cx, |this, cx| {
                    this.status = Some(match result {
                        Ok(summary) => format!("调度器：{}", format_automation_summary(&summary)),
                        Err(err) => err.to_string(),
                    });
                    this.scheduler_next_run_at = Some(format_local_datetime(
                        Local::now() + ChronoDuration::minutes(30),
                    ));
                    cx.notify();
                    this.load_tasks(cx);
                });
            }

            _ = this.update(cx, |this, cx| {
                this.scheduler_running = false;
                this.scheduler_next_run_at = None;
                cx.notify();
            });
        });
        cx.notify();
    }

    fn stop_scheduler_inner(&mut self, cx: &mut Context<Self>) {
        self.scheduler_enabled.store(false, Ordering::SeqCst);
        self.scheduler_running = false;
        self.scheduler_next_run_at = None;
        self.status = Some("调度器已停止".to_string());
        cx.notify();
    }

    fn toggle_scheduler_switch(&mut self, checked: bool, cx: &mut Context<Self>) {
        if checked {
            self.start_scheduler_inner(cx);
        } else {
            self.stop_scheduler_inner(cx);
        }
    }

    fn toggle_automation_panel(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.automation_collapsed = !self.automation_collapsed;
        cx.notify();
    }

    fn toggle_editor_panel(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.editor_collapsed = !self.editor_collapsed;
        cx.notify();
    }

    fn clear_status_message(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.status = None;
        cx.notify();
    }

    fn start_countdown_ticker(&mut self, cx: &mut Context<Self>) {
        self._countdown_task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
    }

    fn render_status_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .absolute()
            .left_0()
            .bottom(px(28.))
            .w(px(148.))
            .p_1()
            .bg(cx.theme().background)
            .border_1()
            .border_color(palette::border(cx.theme()))
            .rounded(px(6.))
            .shadow_md()
            .child(self.render_status_option(SQUARE_TASK_STATUS_DRAFT, "draft", cx))
            .child(self.render_status_option(SQUARE_TASK_STATUS_PENDING, "pending", cx))
            .child(self.render_status_option(SQUARE_TASK_STATUS_SENT, "sent", cx))
            .child(self.render_status_option(SQUARE_TASK_STATUS_SKIPPED, "skipped", cx))
            .child(self.render_status_option(SQUARE_TASK_STATUS_FAILED, "failed", cx))
            .into_any_element()
    }

    fn render_status_option(
        &self,
        status: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_task_status.as_deref() == Some(status);

        h_flex()
            .items_center()
            .justify_between()
            .h(px(26.))
            .px_2()
            .rounded(px(4.))
            .text_size(px(12.))
            .bg(if selected {
                cx.theme().muted.opacity(0.26)
            } else {
                cx.theme().transparent
            })
            .hover(|style| style.bg(palette::hover(cx.theme())))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_selected_task_status(status, label, cx);
                    cx.stop_propagation();
                }),
            )
            .child(div().child(label))
            .when(selected, |this| {
                this.child(
                    Icon::new(IconName::Check)
                        .size_3()
                        .text_color(cx.theme().success),
                )
            })
            .into_any_element()
    }
}

fn format_run_summary(summary: &SquareTaskRunSummary) -> String {
    let mut text = format!(
        "执行完成：处理 {}，成功 {}，跳过 {}，失败 {}",
        summary.processed, summary.success, summary.skipped, summary.failed
    );
    if summary.stopped {
        text.push_str("，已停止");
        if let Some(reason) = &summary.stop_reason {
            text.push_str("：");
            text.push_str(reason);
        }
    }
    text
}

fn format_ai_summary(summary: &SquareAiGenerationSummary) -> String {
    if summary.generated {
        let suffix = summary
            .skipped_reason
            .as_ref()
            .map(|reason| format!("，但需要人工检查：{reason}"))
            .unwrap_or_default();
        return format!(
            "AI 分析已写入任务表：{}{}",
            summary.title.clone().unwrap_or_else(|| "-".to_string()),
            suffix
        );
    }
    let reason = summary
        .skipped_reason
        .clone()
        .unwrap_or_else(|| "AI 分析未生成任务".to_string());
    format!("AI 分析未写入任务表：{reason}")
}

fn format_automation_summary(summary: &SquareAutomationSummary) -> String {
    format!(
        "{}；{}",
        format_ai_summary(&summary.ai),
        format_run_summary(&summary.tasks)
    )
}

impl Render for SquareTasksPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let task_count = self.table.read(cx).delegate().tasks.len();
        let selected_text = self
            .selected_task_id
            .map(|id| format!("已选中任务 #{id}"))
            .unwrap_or_else(|| "未选中任务".to_string());
        let selected_status = self
            .selected_task_status
            .as_deref()
            .unwrap_or("未选择")
            .to_string();
        let has_selected_task = self.selected_task_id.is_some();
        let delete_confirming = self.pending_delete_task_id == self.selected_task_id;
        let next_run_text =
            format_ai_next_run_countdown(self.ai_analysis_enabled, self.ai_next_run_at.as_deref());
        let scheduler_next_run_text = format_scheduler_countdown(
            self.scheduler_running,
            self.scheduler_next_run_at.as_deref(),
        );

        v_flex()
            .size_full()
            .gap_2()
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .when(self.automation_collapsed, |this| this.py_2())
                    .rounded(px(8.))
                    .bg(palette::surface_strong(cx.theme()))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_start()
                            .gap_4()
                            .flex_wrap()
                            .child(
                                v_flex()
                                    .gap_2()
                                    .min_w(px(320.))
                                    .flex_1()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_size(px(16.))
                                                    .font_semibold()
                                                    .child("币安广场任务"),
                                            )
                                            .child(
                                                Button::new("toggle-square-automation-panel")
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(
                                                        Icon::new(if self.automation_collapsed {
                                                            IconName::Maximize
                                                        } else {
                                                            IconName::Minimize
                                                        })
                                                        .size_4(),
                                                    )
                                                    .on_click(cx.listener(
                                                        Self::toggle_automation_panel,
                                                    )),
                                            ),
                                    )
                                    .when(!self.automation_collapsed, |this| this.child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(format!(
                                                "AI 生成草稿需人工确认后才会发送；发送器每 30 分钟检查 pending 任务。当前 {} 条任务。",
                                                task_count
                                            )),
                                    ))
                                    .when(!self.automation_collapsed, |this| this.child(
                                        v_flex()
                                            .gap_1()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .w(px(72.))
                                                            .font_medium()
                                                            .child("AI 下次分析"),
                                                    )
                                                    .child(div().child(next_run_text)),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .w(px(72.))
                                                            .font_medium()
                                                            .child("发送调度"),
                                                    )
                                                    .child(div().child(scheduler_next_run_text)),
                                            ),
                                    )),
                            )
                            .when(!self.automation_collapsed, |this| {
                                this.child(
                                    v_flex()
                                    .gap_2()
                                    .p_2()
                                    .rounded(px(6.))
                                    .bg(palette::surface(cx.theme()))
                                    .border_1()
                                    .border_color(palette::border(cx.theme()))
                                    .min_w(px(360.))
                                    .max_w(px(560.))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                v_flex()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_medium()
                                                            .child("AI 草稿"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(palette::muted(cx.theme()))
                                                            .child("按计划生成草稿，不会直接发送"),
                                                    ),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        Switch::new("square-ai-analysis-switch")
                                                            .checked(self.ai_analysis_enabled)
                                                            .small()
                                                            .tooltip(
                                                                "开启后按 AI 下次分析时间生成草稿任务",
                                                            )
                                                            .on_click(cx.listener(
                                                                |this, checked, _, cx| {
                                                                    if *checked
                                                                        != this.ai_analysis_enabled
                                                                    {
                                                                        this.toggle_ai_analysis_from_switch(
                                                                            *checked, cx,
                                                                        );
                                                                    }
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new("run-square-ai-analysis")
                                                            .outline()
                                                            .xsmall()
                                                            .label(if self.ai_generating {
                                                                "生成中"
                                                            } else {
                                                                "立即生成"
                                                            })
                                                            .loading(self.ai_generating)
                                                            .disabled(self.ai_generating)
                                                            .on_click(cx.listener(
                                                                Self::run_ai_analysis_now,
                                                            )),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                v_flex()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_medium()
                                                            .child("发送调度"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(palette::muted(cx.theme()))
                                                            .child("只发送已确认且到期的 pending 任务"),
                                                    ),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        Switch::new("square-scheduler-switch")
                                                            .checked(self.scheduler_running)
                                                            .small()
                                                            .tooltip(
                                                                "开启后每 30 分钟检查到期 pending 任务",
                                                            )
                                                            .on_click(cx.listener(
                                                                |this, checked, _, cx| {
                                                                    this.toggle_scheduler_switch(
                                                                        *checked, cx,
                                                                    );
                                                                },
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new("run-square-due-tasks")
                                                            .outline()
                                                            .xsmall()
                                                            .label("发送到期")
                                                            .on_click(
                                                                cx.listener(Self::run_due_tasks),
                                                            ),
                                                    ),
                                            ),
                                    ),
                                )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .when(self.editor_collapsed, |this| this.py_2())
                    .rounded(px(8.))
                    .bg(palette::surface(cx.theme()))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_semibold()
                                            .child("任务编辑"),
                                    )
                                    .child(
                                        Button::new("toggle-square-editor-panel")
                                            .ghost()
                                            .xsmall()
                                            .icon(
                                                Icon::new(if self.editor_collapsed {
                                                    IconName::Maximize
                                                } else {
                                                    IconName::Minimize
                                                })
                                                .size_4(),
                                            )
                                            .on_click(cx.listener(Self::toggle_editor_panel)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(cx.theme()))
                                    .child(selected_text),
                            ),
                    )
                    .when(!self.editor_collapsed, |this| this
                    .child(Input::new(&self.message_input).h(px(94.)))
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_start()
                                    .flex_wrap()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .w(px(250.))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(palette::muted(cx.theme()))
                                                    .child("任务名称"),
                                            )
                                            .child(Input::new(&self.name_input)),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .w(px(380.))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(palette::muted(cx.theme()))
                                                    .child("预计发送时间"),
                                            )
                                            .child(Input::new(&self.scheduled_at_input))
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .items_center()
                                                    .flex_wrap()
                                                    .child(
                                                        Button::new("schedule-now")
                                                            .outline()
                                                            .xsmall()
                                                            .label("现在")
                                                            .on_click(cx.listener(
                                                                Self::set_scheduled_now,
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new("schedule-after-30m")
                                                            .outline()
                                                            .xsmall()
                                                            .label("+30 分钟")
                                                            .on_click(cx.listener(
                                                                Self::set_scheduled_after_30m,
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new("schedule-after-1h")
                                                            .outline()
                                                            .xsmall()
                                                            .label("+1 小时")
                                                            .on_click(cx.listener(
                                                                Self::set_scheduled_after_1h,
                                                            )),
                                                    )
                                                    .child(
                                                        Button::new("schedule-tomorrow-morning")
                                                            .outline()
                                                            .xsmall()
                                                            .label("明天 09:00")
                                                            .on_click(cx.listener(
                                                                Self::set_scheduled_tomorrow_morning,
                                                            )),
                                                    ),
                                            )
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .flex_wrap()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(palette::border(cx.theme()))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child(format!("状态：{selected_status}")),
                                    )
                                    .child(
                                        div()
                                            .relative()
                                            .child(
                                                Button::new("square-task-status-picker")
                                                    .outline()
                                                    .xsmall()
                                                    .disabled(!has_selected_task)
                                                    .on_click(cx.listener(Self::toggle_status_picker))
                                                    .child(
                                                        h_flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .child("修改状态")
                                                            .child(
                                                                Icon::new(IconName::ChevronDown)
                                                                    .size_3()
                                                                    .text_color(palette::muted(
                                                                        cx.theme(),
                                                                    )),
                                                            ),
                                                    ),
                                            )
                                            .when(
                                                self.status_picker_open && has_selected_task,
                                                |this| this.child(self.render_status_picker(cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("send-square-now")
                                            .primary()
                                            .xsmall()
                                            .label("立即发送")
                                            .on_click(cx.listener(Self::send_now)),
                                    )
                                    .child(
                                        Button::new("save-square-task")
                                            .outline()
                                            .xsmall()
                                            .label("新增任务")
                                            .disabled(has_selected_task)
                                            .on_click(cx.listener(Self::save_task)),
                                    )
                                    .child(
                                        Button::new("update-square-task")
                                            .outline()
                                            .xsmall()
                                            .label("保存修改")
                                            .disabled(!has_selected_task)
                                            .on_click(cx.listener(Self::update_selected_task)),
                                    )
                                    .child(
                                        Button::new("clear-square-task-selection")
                                            .outline()
                                            .xsmall()
                                            .label("取消选择")
                                            .disabled(!has_selected_task)
                                            .on_click(
                                                cx.listener(Self::clear_selected_task_from_click),
                                            ),
                                    )
                                    .child(
                                        Button::new("delete-square-task")
                                            .outline()
                                            .xsmall()
                                            .label(if delete_confirming {
                                                "确认删除"
                                            } else {
                                                "删除"
                                            })
                                            .disabled(!has_selected_task)
                                            .on_click(cx.listener(Self::delete_selected_task)),
                                    ),
                            ),
                    )),
            )
            .when_some(self.status.clone(), |this, status| {
                let is_error = is_error_status(&status);
                this.child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .p_3()
                        .rounded(px(8.))
                        .bg(if is_error {
                            square_error_background()
                        } else {
                            palette::surface_strong(cx.theme())
                        })
                        .border_1()
                        .border_color(if is_error {
                            square_error_border()
                        } else {
                            palette::border(cx.theme())
                        })
                        .text_size(px(12.))
                        .text_color(if is_error {
                            square_error_text()
                        } else {
                            palette::text(cx.theme())
                        })
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .line_height(px(18.))
                                .child(status),
                        )
                        .child(
                            Button::new("clear-square-status")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::Delete).size_3())
                                .on_click(cx.listener(Self::clear_status_message)),
                        ),
                )
            })
            .child(
                v_flex()
                    .gap_2()
                    .flex_1()
                    .h_full()
                    .min_h(px(300.))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_semibold()
                                            .child("任务表格"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(cx.theme()))
                                            .child("选中行可回填到上方编辑"),
                                    ),
                            )
                            .child(
                                Button::new("refresh-square-tasks")
                                    .outline()
                                    .xsmall()
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, _, cx| this.load_tasks(cx))),
                            ),
                    )
                    .child(
                        div().flex_1().size_full().overflow_hidden().child(
                            DataTable::new(&self.table)
                                .stripe(true)
                                .bordered(true)
                                .scrollbar_visible(true, true),
                        ),
                    ),
            )
    }
}

#[derive(Clone)]
struct SquareTasksTableDelegate {
    columns: Vec<Column>,
    tasks: Vec<BinanceSquareTask>,
    loading: bool,
}

impl Default for SquareTasksTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("id", "ID")
                    .width(px(58.))
                    .fixed_left()
                    .sortable(),
                Column::new("title", "标题").width(px(90.)).sortable(),
                Column::new("source_type", "来源")
                    .width(px(120.))
                    .sortable(),
                Column::new("message", "内容").width(px(360.)),
                Column::new("scheduled_at", "预计发送时间")
                    .width(px(150.))
                    .sortable(),
                Column::new("send_status", "状态").width(px(90.)).sortable(),
                Column::new("updated_at", "更新时间")
                    .width(px(150.))
                    .sortable(),
            ],
            tasks: Vec::new(),
            loading: false,
        }
    }
}

impl SquareTasksTableDelegate {
    fn set_tasks(&mut self, tasks: Vec<BinanceSquareTask>) {
        self.tasks = tasks;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.tasks.clear();
        self.loading = false;
    }

    fn remove_task(&mut self, task_id: i64) {
        self.tasks.retain(|task| task.id != task_id);
        self.loading = false;
    }

    fn task_at(&self, row_ix: usize) -> Option<&BinanceSquareTask> {
        self.tasks.get(row_ix)
    }

    fn cell(value: impl Into<SharedString>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px_1()
            .text_size(px(11.))
            .child(value.into())
            .into_any_element()
    }
}

fn non_empty(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    }
}

fn extract_task_title(message: &str) -> Option<String> {
    message
        .split_whitespace()
        .next()
        .filter(|value| value.starts_with('$') && value.len() > 1)
        .map(str::to_string)
}

fn format_local_datetime(value: chrono::DateTime<Local>) -> String {
    value.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn parse_local_datetime(value: &str) -> Option<chrono::DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M:%S").ok()?;
    Local.from_local_datetime(&naive).single()
}

fn format_countdown_seconds(total_seconds: i64) -> String {
    let total_seconds = total_seconds.max(0);
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if days > 0 {
        format!("{days}天 {hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

fn format_ai_next_run_countdown(enabled: bool, next_run_at: Option<&str>) -> String {
    if !enabled {
        return "未开启".to_string();
    }

    let Some(next_run_at) = next_run_at.filter(|value| !value.trim().is_empty()) else {
        return "计算中".to_string();
    };

    let Some(next_run_time) = parse_local_datetime(next_run_at) else {
        return next_run_at.to_string();
    };

    let remaining = next_run_time
        .signed_duration_since(Local::now())
        .num_seconds();
    if remaining <= 0 {
        format!("已到期，等待执行（{next_run_at}）")
    } else {
        format!(
            "剩余 {}（{next_run_at}）",
            format_countdown_seconds(remaining)
        )
    }
}

fn format_scheduler_countdown(running: bool, next_run_at: Option<&str>) -> String {
    if !running {
        return "未开启".to_string();
    }

    let Some(next_run_at) = next_run_at.filter(|value| !value.trim().is_empty()) else {
        return "计算中".to_string();
    };

    let Some(next_run_time) = parse_local_datetime(next_run_at) else {
        return next_run_at.to_string();
    };

    let remaining = next_run_time
        .signed_duration_since(Local::now())
        .num_seconds();
    if remaining <= 0 {
        format!("即将检查（{next_run_at}）")
    } else {
        format!(
            "剩余 {}（{next_run_at}）",
            format_countdown_seconds(remaining)
        )
    }
}

fn is_error_status(status: &str) -> bool {
    let status = status.to_ascii_lowercase();
    status.contains("失败")
        || status.contains("错误")
        || status.contains("超时")
        || status.contains("未授权")
        || status.contains("不可用")
        || status.contains("failed")
        || status.contains("error")
        || status.contains("forbidden")
        || status.contains("unauthorized")
}

fn square_error_background() -> Hsla {
    palette::error_background()
}

fn square_error_border() -> Hsla {
    palette::error_border()
}

fn square_error_text() -> Hsla {
    palette::error_text()
}

impl TableDelegate for SquareTasksTableDelegate {
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
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(task) = self.tasks.get(row_ix) else {
            return Self::cell("");
        };
        let key = self.columns[col_ix].key.as_ref();

        match key {
            "id" => Self::cell(task.id.to_string()),
            "title" => Self::cell(task.title.clone().unwrap_or_else(|| "-".to_string())),
            "source_type" => Self::cell(non_empty(&task.source_type)),
            "message" => Self::cell(non_empty(&task.message)),
            "scheduled_at" => Self::cell(non_empty(&task.scheduled_at)),
            "send_status" => Self::cell(non_empty(&task.send_status)),
            "updated_at" => Self::cell(task.updated_at.clone()),
            _ => Self::cell(""),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let descending = matches!(sort, ColumnSort::Descending);
        let key = self.columns[col_ix].key.to_string();

        self.tasks.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "id" => a.id.cmp(&b.id),
                "title" => a.title.cmp(&b.title),
                "source_type" => a.source_type.cmp(&b.source_type),
                "scheduled_at" => a.scheduled_at.cmp(&b.scheduled_at),
                "send_status" => a.send_status.cmp(&b.send_status),
                "updated_at" => a.updated_at.cmp(&b.updated_at),
                _ => a.id.cmp(&b.id),
            };
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }
}

pub struct SquareSendLogsPage {
    table: Entity<TableState<SquareSendLogsTableDelegate>>,
    selected_log_id: Option<i64>,
    pending_delete_log_id: Option<i64>,
    status: Option<String>,
    _task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SquareSendLogsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(SquareSendLogsTableDelegate::default(), window, cx)
                .col_movable(false)
                .row_selectable(true)
        });
        let _subscriptions = vec![cx.subscribe_in(&table, window, Self::on_table_event)];

        let mut this = Self {
            table,
            selected_log_id: None,
            pending_delete_log_id: None,
            status: None,
            _task: Task::ready(()),
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.status = Some("正在加载发送日志".to_string());
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(true);
            table.refresh(cx);
        });

        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    binance_tools::db::square::list_square_send_logs_blocking(1000)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(logs) => {
                        let count = logs.len();
                        this.status = Some(format!("当前 {} 条发送日志", count));
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_logs(logs);
                            table.refresh(cx);
                        });
                    }
                    Err(err) => {
                        this.status = Some(err.to_string());
                        this.table.update(cx, |table, cx| {
                            table.delegate_mut().set_error();
                            table.refresh(cx);
                        });
                    }
                }
                cx.notify();
            });
        });
    }

    fn on_table_event(
        &mut self,
        _: &Entity<TableState<SquareSendLogsTableDelegate>>,
        event: &TableEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TableEvent::SelectRow(row_ix) => {
                let selected_col = self.table.read(cx).selected_col();
                self.selected_log_id = self
                    .table
                    .read(cx)
                    .delegate()
                    .log_at(*row_ix)
                    .map(|log| log.id);
                self.pending_delete_log_id = None;
                self.status = Some(match selected_col {
                    Some(col_ix) => format!(
                        "已选中日志 #{}，第 {} 列",
                        self.selected_log_id.unwrap_or_default(),
                        col_ix + 1
                    ),
                    None => format!("已选中日志 #{}", self.selected_log_id.unwrap_or_default()),
                });
                cx.notify();
            }
            TableEvent::SelectColumn(col_ix) => {
                let selected_row = self.table.read(cx).selected_row();
                self.status = Some(match selected_row {
                    Some(row_ix) => format!("已选中第 {} 行，第 {} 列", row_ix + 1, col_ix + 1),
                    None => format!("已选中第 {} 列", col_ix + 1),
                });
                cx.notify();
            }
            _ => {}
        }
    }

    fn copy_selected(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let table = self.table.read(cx);
        let delegate = table.delegate();
        let selected_row = table.selected_row();
        let selected_col = table.selected_col();

        let value = match (selected_row, selected_col) {
            (Some(row_ix), Some(col_ix)) => delegate.value_at(row_ix, col_ix),
            (Some(row_ix), None) => delegate.row_values(row_ix).join("\t"),
            (None, Some(col_ix)) => delegate.column_values(col_ix).join("\n"),
            (None, None) => String::new(),
        };

        if value.is_empty() {
            self.status = Some("请先选中一行或一列".to_string());
        } else {
            cx.write_to_clipboard(ClipboardItem::new_string(value));
            self.status = Some(match (selected_row, selected_col) {
                (Some(_), Some(_)) => "已复制选中字段".to_string(),
                (Some(_), None) => "已复制选中行".to_string(),
                (None, Some(_)) => "已复制选中列".to_string(),
                (None, None) => "请先选中一行或一列".to_string(),
            });
        }
        cx.notify();
    }

    fn delete_selected_log(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(log_id) = self.selected_log_id else {
            self.status = Some("请先选中一条发送日志".to_string());
            cx.notify();
            return;
        };

        if self.pending_delete_log_id != Some(log_id) {
            self.pending_delete_log_id = Some(log_id);
            self.status = Some(format!("再次点击“确认删除”才会删除日志 #{log_id}"));
            cx.notify();
            return;
        }

        match binance_tools::db::square::delete_square_send_log_blocking(log_id) {
            Ok(()) => {
                self.selected_log_id = None;
                self.pending_delete_log_id = None;
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().remove_log(log_id);
                    table.refresh(cx);
                });
                self.status = Some(format!("日志 #{log_id} 已删除"));
            }
            Err(err) => {
                self.status = Some(err.to_string());
            }
        }
        cx.notify();
    }
}

impl Render for SquareSendLogsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let delete_confirming = self.pending_delete_log_id == self.selected_log_id;
        let has_selected_log = self.selected_log_id.is_some();

        v_flex()
            .gap_3()
            .size_full()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .p_4()
                    .rounded(px(8.))
                    .bg(palette::surface_strong(cx.theme()))
                    .border_1()
                    .border_color(palette::border(cx.theme()))
                    .child(
                        v_flex()
                            .gap_1()
                            .flex_1()
                            .min_w(px(240.))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_semibold()
                                    .child("发送消息日志"),
                            )
                            .when_some(self.status.clone(), |this, status| {
                                this.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette::muted(cx.theme()))
                                        .child(status),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .justify_end()
                            .flex_wrap()
                            .child(
                                Button::new("copy-selected-square-log-cell")
                                    .outline()
                                    .xsmall()
                                    .label("复制选中")
                                    .on_click(cx.listener(Self::copy_selected)),
                            )
                            .child(
                                Button::new("delete-selected-square-log")
                                    .outline()
                                    .xsmall()
                                    .label(if delete_confirming {
                                        "确认删除"
                                    } else {
                                        "删除"
                                    })
                                    .disabled(!has_selected_log)
                                    .on_click(cx.listener(Self::delete_selected_log)),
                            )
                            .child(
                                Button::new("refresh-square-send-logs")
                                    .primary()
                                    .xsmall()
                                    .label("刷新日志")
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    ),
            )
            .child(
                v_flex().flex_1().h_full().min_h(px(420.)).w_full().child(
                    div().flex_1().size_full().overflow_hidden().child(
                        DataTable::new(&self.table)
                            .stripe(true)
                            .bordered(true)
                            .scrollbar_visible(true, true),
                    ),
                ),
            )
    }
}

#[derive(Clone)]
struct SquareSendLogsTableDelegate {
    columns: Vec<Column>,
    logs: Vec<BinanceSquareSendLog>,
    loading: bool,
}

impl Default for SquareSendLogsTableDelegate {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new("id", "ID")
                    .width(px(58.))
                    .fixed_left()
                    .sortable(),
                Column::new("task_id", "Task").width(px(70.)).sortable(),
                Column::new("status", "Status").width(px(96.)).sortable(),
                Column::new("response_code", "Code")
                    .width(px(84.))
                    .sortable(),
                Column::new("retry_count", "Retry")
                    .width(px(68.))
                    .sortable(),
                Column::new("sent_at", "Sent At").width(px(150.)).sortable(),
                Column::new("message_digest", "Message").width(px(300.)),
                Column::new("error_message", "Error").width(px(240.)),
            ],
            logs: Vec::new(),
            loading: false,
        }
    }
}

impl SquareSendLogsTableDelegate {
    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    fn set_logs(&mut self, logs: Vec<BinanceSquareSendLog>) {
        self.logs = logs;
        self.loading = false;
    }

    fn set_error(&mut self) {
        self.logs.clear();
        self.loading = false;
    }

    fn remove_log(&mut self, log_id: i64) {
        self.logs.retain(|log| log.id != log_id);
        self.loading = false;
    }

    fn log_at(&self, row_ix: usize) -> Option<&BinanceSquareSendLog> {
        self.logs.get(row_ix)
    }

    fn cell(value: impl Into<SharedString>) -> AnyElement {
        let value = value.into();
        let copy_value = value.to_string();
        div()
            .size_full()
            .flex()
            .items_center()
            .px_1()
            .text_size(px(11.))
            .capture_any_mouse_down(move |event, _, cx| {
                if event.button == MouseButton::Left && event.click_count == 2 {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_value.clone()));
                    cx.stop_propagation();
                }
            })
            .child(value)
            .into_any_element()
    }

    fn value_at(&self, row_ix: usize, col_ix: usize) -> String {
        let Some(log) = self.logs.get(row_ix) else {
            return String::new();
        };
        let Some(column) = self.columns.get(col_ix) else {
            return String::new();
        };

        match column.key.as_ref() {
            "id" => log.id.to_string(),
            "task_id" => log
                .task_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            "status" => log.status.clone(),
            "response_code" => log.response_code.clone().unwrap_or_default(),
            "retry_count" => log.retry_count.to_string(),
            "sent_at" => log.sent_at.clone(),
            "message_digest" => log.message_digest.clone(),
            "error_message" => log.error_message.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn row_values(&self, row_ix: usize) -> Vec<String> {
        (0..self.columns.len())
            .map(|col_ix| self.value_at(row_ix, col_ix))
            .collect()
    }

    fn column_values(&self, col_ix: usize) -> Vec<String> {
        (0..self.logs.len())
            .map(|row_ix| self.value_at(row_ix, col_ix))
            .collect()
    }
}

impl TableDelegate for SquareSendLogsTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.logs.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let value = self.value_at(row_ix, col_ix);
        Self::cell(value)
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let descending = matches!(sort, ColumnSort::Descending);
        let key = self.columns[col_ix].key.to_string();

        self.logs.sort_by(|a, b| {
            let ordering = match key.as_str() {
                "id" => a.id.cmp(&b.id),
                "task_id" => a.task_id.cmp(&b.task_id),
                "status" => a.status.cmp(&b.status),
                "response_code" => a.response_code.cmp(&b.response_code),
                "retry_count" => a.retry_count.cmp(&b.retry_count),
                "sent_at" => a.sent_at.cmp(&b.sent_at),
                _ => a.id.cmp(&b.id),
            };
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }
}

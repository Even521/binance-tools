mod automation;
mod auto_comment;
mod auto_post;
mod auto_reply;
mod core;
mod crop;
mod full_auto;
mod ocr;
mod ocr_status;
mod option;
mod post_history;
mod reply_list;
mod reply_filter;
mod own_post_reply;
mod run_log;
mod vision;
use crate::ui::palette;

use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::{
    backtrace::Backtrace,
    path::PathBuf,
    time::Duration,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};


#[derive(Clone, Copy)]
enum TaskSetting {
    AutoReply,
    AutoPost,
    AutoComment,
    OwnPostReply,
}

pub struct XAssistantPage {
    replying: bool,
    auto_send: bool,
    vision_model: String,
    reply_rules: Entity<InputState>,
    _reply_rules_subscription: Subscription,
    cancel_reply: Arc<AtomicBool>,
    // 0=未标记，1=用户点击停止，2=页面销毁；用于区分任务取消来源。
    cancel_reason: Arc<AtomicU8>,
    extracted_fields: String,
    screenshots: Vec<PathBuf>,
    selected_screenshot: usize,
    image_preview_open: bool,
    show_reply_rules: bool,
    status: Option<String>,
    error: Option<String>,
    _reply_task: Task<()>,
    show_reply_list: bool,
    show_post_history: bool,
    show_task_settings: bool,
    show_auto_post: bool,
    reply_list: Option<Entity<reply_list::ReplyListPage>>,
    post_history: Option<Entity<post_history::PostHistoryPage>>,
    task_settings: binance_tools::db::x_posting::XAssistantTaskSettings,
    render_diagnostic_logged: bool,
    show_run_status: bool,
    log_tail: Option<run_log::LogTail>,
    _log_task: Task<()>,
}

impl Drop for XAssistantPage {
    fn drop(&mut self) {
        // 页面关闭/销毁时通知后台阻塞任务尽快退出，避免工作线程继续运行。
        let token_id = Arc::as_ptr(&self.cancel_reply) as usize;
        let reason_set = self.cancel_reason.compare_exchange(
            0, 2, Ordering::AcqRel, Ordering::Acquire
        ).is_ok();
        let cancel_reason = self.cancel_reason.load(Ordering::Acquire);
        let was_cancelled = self.cancel_reply.swap(true, Ordering::AcqRel);

        if self.replying {
            app_log::x_warn!(
                "X 取消来源诊断：页面 Drop，token=0x{:x}，此前已取消={}，本次新发送={}，本次设置来源={}，最终来源={}，Drop 调用栈：{}",
                token_id,
                was_cancelled,
                !was_cancelled,
                reason_set,
                cancel_reason,
                Backtrace::force_capture()
            );
        } else {
            app_log::x_info!(
                "X 助手页面 Drop：token=0x{:x}，replying=false，此前已取消={}，本次新发送={}，最终来源={}",
                token_id,
                was_cancelled,
                !was_cancelled,
                cancel_reason
            );
        }
    }
}

impl XAssistantPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rules_value = load_reply_rules();
        let reply_rules = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("输入回复规则，例如：简短、自然地回应帖子，不要照抄正文。")
                .default_value(rules_value)
        });
        let reply_rules_subscription =
            cx.subscribe_in(&reply_rules, window, Self::on_reply_rules_input);
        let task_settings = binance_tools::db::x_posting::load_task_settings_blocking()
            .unwrap_or_default();

        Self {
            replying: false,
            auto_send: true,
            vision_model: vision::DEFAULT_VISION_MODEL.to_string(),
            reply_rules,
            _reply_rules_subscription: reply_rules_subscription,
            cancel_reply: Arc::new(AtomicBool::new(false)),
            cancel_reason: Arc::new(AtomicU8::new(0)),
            extracted_fields: String::new(),
            screenshots: Vec::new(),
            selected_screenshot: 0,
            image_preview_open: false,
            show_reply_rules: false,
            status: None,
            error: None,
            _reply_task: Task::ready(()),
            show_reply_list: false,
            show_post_history: false,
            show_task_settings: false,
            show_auto_post: false,
            reply_list: None,
            post_history: None,
            task_settings,
            render_diagnostic_logged: false,
            show_run_status: true,
            log_tail: None,
            _log_task: Task::ready(()),
        }
    }

    fn start_all_automation(&mut self, cx: &mut Context<Self>) {
        if self.replying {
            return;
        }
        if self.task_settings.auto_post_enabled {
            // 自动回复与自动发帖同时启用时，必须先完成自动回复首轮。
            // 若自动发送被关闭，不允许静默退化为“启动后立即发帖”。
            if self.task_settings.auto_reply_enabled && !self.auto_send {
                self.status = None;
                self.error = Some(
                    "自动回复调度已开启，但「自动发送」处于关闭状态。为避免启动后直接发帖，请先开启自动发送，再开始自动化。"
                        .to_string(),
                );
                cx.notify();
                return;
            }
            self.start_auto_post(cx);
            return;
        }
        if !self.task_settings.auto_reply_enabled {
            self.error = Some("没有可启动的自动化任务。请先在「任务设置」中开启自动回复或自动发帖。".to_string());
            self.status = None;
            cx.notify();
            return;
        }
        self.start_visual_reply(false, true, true, cx);
    }

    fn start_auto_post(&mut self, cx: &mut Context<Self>) {
        if self.replying || !self.task_settings.auto_post_enabled {
            return;
        }
        self.replying = true;
        self.start_log_monitor(cx);
        self.error = None;
        self.cancel_reply = Arc::new(AtomicBool::new(false));
        self.cancel_reason = Arc::new(AtomicU8::new(0));
        self.status = Some(if self.task_settings.auto_reply_enabled && self.auto_send {
            "自动发帖与自动回复已启动；两类任务通过同一浏览器锁串行操作。点击停止可结束。".to_string()
        } else if self.task_settings.auto_reply_enabled {
            "自动发帖已启动；自动回复开关虽已开启，但自动发送关闭，本轮只执行发帖。".to_string()
        } else {
            "自动发帖已启动；点击停止可结束。".to_string()
        });
        cx.notify();

        let cancel = Arc::clone(&self.cancel_reply);
        let news_filter = auto_post::content::NewsFilter::from_setting(
            &self.task_settings.auto_post_news_type,
        );
        let reply_options = if self.task_settings.auto_reply_enabled && self.auto_send {
            Some(vision::VisionReplyOptions {
                api_url: vision::DEFAULT_OLLAMA_URL.to_string(),
                model: self.vision_model.clone(),
                auto_send: true,
                hover_only: false,
                stage2_ocr: true,
                reply_rules: self.reply_rules.read(cx).text().to_string(),
            })
        } else {
            None
        };
        self._reply_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(options) = reply_options {
                        auto_post::executor::run_with_auto_reply(Arc::clone(&cancel), options, news_filter)
                    } else {
                        auto_post::executor::run_loop(&cancel, news_filter)
                    }
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.replying = false;
                match result {
                    Ok(count) => {
                        this.error = None;
                        this.status = Some(format!("自动发帖已停止，本轮确认发布 {count} 条。"));
                    }
                    Err(error) => {
                        this.status = None;
                        this.error = Some(format!("自动发帖停止：{error:#}"));
                    }
                }
                cx.notify();
            });
        });
    }

    fn start_log_monitor(&mut self, cx: &mut Context<Self>) {
        self.log_tail = Some(run_log::LogTail::new());
        self._log_task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(800)).await;
                let keep_running = this.update(cx, |this, cx| {
                    if !this.replying {
                        return false;
                    }
                    if this.log_tail.as_mut().is_some_and(run_log::LogTail::poll) {
                        cx.notify();
                    }
                    true
                }).unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        });
    }

    fn set_auto_post_news_type(&mut self, news_type: &str, cx: &mut Context<Self>) {
        self.task_settings.auto_post_news_type = match news_type {
            "crypto" => "crypto".to_string(),
            "entertainment" => "entertainment".to_string(),
            _ => "all".to_string(),
        };
        self.persist_task_settings(cx);
    }

    fn start_test_auto_post(&mut self, cx: &mut Context<Self>) {
        app_log::x_info!("X 自动发帖测试：收到测试按钮点击，当前 replying={}", self.replying);
        if self.replying {
            app_log::x_warn!("X 自动发帖测试：已有自动化任务运行，忽略本次点击");
            return;
        }
        self.replying = true;
        self.start_log_monitor(cx);
        self.error = None;
        self.cancel_reply = Arc::new(AtomicBool::new(false));
        self.cancel_reason = Arc::new(AtomicU8::new(0));
        self.status = Some(
            "测试发帖已启动：只选取一条所选类型新闻，完成真实发布与结果核验；不会启动连续发布。".to_string(),
        );
        cx.notify();

        let cancel = Arc::clone(&self.cancel_reply);
        let filter = auto_post::content::NewsFilter::from_setting(
            &self.task_settings.auto_post_news_type,
        );
        app_log::x_info!("X 自动发帖测试：任务已提交后台，新闻类型={:?}", filter);
        self._reply_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    app_log::x_info!("X 自动发帖测试：后台执行开始；先读取新闻并生成文案，准备完成后才会打开浏览器");
                    auto_post::executor::run_test_once(&cancel, filter)
                })
                .await;
            app_log::x_info!("X 自动发帖测试：后台执行结束，准备更新界面结果");
            _ = this.update(cx, |this, cx| {
                this.replying = false;
                match result {
                    Ok(result) if result.outcome == auto_post::submit::SubmitOutcome::Published => {
                        this.error = None;
                        this.status = Some(format!(
                            "测试发帖成功：post_id={}。已完成提交和页面结果核验。",
                            result.post_id
                        ));
                    }
                    Ok(result) => {
                        this.status = None;
                        this.error = Some(format!(
                            "测试发帖结果不确定：post_id={}。请先人工检查 X 页面，禁止直接重试。{}",
                            result.post_id, result.message
                        ));
                    }
                    Err(error) => {
                        this.status = None;
                        this.error = Some(format!("测试发帖失败：{error:#}"));
                    }
                }
                cx.notify();
            });
        });
    }

    fn start_visual_reply(
        &mut self,
        hover_only: bool,
        stage2_ocr: bool,
        full_auto: bool,
        cx: &mut Context<Self>,
    ) {
        if self.replying {
            return;
        }
        if full_auto && !self.auto_send {
            self.error = Some("全自动回复需要先开启‘自动发送’。".to_string());
            cx.notify();
            return;
        }
        self.replying = true;
        self.start_log_monitor(cx);
        self.error = None;
        self.cancel_reply = Arc::new(AtomicBool::new(false));
        self.cancel_reason = Arc::new(AtomicU8::new(0));
        let token_id = Arc::as_ptr(&self.cancel_reply) as usize;
        app_log::x_info!(
            "X 视觉回复启动：创建独立取消令牌 token=0x{:x}，hover_only={}，stage2_ocr={}，full_auto={}",
            token_id,
            hover_only,
            stage2_ocr,
            full_auto
        );
        self.status = Some(if full_auto {
            "全自动循环已启动；每轮完成后随机距离滚动，异常时按安全流程恢复，点击停止可结束。"
                .to_string()
        } else if hover_only {
            "5 秒后开始识别并移到回复按钮。AI 识图约需 15~30 秒，请切到 X 浏览器窗口。".to_string()
        } else {
            "5 秒后开始自动回复流程。AI 识图约需 15~30 秒，请确保 X 浏览器处于前台。".to_string()
        });
        cx.notify();

        let auto_send = self.auto_send;
        let reply_rules = self.reply_rules.read(cx).text().to_string();
        let model = self.vision_model.clone();
        let cancel = Arc::clone(&self.cancel_reply);
        let cancel_for_ui = Arc::clone(&cancel);
        let cancel_reason_for_ui = Arc::clone(&self.cancel_reason);
        self._reply_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let options = vision::VisionReplyOptions {
                        api_url: vision::DEFAULT_OLLAMA_URL.to_string(),
                        model,
                        auto_send,
                        hover_only,
                        stage2_ocr,
                        reply_rules,
                    };
                    if full_auto {
                        // 全自动回复在每轮识图、滚动和恢复动作处分别取得浏览器锁，
                        // 让自动发帖任务可以在两轮回复之间安全使用同一浏览器。
                        full_auto::run(options, &cancel)
                    } else {
                        core::task_queue::run_browser_task(|| {
                            vision::run_visual_reply(options, &cancel)
                        })
                    }
                })
                .await;

            let was_cancelled = cancel_for_ui.load(Ordering::Acquire);
            let cancel_reason = cancel_reason_for_ui.load(Ordering::Acquire);
            let token_id = Arc::as_ptr(&cancel_for_ui) as usize;
            app_log::x_info!(
                "X 视觉回复：后台任务已结束，token=0x{:x}，取消标记={}，取消来源={}（0=未知，1=用户停止，2=页面销毁），准备回收 UI 状态",
                token_id,
                was_cancelled,
                cancel_reason
            );

            if let Err(update_error) = this.update(cx, |this, cx| {
                // 无论成功、失败还是手动取消，任务结束后都必须解除运行态。
                this.replying = false;
                match result {
                    Err(err)
                        if was_cancelled
                            && format!("{err:#}").contains("视觉回复流程已停止") =>
                    {
                        this.status = Some("任务已中断，后台任务已结束。".to_string());
                        this.error = None;
                    }
                    Err(err) => {
                        this.status = None;
                        this.error = Some(if was_cancelled {
                            format!("任务异常退出（同时收到停止信号）：{err:#}")
                        } else {
                            format!("{err:#}")
                        });
                    }
                    Ok(_reply) if was_cancelled => {
                        this.status = Some("任务已中断，后台任务已安全结束。".to_string());
                        this.error = None;
                    }
                    Ok(reply) => {
                            this.screenshots = reply.screenshots;
                            this.selected_screenshot = this.screenshots.len().saturating_sub(1);
                            this.extracted_fields = format!(
                                "发送者：{}\n账号：{}\n时间：{}\n\n帖子：\n{}\n\n回复：\n{}\n\n{}",
                                if reply.author_name.is_empty() {
                                    "未识别"
                                } else {
                                    &reply.author_name
                                },
                                if reply.account_handle.is_empty() {
                                    "未识别"
                                } else {
                                    &reply.account_handle
                                },
                                if reply.posted_at.is_empty() {
                                    "未识别"
                                } else {
                                    &reply.posted_at
                                },
                                reply.post_text,
                                reply.reply_text,
                                reply.summary
                            );
                            this.status = Some(if reply.completed {
                                format!("已完成。{}", reply.summary)
                            } else {
                                reply.summary
                            });
                        this.error = None;
                    }
                }
                cx.notify();
                app_log::x_info!(
                    "X 视觉回复：UI 状态已回收，replying={}",
                    this.replying
                );
            }) {
                app_log::x_error!(
                    "X 视觉回复：后台已结束，但 GPUI UI 状态更新失败：{}",
                    update_error
                );
            }
        });
    }

    fn toggle_task_setting(&mut self, setting: TaskSetting, cx: &mut Context<Self>) {
        match setting {
            TaskSetting::AutoReply => self.task_settings.auto_reply_enabled = !self.task_settings.auto_reply_enabled,
            TaskSetting::AutoPost => self.task_settings.auto_post_enabled = !self.task_settings.auto_post_enabled,
            TaskSetting::AutoComment => self.task_settings.auto_comment_enabled = !self.task_settings.auto_comment_enabled,
            TaskSetting::OwnPostReply => self.task_settings.own_post_reply_enabled = !self.task_settings.own_post_reply_enabled,
        }
        self.persist_task_settings(cx);
    }

    fn adjust_posts_per_hour(&mut self, delta: i8, cx: &mut Context<Self>) {
        let next = i16::from(self.task_settings.posts_per_hour) + i16::from(delta);
        self.task_settings.posts_per_hour = next.clamp(1, 10) as u8;
        self.persist_task_settings(cx);
    }

    fn persist_task_settings(&mut self, cx: &mut Context<Self>) {
        match binance_tools::db::x_posting::save_task_settings_blocking(&self.task_settings) {
            Ok(()) => {
                self.status = Some("X 自动化任务设置已保存到本地 SQLite。".to_string());
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!("保存 X 自动化任务设置失败：{error:#}"));
            }
        }
        cx.notify();
    }

    fn render_auto_reply_setting_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("x-task-auto-reply")
            .small()
            .outline()
            .label(if self.task_settings.auto_reply_enabled {
                "自动回复调度：开"
            } else {
                "自动回复调度：关"
            })
            .when(self.task_settings.auto_reply_enabled, |button| button.primary())
            .on_click(cx.listener(|this, _, _, cx| {
                app_log::x_info!("X 任务设置：点击自动回复开关");
                this.toggle_task_setting(TaskSetting::AutoReply, cx);
            }))
            .into_any_element()
    }

    fn render_auto_post_setting_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("x-task-auto-post")
            .small()
            .outline()
            .label(if self.task_settings.auto_post_enabled {
                "自动发帖：开"
            } else {
                "自动发帖：关"
            })
            .when(self.task_settings.auto_post_enabled, |button| button.primary())
            .on_click(cx.listener(|this, _, _, cx| {
                app_log::x_info!("X 任务设置：点击自动发帖开关");
                this.toggle_task_setting(TaskSetting::AutoPost, cx);
            }))
            .into_any_element()
    }

    fn render_auto_comment_setting_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("x-task-auto-comment")
            .small()
            .outline()
            .label(if self.task_settings.auto_comment_enabled {
                "发布后自动评论：开"
            } else {
                "发布后自动评论：关"
            })
            .when(self.task_settings.auto_comment_enabled, |button| button.primary())
            .on_click(cx.listener(|this, _, _, cx| {
                app_log::x_info!("X 任务设置：点击自动评论开关");
                this.toggle_task_setting(TaskSetting::AutoComment, cx);
            }))
            .into_any_element()
    }

    fn render_own_post_reply_setting_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("x-task-own-post-reply")
            .small()
            .outline()
            .label(if self.task_settings.own_post_reply_enabled {
                "回复自己帖子评论：开（预留）"
            } else {
                "回复自己帖子评论：关（预留）"
            })
            .when(self.task_settings.own_post_reply_enabled, |button| button.primary())
            .on_click(cx.listener(|this, _, _, cx| {
                app_log::x_info!("X 任务设置：点击回复自己帖子评论开关");
                this.toggle_task_setting(TaskSetting::OwnPostReply, cx);
            }))
            .into_any_element()
    }

    fn render_post_rate_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let minus_button = Button::new("x-task-post-rate-minus")
            .small()
            .outline()
            .label("−")
            .on_click(cx.listener(|this, _, _, cx| {
                app_log::x_info!("X 任务设置：每小时发帖数量减一");
                this.adjust_posts_per_hour(-1, cx);
            }))
            .into_any_element();

        let plus_button = Button::new("x-task-post-rate-plus")
            .small()
            .outline()
            .label("+")
            .on_click(cx.listener(|this, _, _, cx| {
                app_log::x_info!("X 任务设置：每小时发帖数量加一");
                this.adjust_posts_per_hour(1, cx);
            }))
            .into_any_element();

        h_flex()
            .items_center()
            .gap_2()
            .child(div().text_size(px(12.)).child("每小时发布数量"))
            .child(minus_button)
            .child(
                div()
                    .text_size(px(14.))
                    .font_semibold()
                    .child(format!("{} 条", self.task_settings.posts_per_hour)),
            )
            .child(plus_button)
            .into_any_element()
    }

    fn render_task_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        app_log::x_info!("X 任务设置：开始渲染设置面板");

        // 先创建需要可变借用 Context 的按钮，再读取主题，避免 cx 的借用冲突。
        let auto_reply_button = self.render_auto_reply_setting_button(cx);
        let auto_post_button = self.render_auto_post_setting_button(cx);
        let auto_comment_button = self.render_auto_comment_setting_button(cx);
        let own_post_reply_button = self.render_own_post_reply_setting_button(cx);
        let rate_controls = self.render_post_rate_controls(cx);

        let theme = cx.theme();
        let panel = v_flex()
            .gap_2()
            .rounded(px(10.))
            .border_1()
            .border_color(palette::border(theme))
            .bg(palette::surface(theme))
            .p_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_size(px(14.)).font_semibold().child("自动化任务开关"))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::muted(theme))
                            .child("开启后纳入任务调度；关闭后不会安排对应类型的任务。"),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(auto_reply_button)
                    .child(auto_post_button)
                    .child(auto_comment_button)
                    .child(own_post_reply_button),
            )
            .child(rate_controls)
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(palette::muted(theme))
                    .child("设置会保存到本地数据库。自动发帖按工作区选择的新闻类型筛选候选，并遵守每小时发布上限。"),
            )
            .into_any_element();

        app_log::x_info!("X 任务设置：设置面板构建完成");
        panel
    }

    fn open_reply_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = &self.reply_list {
            page.update(cx, |page, cx| page.reload(cx));
        } else {
            self.reply_list = Some(cx.new(|cx| reply_list::ReplyListPage::new(window, cx)));
        }
        self.show_run_status = false;
        self.show_reply_list = true;
        self.show_post_history = false;
        self.show_task_settings = false;
        self.show_auto_post = false;
        cx.notify();
    }

    fn show_reply_flow(&mut self, cx: &mut Context<Self>) {
        self.show_run_status = false;
        self.show_reply_list = false;
        self.show_post_history = false;
        self.show_task_settings = false;
        self.show_auto_post = false;
        self.error = None;
        cx.notify();
    }

    fn open_post_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = &self.post_history {
            page.update(cx, |page, cx| page.reload(cx));
        } else {
            self.post_history = Some(cx.new(|cx| post_history::PostHistoryPage::new(window, cx)));
        }
        self.show_run_status = false;
        self.show_reply_list = false;
        self.show_post_history = true;
        self.show_task_settings = false;
        self.show_auto_post = false;
        cx.notify();
    }

    fn open_task_settings(&mut self, cx: &mut Context<Self>) {
        app_log::x_info!("X 任务设置：收到打开页面事件");
        self.show_run_status = false;
        self.show_reply_list = false;
        self.show_post_history = false;
        self.show_task_settings = true;
        self.show_auto_post = false;
        self.error = None;
        cx.notify();
    }

    fn open_auto_post(&mut self, cx: &mut Context<Self>) {
        self.show_run_status = false;
        self.show_reply_list = false;
        self.show_post_history = false;
        self.show_task_settings = false;
        self.show_auto_post = true;
        self.error = None;
        self.status = None;
        cx.notify();
    }

    fn on_reply_rules_input(
        &mut self,
        input: &Entity<InputState>,
        _: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rules = input.read(cx).text().to_string();
        let path = PathBuf::from("config").join("x_reply_rules.txt");
        let saved = std::fs::create_dir_all(path.parent().unwrap())
            .and_then(|()| std::fs::write(path, rules));
        if let Err(error) = saved {
            self.error = Some(format!("保存回复规则失败：{error}"));
        }
        cx.notify();
    }

    fn stop_visual_reply(&mut self, cx: &mut Context<Self>) {
        let token_id = Arc::as_ptr(&self.cancel_reply) as usize;
        let reason_set = self.cancel_reason.compare_exchange(
            0, 1, Ordering::AcqRel, Ordering::Acquire
        ).is_ok();
        let cancel_reason = self.cancel_reason.load(Ordering::Acquire);
        let was_running = self.cancel_reply.swap(true, Ordering::AcqRel);
        app_log::x_warn!(
            "X 取消来源诊断：停止按钮写入取消标记，token=0x{:x}，此前已取消={}，本次新发送={}，本次设置来源={}，最终来源={}，调用栈：{}",
            token_id,
            was_running,
            !was_running,
            reason_set,
            cancel_reason,
            Backtrace::force_capture()
        );
        self.status = Some("已收到停止请求，正在等待后台任务安全退出；若发布操作可能已发生，结果会保留为待核验状态，不会自动重发。".to_string());
        cx.notify();
    }

    fn toggle_auto_send(&mut self, cx: &mut Context<Self>) {
        self.auto_send = !self.auto_send;
        cx.notify();
    }

    fn open_image_preview(&mut self, cx: &mut Context<Self>) {
        self.image_preview_open = true;
        cx.notify();
    }

    fn close_image_preview(&mut self, cx: &mut Context<Self>) {
        self.image_preview_open = false;
        cx.notify();
    }
}

fn load_reply_rules() -> String {
    let path = PathBuf::from("config").join("x_reply_rules.txt");
    std::fs::read_to_string(path).unwrap_or_else(|_| {
        "用自然、简短的中文回应帖子内容，不超过 10 字；不要照抄正文，不要提及自己是 AI。"
            .to_string()
    })
}

impl XAssistantPage {
    // 将每个按钮拆分到独立函数，避免在 GPUI render 回调中创建一个过大的临时栈帧。
    fn render_status_tab_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.show_run_status;
        v_flex()
            .w(px(112.))
            .gap_1()
            .child(
                Button::new("x-assistant-status-tab")
                    .ghost()
                    .small()
                    .label("运行状态")
                    .on_click(cx.listener(|this, _, _, cx| {
                this.show_run_status = true;
                this.show_reply_list = false;
                this.show_post_history = false;
                this.show_task_settings = false;
                this.show_auto_post = false;
                cx.notify();
            })),
            )
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .when(selected, |line| line.bg(gpui::hsla(0.59, 0.78, 0.56, 1.0))),
            )
            .into_any_element()
    }
    fn render_reply_flow_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = !self.show_run_status && !self.show_reply_list && !self.show_post_history && !self.show_task_settings && !self.show_auto_post;
        v_flex()
            .w(px(112.))
            .gap_1()
            .child(
                Button::new("x-assistant-reply-flow-tab")
                    .ghost()
                    .small()
                    .label("自动回复")
                    .on_click(cx.listener(|this, _, _, cx| this.show_reply_flow(cx))),
            )
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .when(selected, |line| line.bg(gpui::hsla(0.59, 0.78, 0.56, 1.0))),
            )
            .into_any_element()
    }
    fn render_auto_post_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.show_auto_post;
        v_flex()
            .w(px(112.))
            .gap_1()
            .child(
                Button::new("x-assistant-auto-post-tab")
                    .ghost()
                    .small()
                    .label("自动发帖")
                    .on_click(cx.listener(|this, _, _, cx| this.open_auto_post(cx))),
            )
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .when(selected, |line| line.bg(gpui::hsla(0.59, 0.78, 0.56, 1.0))),
            )
            .into_any_element()
    }
    fn render_start_all_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("x-assistant-start-all-automation")
            .label("开始")
            .small()
            .rounded_full()
            .bg(gpui::hsla(0.39, 0.58, 0.40, 1.0))
            .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
            .px_4()
            .disabled(self.replying)
            .tooltip("启动自动化回复、发帖")
            .on_click(cx.listener(|this, _, _, cx| this.start_all_automation(cx)))
            .into_any_element()
    }

    fn render_stop_all_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("x-assistant-stop-all-automation")
            .label("停止")
            .small()
            .rounded_full()
            .bg(gpui::hsla(0.00, 0.68, 0.52, 1.0))
            .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
            .px_4()
            .disabled(!self.replying)
            .tooltip("中断自动化")
            .on_click(cx.listener(|this, _, _, cx| this.stop_visual_reply(cx)))
            .into_any_element()
    }

    fn render_reply_list_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.show_reply_list;
        v_flex()
            .w(px(112.))
            .gap_1()
            .child(
                Button::new("x-assistant-reply-list-tab")
                    .ghost()
                    .small()
                    .label("回复列表")
                    .on_click(cx.listener(|this, _, window, cx| this.open_reply_list(window, cx))),
            )
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .when(selected, |line| line.bg(gpui::hsla(0.59, 0.78, 0.56, 1.0))),
            )
            .into_any_element()
    }
    fn render_post_history_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.show_post_history;
        v_flex()
            .w(px(112.))
            .gap_1()
            .child(
                Button::new("x-assistant-post-history-tab")
                    .ghost()
                    .small()
                    .label("发帖历史")
                    .on_click(cx.listener(|this, _, window, cx| this.open_post_history(window, cx))),
            )
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .when(selected, |line| line.bg(gpui::hsla(0.59, 0.78, 0.56, 1.0))),
            )
            .into_any_element()
    }
    fn render_task_settings_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.show_task_settings;
        v_flex()
            .w(px(112.))
            .gap_1()
            .child(
                Button::new("x-assistant-task-settings-tab")
                    .ghost()
                    .small()
                    .label("任务设置")
                    .on_click(cx.listener(|this, _, _, cx| this.open_task_settings(cx))),
            )
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .when(selected, |line| line.bg(gpui::hsla(0.59, 0.78, 0.56, 1.0))),
            )
            .into_any_element()
    }
    fn render_page_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let status_button = self.render_status_tab_button(cx);
        let reply_flow_button = self.render_reply_flow_button(cx);
        let auto_post_button = self.render_auto_post_button(cx);
        let start_all_button = self.render_start_all_button(cx);
        let stop_all_button = self.render_stop_all_button(cx);
        let reply_list_button = self.render_reply_list_button(cx);
        let post_history_button = self.render_post_history_button(cx);
        let task_settings_button = self.render_task_settings_button(cx);

        let theme = cx.theme();
        // 六个页面统一使用下划线 Tabs；运行状态和全局控制按钮位于同一行。
        let run_state = div()
            .rounded_full()
            .border_1()
            .border_color(if self.error.is_some() {
                gpui::hsla(0.00, 0.62, 0.78, 1.0)
            } else if self.replying {
                gpui::hsla(0.39, 0.42, 0.78, 1.0)
            } else {
                palette::border(theme)
            })
            .bg(if self.error.is_some() {
                gpui::hsla(0.00, 0.72, 0.95, 1.0)
            } else if self.replying {
                gpui::hsla(0.39, 0.42, 0.94, 1.0)
            } else {
                palette::surface(theme)
            })
            .px_3()
            .py_1()
            .text_size(px(12.))
            .font_semibold()
            .text_color(if self.error.is_some() {
                palette::error_text()
            } else if self.replying {
                gpui::hsla(0.39, 0.52, 0.30, 1.0)
            } else {
                palette::muted(theme)
            })
            .child(if self.error.is_some() {
                "● 运行异常"
            } else if self.replying {
                "● 运行中"
            } else {
                "● 已停止"
            })
            .into_any_element();

        h_flex()
            .items_end()
            .gap_2()
            .child(status_button)
            .child(reply_flow_button)
            .child(auto_post_button)
            .child(reply_list_button)
            .child(post_history_button)
            .child(task_settings_button)
            .child(div().flex_1())
            .child(run_state)
            .child(start_all_button)
            .child(stop_all_button)
            .into_any_element()
    }
}

impl Render for XAssistantPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.render_diagnostic_logged {
            self.render_diagnostic_logged = true;
        }
        if self.show_run_status {
            let menu = self.render_page_menu(cx);
            let theme = cx.theme();
            let logs = self.log_tail.as_ref().map(run_log::LogTail::lines).unwrap_or(&[]);
            let reply_stats = binance_tools::db::x_assistant::reply_stats_blocking().ok();
            let published_posts = binance_tools::db::x_posting::list_published_posts_blocking(500)
                .unwrap_or_default();
            let today = app_log::beijing_now().format("%Y-%m-%d").to_string();
            let today_posts = published_posts.iter().filter(|post| {
                let timestamp = post.published_at.as_deref().unwrap_or(&post.created_at);
                app_log::format_beijing_timestamp(timestamp).starts_with(&today)
            }).collect::<Vec<_>>();
            let post_total = today_posts.len() as u64;
            let post_success = today_posts.iter().filter(|post| post.status == "published").count() as u64;
            let post_failed = today_posts.iter().filter(|post| post.status == "failed").count() as u64;
            let stat = |label: &'static str, value: u64, color: gpui::Hsla, label_color: gpui::Hsla, background: gpui::Hsla| {
                div().flex_1().min_w_0().rounded(px(7.)).border_1()
                    .border_color(background).bg(background)
                    .px_3().py_2()
                    .child(div().text_size(px(10.)).text_color(label_color).child(label))
                    .child(div().text_size(px(18.)).font_semibold().text_color(color).child(value.to_string()))
            };
            let log_content = if self.log_tail.is_none() {
                div().flex_1().min_h_0().text_size(px(12.)).text_color(palette::muted(theme))
                    .child("X 助手未启动，日志读取已停止。启动任务后才会读取 logs 目录中的日志。").into_any_element()
            } else if logs.is_empty() {
                div().flex_1().min_h_0().text_size(px(12.)).text_color(palette::muted(theme))
                    .child("等待日志输出…").into_any_element()
            } else {
                v_flex().flex_1().min_h_0().gap_1().overflow_y_scrollbar()
                    .children(logs.iter().map(|line| {
                        let color = if line.contains("[ERROR]") { palette::error_text() }
                            else if line.contains("[WARN]") { gpui::hsla(0.12, 0.82, 0.48, 1.0) }
                            else { palette::text(theme) };
                        div().text_size(px(11.)).text_color(color).child(line.clone())
                    })).into_any_element()
            };
            let reply_stats_panel = v_flex().flex_1().min_w_0().gap_2()
                .child(div().text_size(px(13.)).font_semibold().child("自动回复统计"))
                .child(h_flex().gap_2()
                    .child(stat("今日总数", reply_stats.as_ref().map_or(0, |v| v.today_total), gpui::hsla(0.59, 0.75, 0.34, 1.0), gpui::hsla(0.59, 0.48, 0.43, 1.0), gpui::hsla(0.59, 0.72, 0.95, 1.0)))
                    .child(stat("成功", reply_stats.as_ref().map_or(0, |v| v.today_success), gpui::hsla(0.39, 0.52, 0.34, 1.0), gpui::hsla(0.39, 0.38, 0.42, 1.0), gpui::hsla(0.39, 0.42, 0.94, 1.0)))
                    .child(stat("失败", reply_stats.as_ref().map_or(0, |v| v.today_failed), gpui::hsla(0.00, 0.62, 0.42, 1.0), gpui::hsla(0.00, 0.45, 0.48, 1.0), gpui::hsla(0.00, 0.72, 0.95, 1.0))));
            let post_stats_panel = v_flex().flex_1().min_w_0().gap_2()
                .child(div().text_size(px(13.)).font_semibold().child("自动发帖统计"))
                .child(h_flex().gap_2()
                    .child(stat("今日总数", post_total, gpui::hsla(0.59, 0.75, 0.34, 1.0), gpui::hsla(0.59, 0.48, 0.43, 1.0), gpui::hsla(0.59, 0.72, 0.95, 1.0)))
                    .child(stat("成功", post_success, gpui::hsla(0.39, 0.52, 0.34, 1.0), gpui::hsla(0.39, 0.38, 0.42, 1.0), gpui::hsla(0.39, 0.42, 0.94, 1.0)))
                    .child(stat("失败", post_failed, gpui::hsla(0.00, 0.62, 0.42, 1.0), gpui::hsla(0.00, 0.45, 0.48, 1.0), gpui::hsla(0.00, 0.72, 0.95, 1.0))));
            let stats_row = h_flex().gap_3()
                .child(reply_stats_panel)
                .child(post_stats_panel);
            return v_flex().size_full().min_w_0().min_h_0().gap_3().p_4()
                .child(menu)
                .child(stats_row)
                .child(v_flex().flex_1().min_h_0().gap_2()
                    .rounded(px(8.)).border_1().border_color(palette::border(theme))
                    .bg(gpui::hsla(0.61, 0.08, 0.95, 1.0)).p_3()
                    .child(h_flex().items_center().justify_between()
                        .child(div().text_size(px(14.)).font_semibold().child("实时运行日志"))
                        .child(div().text_size(px(11.)).text_color(if self.replying { theme.success } else { palette::muted(theme) })
                            .child(if self.replying { "● 实时读取中" } else { "● 已停止读取" })))
                    .child(log_content))
                .into_any_element();
        }

        if self.show_task_settings {
            let menu = self.render_page_menu(cx);
            let task_settings = self.render_task_settings(cx);
            let theme = cx.theme();
            return v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .gap_3()
                .p_4()
                .child(menu)
                .child(
                    v_flex()
                        .gap_1()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(palette::border(theme))
                        .bg(palette::surface(theme))
                        .p_4()
                        .child(div().text_size(px(20.)).font_semibold().child("任务设置"))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette::muted(theme))
                                .child("集中管理自动回复、自动发帖、自动评论和发布频率。这里的开关只负责任务配置，不会和回复操作按钮混排。"),
                        ),
                )
                .child(task_settings)
                .into_any_element();
        }

        if self.show_auto_post {
            let menu = self.render_page_menu(cx);
            let theme = cx.theme();
            return v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .gap_3()
                .p_4()
                .child(menu)
                .child(
                    v_flex()
                        .gap_2()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(palette::border(theme))
                        .bg(palette::surface(theme))
                        .p_4()
                        .child(div().text_size(px(20.)).font_semibold().child("自动发帖"))
                        .child(div().text_size(px(12.)).text_color(palette::muted(theme)).child("先选择新闻类型，再测试单条真实发帖或启动持续自动发布。"))
                        .child(div().text_size(px(13.)).font_semibold().child("新闻类型"))
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new("x-auto-post-type-all")
                                        .small()
                                        .label("全部新闻")
                                        .when(self.task_settings.auto_post_news_type == "all", |button| button.primary())
                                        .when(self.task_settings.auto_post_news_type != "all", |button| button.outline())
                                        .disabled(self.replying)
                                        .on_click(cx.listener(|this, _, _, cx| this.set_auto_post_news_type("all", cx))),
                                )
                                .child(
                                    Button::new("x-auto-post-type-crypto")
                                        .small()
                                        .label("币圈新闻")
                                        .when(self.task_settings.auto_post_news_type == "crypto", |button| button.primary())
                                        .when(self.task_settings.auto_post_news_type != "crypto", |button| button.outline())
                                        .disabled(self.replying)
                                        .on_click(cx.listener(|this, _, _, cx| this.set_auto_post_news_type("crypto", cx))),
                                )
                                .child(
                                    Button::new("x-auto-post-type-entertainment")
                                        .small()
                                        .label("娱乐新闻")
                                        .when(self.task_settings.auto_post_news_type == "entertainment", |button| button.primary())
                                        .when(self.task_settings.auto_post_news_type != "entertainment", |button| button.outline())
                                        .disabled(self.replying)
                                        .on_click(cx.listener(|this, _, _, cx| this.set_auto_post_news_type("entertainment", cx))),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Button::new("x-auto-post-test")
                                        .label("测试发帖（真实发布 1 条）")
                                        .outline()
                                        .disabled(self.replying)
                                        .on_click(cx.listener(|this, _, _, cx| this.start_test_auto_post(cx))),
                                )
                                .child(
                                    Button::new("x-auto-post-start")
                                        .label(if self.replying && self.cancel_reply.load(Ordering::Acquire) { "正在停止…" } else if self.replying { "自动发帖运行中…" } else { "启动自动发帖" })
                                        .primary()
                                        .disabled(self.replying || !self.task_settings.auto_post_enabled)
                                        .on_click(cx.listener(|this, _, _, cx| this.start_auto_post(cx))),
                                )
                                .child(
                                    Button::new("x-auto-post-stop")
                                        .label("停止")
                                        .outline()
                                        .disabled(!self.replying)
                                        .on_click(cx.listener(|this, _, _, cx| this.stop_visual_reply(cx))),
                                ),
                        )
                        .child(
                            div()
                                .rounded(px(8.))
                                .border_1()
                                .border_color(palette::border(theme))
                                .bg(theme.background)
                                .p_3()
                                .text_size(px(12.))
                                .text_color(palette::muted(theme))
                                 .child(if self.task_settings.auto_post_enabled {
                                    "自动发帖已启用。正式启动会按所选新闻类型持续筛选并发布，受每小时上限控制。"
                                } else {
                                    "测试按钮会真实发布一条帖子；正式连续发布前，请先在「任务设置」中开启自动发帖。"
                                })
                                .child("测试发帖同样计入每小时发布上限。若提交后结果不明确，会暂停并要求人工核验，不会自动重发。"),
                        ),
                )
                .into_any_element();
        }

        if self.show_reply_list || self.show_post_history {
            return v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .gap_3()
                .p_4()
                .child(self.render_page_menu(cx))
                .when_some(self.reply_list.clone(), |page, reply_list| {
                    if self.show_reply_list { page.child(reply_list) } else { page }
                })
                .when_some(self.post_history.clone(), |page, post_history| {
                    if self.show_post_history { page.child(post_history) } else { page }
                })
                .into_any_element();
        }
        let menu = self.render_page_menu(cx);
        let theme = cx.theme();
        let selected_image = self.screenshots.get(self.selected_screenshot).cloned();
        let zoom_image = selected_image.clone();

        let element = v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .relative()
            .gap_3()
            .p_4()
            .child(menu)
            .child(
                v_flex()
                    .gap_1()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(palette::border(theme))
                    .bg(palette::surface(theme))
                    .p_4()
                    .child(div().text_size(px(20.)).font_semibold().child("X 辅助"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(theme))
                            .child("截取浏览器画面，交给 192.168.31.60 的 Ollama 视觉模型识别，再模拟鼠标键盘回复。"),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(palette::surface(theme))
                    .p_3()
                    .child(
                        Button::new("x-assistant-open-x")
                            .outline()
                            .small()
                            .label("打开 X")
                            .on_click(|_, _, cx| cx.open_url("https://x.com/home")),
                    )
                    .child(
                        Button::new("x-assistant-visual-reply-hover")
                            .outline()
                            .small()
                            .label(if self.replying && self.cancel_reply.load(Ordering::Acquire) {
                                "正在停止…"
                            } else if self.replying {
                                "运行中…"
                            } else {
                                "移到回复按钮"
                            })
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(true, false, false, cx))),
                    )
                    .child(
                        Button::new("x-assistant-visual-reply-full")
                            .outline()
                            .small()
                            .label(if self.replying && self.cancel_reply.load(Ordering::Acquire) {
                                "正在停止…"
                            } else if self.replying {
                                "自动回复中（视觉 AI 阶段二）…"
                            } else {
                                "自动回复（视觉 AI 阶段二）"
                            })
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(false, false, false, cx))),
                    )
                    .child(
                        Button::new("x-assistant-text-ocr-reply")
                            .label(if self.replying && self.cancel_reply.load(Ordering::Acquire) { "正在停止…" } else if self.replying { "自动回复中（文本 AI + OCR）…" } else { "自动回复（文本 AI + OCR 阶段二）" })
                            .primary()
                            .small()
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(false, true, false, cx))),
                    )
                    .child(
                        Button::new("x-assistant-full-auto-reply")
                            .outline()
                            .small()
                            .label(if self.replying && self.cancel_reply.load(Ordering::Acquire) { "正在停止…" } else if self.replying { "全自动运行中…" } else { "全自动回复" })
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(false, true, true, cx))),
                    )
                    .child(
                        Button::new("x-assistant-stop-reply")
                            .outline()
                            .small()
                            .label("停止")
                            .disabled(!self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.stop_visual_reply(cx))),
                    )
                    .child(
                        Button::new("x-assistant-auto-send")
                            .outline()
                            .small()
                            .label(if self.auto_send {
                                "自动发送：开"
                            } else {
                                "自动发送：关"
                            })
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_send(cx))),
                    )
                    .child(
                        div()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(palette::border_soft(theme))
                            .bg(theme.background)
                            .px_2()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(palette::muted(theme))
                            .child(format!("模型 {}", self.vision_model)),
                    )
                    .child(
                        Button::new("x-assistant-copy-fields")
                            .ghost()
                            .xsmall()
                            .label("复制字段")
                            .disabled(self.extracted_fields.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.extracted_fields.clone()));
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(palette::border(theme))
                    .bg(palette::surface(theme))
                    .p_3()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_size(px(14.)).font_semibold().child("回复规则"))
                            .child(
                                Button::new("x-assistant-toggle-reply-rules")
                                    .ghost()
                                    .xsmall()
                                    .label(if self.show_reply_rules { "收起规则 ▴" } else { "展开规则 ▾" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_reply_rules = !this.show_reply_rules;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.show_reply_rules, |panel| {
                        panel.child(
                            div()
                                .h(px(120.))
                                .w_full()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(palette::border(theme))
                                .bg(theme.background)
                                .p_2()
                                .child(Input::new(&self.reply_rules).appearance(false)),
                        )
                    }),
            )
            .when_some(self.status.clone(), |parent, status| {
                parent.child(
                    div()
                        .w_full()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if self.error.is_some() {
                            palette::error_border()
                        } else if self.replying {
                            palette::border(theme)
                        } else {
                            theme.success.opacity(0.35)
                        })
                        .bg(if self.error.is_some() {
                            palette::error_background()
                        } else if self.replying {
                            theme.background
                        } else {
                            theme.success.opacity(0.10)
                        })
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(if self.error.is_some() {
                            palette::error_text()
                        } else if self.replying {
                            palette::muted(theme)
                        } else {
                            palette::text(theme)
                        })
                        .truncate()
                        .child(status),
                )
            })
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    v_flex()
                        .w_full()
                        .gap_1()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(palette::error_border())
                        .bg(palette::error_background())
                        .px_3()
                        .py_2()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_semibold()
                                .text_color(palette::error_text())
                                .child("自动回复中断"),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette::error_text())
                                .child(error),
                        ),
                )
            })
            .when_some(
                selected_image,
                |parent, image_path| {
                    parent.child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_start()
                            .gap_4()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_3()
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(palette::border(theme))
                                    .bg(palette::surface(theme))
                                    .p_3()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(div().text_size(px(14.)).font_semibold().child("截图预览"))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(palette::muted(theme))
                                                    .child("点击图片可放大查看"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .flex()
                                            .items_start()
                                            .justify_center()
                                            .child(
                                                div()
                                                    .id("x-assistant-image-preview")
                                                    .w_full()
                                                    .h(px(320.))
                                                    .rounded(px(8.))
                                                    .cursor_pointer()
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _, cx| this.open_image_preview(cx)),
                                                    )
                                                    .child(img(image_path.clone()).size_full().object_fit(ObjectFit::Contain)),
                                            ),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_3()
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(palette::border(theme))
                                    .bg(palette::surface(theme))
                                    .p_3()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .justify_between()
                                            .child(div().text_size(px(14.)).font_semibold().child("解析内容"))
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(palette::muted(theme))
                                                    .child("发送者 · 账号 · 时间 · 正文"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_y_scrollbar()
                                            .w_full()
                                            .py_2()
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .child(self.extracted_fields.clone()),
                                            ),
                                    ),
                            ),
                    )
                },
            )
            .when(self.image_preview_open, |parent| {
                parent.when_some(zoom_image, |parent, image_path| {
                    parent.child(
                        div()
                            .size_full()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .occlude()
                            .bg(gpui::black().opacity(0.62))
                            .child(
                                v_flex()
                                    .absolute()
                                    .top(px(24.))
                                    .left(px(32.))
                                    .right(px(32.))
                                    .bottom(px(24.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(theme.border)
                                    .shadow_md()
                                    .bg(theme.background)
                                    .child(
                                        h_flex()
                                            .h(px(44.))
                                            .items_center()
                                            .justify_between()
                                            .px_3()
                                            .border_b_1()
                                            .border_color(theme.border)
                                            .child(div().font_semibold().child("图片预览"))
                                            .child(
                                                Button::new("x-assistant-close-image-preview")
                                                    .label("关闭")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.close_image_preview(cx);
                                                    })),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .p_2()
                                            .child(img(image_path).size_full().object_fit(ObjectFit::Contain)),
                                    ),
                            ),
                    )
                })
            })
            .into_any_element();
        element
    }
}
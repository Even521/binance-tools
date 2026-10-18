mod automation;

mod vision;

use crate::ui::palette;

use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct XAssistantPage {
    replying: bool,
    auto_send: bool,
    vision_model: String,
    reply_rules: Entity<InputState>,
    _reply_rules_subscription: Subscription,
    cancel_reply: Arc<AtomicBool>,
    extracted_fields: String,
    screenshots: Vec<PathBuf>,
    selected_screenshot: usize,
    image_preview_open: bool,
    status: Option<String>,
    error: Option<String>,
    _reply_task: Task<()>,
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

        Self {
            replying: false,
            auto_send: true,
            vision_model: vision::DEFAULT_VISION_MODEL.to_string(),
            reply_rules,
            _reply_rules_subscription: reply_rules_subscription,
            cancel_reply: Arc::new(AtomicBool::new(false)),
            extracted_fields: String::new(),
            screenshots: Vec::new(),
            selected_screenshot: 0,
            image_preview_open: false,
            status: None,
            error: None,
            _reply_task: Task::ready(()),
        }
    }

    fn start_visual_reply(&mut self, hover_only: bool, cx: &mut Context<Self>) {
        if self.replying {
            return;
        }
        self.replying = true;
        self.error = None;
        self.cancel_reply.store(false, Ordering::Relaxed);
        self.status = Some(if hover_only {
            "5 秒后开始识别并移到回复按钮。AI 识图约需 15~30 秒，请切到 X 浏览器窗口。".to_string()
        } else {
            "5 秒后开始自动回复流程。AI 识图约需 15~30 秒，请确保 X 浏览器处于前台。".to_string()
        });
        cx.notify();

        let auto_send = self.auto_send;
        let reply_rules = self.reply_rules.read(cx).text().to_string();
        let model = self.vision_model.clone();
        let cancel = Arc::clone(&self.cancel_reply);
        self._reply_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    vision::run_visual_reply(
                        vision::VisionReplyOptions {
                            api_url: vision::DEFAULT_OLLAMA_URL.to_string(),
                            model,
                            auto_send,
                            hover_only,
                            reply_rules,
                        },
                        &cancel,
                    )
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.replying = false;
                match result {
                    Ok(reply) => {
                        this.screenshots = reply.screenshots;
                        this.selected_screenshot = this.screenshots.len().saturating_sub(1);
                        this.extracted_fields = format!(
                            "帖子：\n{}\n\n回复：\n{}\n\n{}",
                            reply.post_text, reply.reply_text, reply.summary
                        );
                        this.status = Some(if reply.completed {
                            format!("已完成。{}", reply.summary)
                        } else {
                            reply.summary
                        });
                    }
                    Err(err) => {
                        let message = format!("{err:#}");
                        if message.contains("已停止") {
                            this.status = Some("已停止，AI 识图已中断。".to_string());
                        } else {
                            this.error = Some(format!("视觉回复失败：{message}"));
                        }
                    }
                }
                cx.notify();
            });
        });
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
        self.cancel_reply.store(true, Ordering::Relaxed);
        self.status = Some("已停止，正在中断 AI 识图。".to_string());
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

impl Render for XAssistantPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected_image = self.screenshots.get(self.selected_screenshot).cloned();
        let zoom_image = selected_image.clone();

        v_flex()
            .size_full()
            .relative()
            .gap_3()
            .p_3()
            .child(
                v_flex()
                    .gap_1()
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
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(palette::surface(theme))
                    .p_2()
                    .child(
                        Button::new("x-assistant-open-x")
                            .label("打开 X")
                            .on_click(|_, _, cx| cx.open_url("https://x.com/home")),
                    )
                    .child(
                        Button::new("x-assistant-visual-reply-hover")
                            .label(if self.replying {
                                "运行中…"
                            } else {
                                "移到回复按钮"
                            })
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(true, cx))),
                    )
                    .child(
                        Button::new("x-assistant-visual-reply-full")
                            .label(if self.replying {
                                "自动回复中…"
                            } else {
                                "自动回复"
                            })
                            .primary()
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(false, cx))),
                    )
                    .child(
                        Button::new("x-assistant-stop-reply")
                            .label("停止")
                            .disabled(!self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.stop_visual_reply(cx))),
                    )
                    .child(
                        Button::new("x-assistant-auto-send")
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
                            .px_2()
                            .text_size(px(12.))
                            .text_color(palette::muted(theme))
                            .child(format!("模型 {}", self.vision_model)),
                    )
                    .child(
                        Button::new("x-assistant-copy-fields")
                            .label("复制字段")
                            .disabled(self.extracted_fields.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.extracted_fields.clone()));
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_size(px(13.)).font_semibold().child("回复规则"))
                    .child(
                        div()
                            .h(px(92.))
                            .w_full()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.border)
                            .bg(palette::surface(theme))
                            .p_2()
                            .child(Input::new(&self.reply_rules).appearance(false)),
                    ),
            )
            .when_some(self.status.clone(), |parent, status| {
                parent.child(
                    div()
                        .w_full()
                        .rounded(px(6.))
                        .bg(palette::surface(theme))
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(palette::muted(theme))
                        .truncate()
                        .child(status),
                )
            })
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .w_full()
                        .rounded(px(6.))
                        .bg(palette::error_background())
                        .px_3()
                        .py_2()
                        .text_color(palette::error_text())
                        .child(error),
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
                            .gap_3()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_2()
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
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                div()
                                                    .id("x-assistant-image-preview")
                                                    .w(px(520.))
                                                    .h(px(640.))
                                                    .rounded(px(6.))
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
                                    .gap_2()
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
    }
}

mod automation;
mod full_auto;
mod reply_list;
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
    show_reply_rules: bool,
    status: Option<String>,
    error: Option<String>,
    _reply_task: Task<()>,
    show_reply_list: bool,
    reply_list: Entity<reply_list::ReplyListPage>,
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
        let reply_list = cx.new(|cx| reply_list::ReplyListPage::new(window, cx));

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
            show_reply_rules: false,
            status: None,
            error: None,
            _reply_task: Task::ready(()),
            show_reply_list: false,
            reply_list,
        }
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
        self.error = None;
        self.cancel_reply.store(false, Ordering::Relaxed);
        self.status = Some(if full_auto {
            "全自动循环已启动；每轮后随机刷新或滚动，点击停止按钮可结束。".to_string()
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
                        full_auto::run(options, &cancel)
                    } else {
                        vision::run_visual_reply(options, &cancel)
                    }
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.replying = false;
                match result {
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
                    }
                    Err(err) => {
                        this.status = None;
                        this.error = Some(format!("{err:#}"));
                    }
                }
                cx.notify();
            });
        });
    }

    fn open_reply_list(&mut self, cx: &mut Context<Self>) {
        self.show_reply_list = true;
        self.reply_list.update(cx, |page, cx| page.reload(cx));
        cx.notify();
    }

    fn show_reply_flow(&mut self, cx: &mut Context<Self>) {
        self.show_reply_list = false;
        self.error = None;
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

impl XAssistantPage {
    fn render_page_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_2()
            .child(
                Button::new("x-assistant-reply-flow-tab")
                    .label("自动回复")
                    .when(!self.show_reply_list, |button| button.primary())
                    .when(self.show_reply_list, |button| button.outline())
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.show_reply_flow(cx))),
            )
            .child(
                Button::new("x-assistant-reply-list-tab")
                    .label("回复列表")
                    .when(self.show_reply_list, |button| button.primary())
                    .when(!self.show_reply_list, |button| button.outline())
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.open_reply_list(cx))),
            )
            .into_any_element()
    }
}

impl Render for XAssistantPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.show_reply_list {
            return v_flex()
                .size_full()
                .gap_3()
                .p_3()
                .child(self.render_page_menu(cx))
                .child(self.reply_list.clone())
                .into_any_element();
        }
        let menu = self.render_page_menu(cx);
        let theme = cx.theme();
        let selected_image = self.screenshots.get(self.selected_screenshot).cloned();
        let zoom_image = selected_image.clone();

        v_flex()
            .size_full()
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
                            .label(if self.replying {
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
                            .label(if self.replying {
                                "自动回复中（视觉 AI 阶段二）…"
                            } else {
                                "自动回复（视觉 AI 阶段二）"
                            })
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(false, false, false, cx))),
                    )
                    .child(
                        Button::new("x-assistant-text-ocr-reply")
                            .label(if self.replying { "自动回复中（文本 AI + OCR）…" } else { "自动回复（文本 AI + OCR 阶段二）" })
                            .primary()
                            .small()
                            .disabled(self.replying)
                            .on_click(cx.listener(|this, _, _, cx| this.start_visual_reply(false, true, false, cx))),
                    )
                    .child(
                        Button::new("x-assistant-full-auto-reply")
                            .outline()
                            .small()
                            .label(if self.replying { "全自动运行中…" } else { "全自动回复" })
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
                        } else {
                            theme.success.opacity(0.35)
                        })
                        .bg(if self.error.is_some() {
                            palette::error_background()
                        } else {
                            theme.success.opacity(0.10)
                        })
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(if self.error.is_some() {
                            palette::error_text()
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
            .into_any_element()
    }
}

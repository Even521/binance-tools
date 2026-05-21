use crate::ui::palette;
use binance_tools::ai::{
    AiSettings, ChatMessage as CoreChatMessage, ModelSelection,
    send_chat_with_model_streaming_blocking,
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    spinner::Spinner,
    text::TextView,
    tooltip::Tooltip,
    v_flex,
};
use std::{
    sync::mpsc::{self, TryRecvError},
    time::Duration,
};

actions!(ai_chat, [ToggleAiChat, OpenAiProviders]);

const THINKING_MESSAGE: &str = "Thinking...";

pub enum AiChatEvent {
    OpenProviders,
}

#[derive(Clone, Debug)]
struct ChatMessage {
    role: MessageRole,
    content: String,
    request_content: Option<String>,
    feedback: Option<MessageFeedback>,
    failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageFeedback {
    Like,
    Dislike,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageAction {
    Like,
    Dislike,
    Copy,
    Retry,
    Continue,
}

impl MessageAction {
    fn id(self) -> &'static str {
        match self {
            Self::Like => "assistant-like",
            Self::Dislike => "assistant-dislike",
            Self::Copy => "assistant-copy",
            Self::Retry => "assistant-retry",
            Self::Continue => "assistant-continue",
        }
    }
}

#[derive(Clone, Debug)]
struct ModelSelectItem {
    selection: ModelSelection,
    label: String,
}

#[derive(Clone, Debug)]
struct ContextUsage {
    input_used: usize,
    input_limit: Option<u32>,
    output_used: usize,
    output_limit: Option<u32>,
}

pub struct AiChatPanel {
    visible: bool,
    messages: Vec<ChatMessage>,
    input: Entity<InputState>,
    model_search: Entity<InputState>,
    selected_model: ModelSelection,
    model_picker_open: bool,
    composer_expanded: bool,
    ai_settings: AiSettings,
    settings_status: Option<String>,
    loading: bool,
    _send_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl AiChatPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (ai_settings, settings_status) = match AiSettings::load_default() {
            Ok(settings) => (settings, None),
            Err(err) => (AiSettings::default(), Some(err.to_string())),
        };
        let selected_model = first_custom_model(&ai_settings)
            .map(|item| item.selection)
            .unwrap_or_else(|| ai_settings.selected_model().clone());

        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(
                    ai_settings.agent.message_editor_min_lines.into(),
                    ai_settings
                        .agent
                        .message_editor_min_lines
                        .saturating_mul(2)
                        .into(),
                )
                .placeholder("Message the MoonRich Agent - @ to include context")
                .default_value("")
        });
        let model_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Select a model...")
                .default_value("")
        });
        let _subscriptions = vec![cx.subscribe_in(&input, window, Self::on_input_event)];

        Self {
            visible: false,
            messages: Vec::new(),
            input,
            model_search,
            selected_model,
            model_picker_open: false,
            composer_expanded: false,
            ai_settings,
            settings_status,
            loading: false,
            _send_task: Task::ready(()),
            _subscriptions,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        cx.notify();
    }

    pub fn reload_ai_settings(&mut self, cx: &mut Context<Self>) {
        let selected_model = self.selected_model.clone();
        match AiSettings::load_default() {
            Ok(settings) => {
                self.selected_model = if custom_model_exists(&settings, &selected_model) {
                    selected_model
                } else {
                    first_custom_model(&settings)
                        .map(|item| item.selection)
                        .unwrap_or(selected_model)
                };
                self.ai_settings = settings;
                self.settings_status = None;
            }
            Err(err) => {
                self.settings_status = Some(err.to_string());
            }
        }
        cx.notify();
    }

    pub fn submit_external_prompt(
        &mut self,
        prompt: String,
        display_content: String,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }

        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        self.visible = true;
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: display_content,
            request_content: Some(prompt),
            feedback: None,
            failed: false,
        });
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: THINKING_MESSAGE.to_string(),
            request_content: None,
            feedback: None,
            failed: false,
        });

        let history = self.core_history();
        let selected_model = self.selected_model();
        let settings = self.ai_settings.clone();
        self.loading = true;
        cx.notify();
        self.dispatch_ai_request(settings, selected_model, history, cx);
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::PressEnter { secondary } = event {
            if !secondary {
                self.send_message(window, cx);
                cx.stop_propagation();
            }
        }
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }

        let text = self.input.read(cx).text().to_string().trim().to_string();
        if text.is_empty() {
            return;
        }

        let selected_model = self.selected_model();
        let settings = self.ai_settings.clone();

        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: text,
            request_content: None,
            feedback: None,
            failed: false,
        });
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: THINKING_MESSAGE.to_string(),
            request_content: None,
            feedback: None,
            failed: false,
        });

        let history = self.core_history();
        self.loading = true;
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();

        self.dispatch_ai_request(settings, selected_model, history, cx);
    }

    fn dispatch_ai_request(
        &mut self,
        settings: AiSettings,
        selected_model: ModelSelection,
        history: Vec<CoreChatMessage>,
        cx: &mut Context<Self>,
    ) {
        enum StreamEvent {
            Delta(String),
            Done(Result<(), String>),
        }

        self._send_task = cx.spawn(async move |this, cx| {
            let (tx, rx) = mpsc::channel::<StreamEvent>();
            let worker_tx = tx.clone();
            let worker = cx.background_spawn(async move {
                let result = send_chat_with_model_streaming_blocking(
                    &settings,
                    selected_model,
                    &history,
                    |delta| {
                        _ = worker_tx.send(StreamEvent::Delta(delta.to_string()));
                    },
                )
                .map(|_| ())
                .map_err(|err| err.to_string());

                _ = tx.send(StreamEvent::Done(result));
            });

            let mut done = false;
            while !done {
                loop {
                    match rx.try_recv() {
                        Ok(StreamEvent::Delta(delta)) => {
                            _ = this.update(cx, |this, cx| {
                                if let Some(message) = this
                                    .messages
                                    .iter_mut()
                                    .rev()
                                    .find(|message| message.role == MessageRole::Assistant)
                                {
                                    if message.content == THINKING_MESSAGE {
                                        message.content.clear();
                                    }
                                    message.content.push_str(&delta);
                                    message.feedback = None;
                                    message.failed = false;
                                }
                                cx.notify();
                            });
                        }
                        Ok(StreamEvent::Done(result)) => {
                            done = true;
                            _ = this.update(cx, |this, cx| {
                                this.loading = false;
                                if let Err(error) = result {
                                    if let Some(message) = this
                                        .messages
                                        .iter_mut()
                                        .rev()
                                        .find(|message| message.role == MessageRole::Assistant)
                                    {
                                        if message.content == THINKING_MESSAGE
                                            || message.content.trim().is_empty()
                                        {
                                            message.content = error;
                                        } else {
                                            message.content.push_str("\n\n");
                                            message.content.push_str(&error);
                                        }
                                        message.feedback = None;
                                        message.failed = true;
                                    }
                                }
                                cx.notify();
                            });
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            done = true;
                            _ = this.update(cx, |this, cx| {
                                this.loading = false;
                                if let Some(message) = this
                                    .messages
                                    .iter_mut()
                                    .rev()
                                    .find(|message| message.role == MessageRole::Assistant)
                                {
                                    if message.content == THINKING_MESSAGE {
                                        message.content =
                                            "AI 流式响应已中断，请点击重试。".to_string();
                                        message.failed = true;
                                    }
                                }
                                cx.notify();
                            });
                            break;
                        }
                    }
                }

                if !done {
                    Timer::after(Duration::from_millis(24)).await;
                }
            }

            _ = worker.await;
        });
    }

    fn set_message_feedback(
        &mut self,
        index: usize,
        feedback: MessageFeedback,
        cx: &mut Context<Self>,
    ) {
        if let Some(message) = self.messages.get_mut(index) {
            message.feedback = if message.feedback == Some(feedback) {
                None
            } else {
                Some(feedback)
            };
        }
        cx.notify();
    }

    fn retry_assistant_message(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.loading || index >= self.messages.len() {
            return;
        }

        let Some(user_index) = self.messages[..index]
            .iter()
            .rposition(|message| message.role == MessageRole::User)
        else {
            return;
        };

        self.messages.truncate(index);
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: THINKING_MESSAGE.to_string(),
            request_content: None,
            feedback: None,
            failed: false,
        });

        let history = self.core_history();
        let selected_model = self.selected_model();
        let settings = self.ai_settings.clone();
        self.loading = true;
        let _ = user_index;
        cx.notify();
        self.dispatch_ai_request(settings, selected_model, history, cx);
    }

    fn continue_assistant_message(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.messages.is_empty() {
            return;
        }

        let mut history = self.core_history();
        history.push(CoreChatMessage::user("Continue"));
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: THINKING_MESSAGE.to_string(),
            request_content: None,
            feedback: None,
            failed: false,
        });

        let selected_model = self.selected_model();
        let settings = self.ai_settings.clone();
        self.loading = true;
        cx.notify();
        self.dispatch_ai_request(settings, selected_model, history, cx);
    }

    fn toggle_composer_expanded(&mut self, cx: &mut Context<Self>) {
        self.composer_expanded = !self.composer_expanded;
        cx.notify();
    }

    fn selected_model(&self) -> ModelSelection {
        self.selected_model.clone()
    }

    fn core_history(&self) -> Vec<CoreChatMessage> {
        self.messages
            .iter()
            .filter_map(|message| match message.role {
                MessageRole::User => Some(CoreChatMessage::user(
                    message
                        .request_content
                        .as_ref()
                        .unwrap_or(&message.content)
                        .clone(),
                )),
                MessageRole::Assistant
                    if message.content != THINKING_MESSAGE && !message.failed =>
                {
                    Some(CoreChatMessage::assistant(message.content.clone()))
                }
                MessageRole::Assistant => None,
            })
            .collect()
    }

    fn context_usage(&self, cx: &mut Context<Self>) -> ContextUsage {
        let (input_limit, output_limit) = self.selected_model_limits();
        let mut input_used = self
            .messages
            .iter()
            .map(|message| estimate_tokens(&message.content))
            .sum::<usize>();
        let current_input = self.input.read(cx).text().to_string();
        if !current_input.trim().is_empty() {
            input_used += estimate_tokens(&current_input);
        }

        let output_used = self
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.role == MessageRole::Assistant
                    && message.content != THINKING_MESSAGE
                    && !message.failed
            })
            .map(|message| estimate_tokens(&message.content))
            .unwrap_or_default();

        ContextUsage {
            input_used,
            input_limit,
            output_used,
            output_limit,
        }
    }

    fn selected_model_limits(&self) -> (Option<u32>, Option<u32>) {
        self.ai_settings
            .language_models
            .openai_compatible
            .get(&self.selected_model.provider)
            .and_then(|provider| {
                provider
                    .available_models
                    .iter()
                    .find(|model| model.name == self.selected_model.model)
            })
            .map(|model| {
                (
                    Some(model.max_tokens),
                    model.max_output_tokens.or(model.max_completion_tokens),
                )
            })
            .unwrap_or((None, None))
    }

    pub fn render_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().into_any_element();
        }

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(palette::text(cx.theme()))
            .child(self.render_header(cx))
            .when_some(self.settings_status.clone(), |parent, status| {
                parent.child(
                    div()
                        .mx_2()
                        .mt_2()
                        .p_2()
                        .rounded(px(6.))
                        .bg(cx.theme().danger.opacity(0.12))
                        .text_size(px(11.))
                        .text_color(cx.theme().danger_foreground.opacity(0.9))
                        .child(status),
                )
            })
            .when(self.messages.is_empty(), |parent| {
                parent.child(self.render_empty_stage(cx))
            })
            .when(!self.messages.is_empty(), |parent| {
                parent
                    .child(self.render_thread(window, cx))
                    .child(self.render_composer(cx))
            })
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();
        let has_messages = !self.messages.is_empty();
        let title = self
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.clone())
            .unwrap_or_else(|| "Zed Agent".to_string());

        h_flex()
            .items_center()
            .justify_between()
            .h(px(34.))
            .px_2()
            .border_b_1()
            .border_color(palette::border(app_theme))
            .bg(app_theme.background)
            .child(
                h_flex()
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap_1()
                    .text_size(px(13.))
                    .text_color(palette::text_strong(app_theme))
                    .when(!has_messages, |parent| {
                        parent.child(
                            div()
                                .w(px(14.))
                                .h(px(14.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.))
                                .border_1()
                                .border_color(palette::border_soft(app_theme))
                                .text_color(palette::muted(app_theme))
                                .child(Icon::new(IconName::Bot).size_3()),
                        )
                    })
                    .child(title),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when(has_messages, |parent| {
                        parent.child(self.render_header_button(
                            "new-thread",
                            IconName::Plus,
                            true,
                            cx,
                        ))
                    })
                    .child(self.render_header_button(
                        "expand-thread",
                        IconName::ChevronUp,
                        false,
                        cx,
                    ))
                    .child(self.render_header_button("more-thread", IconName::Ellipsis, false, cx)),
            )
    }

    fn render_header_button(
        &self,
        id: &'static str,
        icon: IconName,
        new_thread: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(id)
            .ghost()
            .xsmall()
            .text_size(px(13.))
            .text_color(palette::muted(cx.theme()))
            .when(new_thread, |button| {
                button.on_click(cx.listener(|this, _, _, cx| {
                    this.messages.clear();
                    cx.notify();
                }))
            })
            .child(
                div()
                    .w(px(22.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .hover(|style| style.bg(palette::hover(cx.theme())))
                    .child(Icon::new(icon).size_4()),
            )
    }

    fn render_empty_stage(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();

        v_flex()
            .flex_1()
            .bg(app_theme.background)
            .child(
                div()
                    .flex_1()
                    .px_3()
                    .pt_4()
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child(Input::new(&self.input).appearance(false)),
            )
            .child(self.render_toolbar(cx, false))
    }

    fn render_thread(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_basis(relative(if self.composer_expanded { 0.2 } else { 0.8 }))
            .min_h(px(96.))
            .overflow_y_scrollbar()
            .px_2()
            .pt_4()
            .pb_6()
            .gap_5()
            .children(
                self.messages
                    .iter()
                    .enumerate()
                    .map(|(index, message)| match message.role {
                        MessageRole::User => self.render_user_message(index, &message.content, cx),
                        MessageRole::Assistant => {
                            self.render_assistant_message(index, &message.content, window, cx)
                        }
                    }),
            )
    }

    fn render_user_message(
        &self,
        index: usize,
        content: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();

        div()
            .id(("ai-user-message", index))
            .w_full()
            .min_h(px(44.))
            .px_3()
            .py_3()
            .rounded(px(6.))
            .border_1()
            .border_color(palette::border(app_theme))
            .bg(palette::surface_strong(app_theme))
            .text_size(px(12.))
            .text_color(palette::text_strong(app_theme))
            .overflow_hidden()
            .child(content.to_string())
            .into_any_element()
    }

    fn render_assistant_message(
        &self,
        index: usize,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        let muted_foreground = palette::muted(app_theme);
        let is_finished = content != THINKING_MESSAGE;

        v_flex()
            .id(("ai-assistant-message", index))
            .px_3()
            .mt_1()
            .gap_4()
            .child(self.render_assistant_content(index, content, window, cx))
            .when(is_finished, |parent| {
                parent.child(
                    h_flex()
                        .justify_end()
                        .gap_1()
                        .text_color(muted_foreground.opacity(0.7))
                        .child(self.render_message_action(
                            index,
                            MessageAction::Like,
                            IconName::ThumbsUp,
                            content,
                            self.messages[index].feedback == Some(MessageFeedback::Like),
                            cx,
                        ))
                        .child(self.render_message_action(
                            index,
                            MessageAction::Dislike,
                            IconName::ThumbsDown,
                            content,
                            self.messages[index].feedback == Some(MessageFeedback::Dislike),
                            cx,
                        ))
                        .child(self.render_message_action(
                            index,
                            MessageAction::Copy,
                            IconName::Copy,
                            content,
                            false,
                            cx,
                        ))
                        .child(self.render_message_action(
                            index,
                            MessageAction::Retry,
                            IconName::Redo2,
                            content,
                            false,
                            cx,
                        ))
                        .child(self.render_message_action(
                            index,
                            MessageAction::Continue,
                            IconName::ArrowUp,
                            content,
                            false,
                            cx,
                        )),
                )
            })
            .into_any_element()
    }

    fn render_assistant_content(
        &self,
        index: usize,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        if content == THINKING_MESSAGE {
            return h_flex()
                .items_center()
                .gap_2()
                .py_1()
                .text_color(palette::muted(app_theme))
                .child(
                    div()
                        .w(px(18.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(palette::border_soft(app_theme))
                        .child(Icon::new(IconName::Bot).size_3()),
                )
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .small()
                        .color(palette::muted(app_theme)),
                )
                .into_any_element();
        }

        div()
            .w_full()
            .text_color(palette::muted(app_theme))
            .child(
                TextView::markdown(
                    ("ai-assistant-content", index),
                    content.to_string(),
                    window,
                    cx,
                )
                .selectable(true),
            )
            .into_any_element()
    }

    fn render_message_action(
        &self,
        index: usize,
        action: MessageAction,
        icon: IconName,
        content: &str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let copied_text = content.to_string();
        let icon_color = if active {
            cx.theme().success
        } else {
            palette::muted_soft(cx.theme())
        };

        Button::new((action.id(), index))
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |this, _, _, cx| match action {
                MessageAction::Like => this.set_message_feedback(index, MessageFeedback::Like, cx),
                MessageAction::Dislike => {
                    this.set_message_feedback(index, MessageFeedback::Dislike, cx)
                }
                MessageAction::Copy => {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied_text.clone()));
                }
                MessageAction::Retry => this.retry_assistant_message(index, cx),
                MessageAction::Continue => this.continue_assistant_message(cx),
            }))
            .child(
                div()
                    .w(px(20.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .hover(|style| style.bg(palette::hover(cx.theme())))
                    .child(Icon::new(icon).size_3().text_color(icon_color)),
            )
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();

        v_flex()
            .flex_basis(relative(if self.composer_expanded { 0.8 } else { 0.2 }))
            .min_h(px(120.))
            .border_t_1()
            .border_color(palette::border(app_theme))
            .bg(app_theme.background)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .m_2()
                    .px_3()
                    .py_3()
                    .pr_8()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(palette::surface(app_theme))
                    .text_size(px(12.))
                    .text_color(palette::text(app_theme))
                    .child(Input::new(&self.input).appearance(false))
                    .child(
                        Button::new("toggle-composer-expanded")
                            .ghost()
                            .xsmall()
                            .absolute()
                            .top(px(8.))
                            .right(px(6.))
                            .icon(
                                Icon::new(if self.composer_expanded {
                                    IconName::Minimize
                                } else {
                                    IconName::Maximize
                                })
                                .size_3(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_composer_expanded(cx);
                            })),
                    ),
            )
            .child(self.render_toolbar(cx, true))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>, active_thread: bool) -> impl IntoElement {
        h_flex()
            .items_center()
            .h(px(36.))
            .flex_none()
            .px_2()
            .gap_1()
            .child(self.render_toolbar_button("add-context", IconName::Plus, cx))
            .child(self.render_toolbar_button("web-context", IconName::Globe, cx))
            .child(div().flex_1())
            .child(self.render_context_indicator(cx))
            .child(self.render_mode_button("Write", true, cx))
            .child(self.render_model_button(cx))
            .child(
                Button::new("send-message")
                    .ghost()
                    .xsmall()
                    .disabled(self.loading)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.send_message(window, cx);
                    }))
                    .child(
                        div()
                            .w(px(22.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .bg(if active_thread {
                                color_stop()
                            } else {
                                cx.theme().muted.opacity(0.45)
                            })
                            .text_color(palette::text(cx.theme()))
                            .child(
                                Icon::new(if self.loading {
                                    IconName::LoaderCircle
                                } else {
                                    IconName::ArrowUp
                                })
                                .size_4(),
                            ),
                    ),
            )
    }

    fn render_context_indicator(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let usage = self.context_usage(cx);
        let theme = cx.theme();
        let input_limit = usage
            .input_limit
            .map(|limit| format_token_count(limit as u64))
            .unwrap_or_else(|| "-".to_string());
        let output_limit = usage
            .output_limit
            .map(|limit| format_token_count(limit as u64))
            .unwrap_or_else(|| "-".to_string());
        let input_line = format!(
            "Input: {} / {}",
            format_token_count(usage.input_used as u64),
            input_limit
        );
        let output_line = format!(
            "Output: {} / {}",
            format_token_count(usage.output_used as u64),
            output_limit
        );
        let tooltip_input_line = input_line.clone();
        let tooltip_output_line = output_line.clone();

        div()
            .id("ai-context-usage")
            .flex_none()
            .child(
                h_flex()
                    .items_center()
                    .justify_center()
                    .w(px(24.))
                    .h(px(24.))
                    .rounded_full()
                    .text_color(palette::muted(theme))
                    .hover(|style| style.bg(palette::hover(theme)))
                    .child(Icon::new(IconName::Info).size_3()),
            )
            .tooltip(move |window, cx| {
                let input_line = tooltip_input_line.clone();
                let output_line = tooltip_output_line.clone();
                Tooltip::element(move |_, cx| {
                    v_flex()
                        .gap_1()
                        .min_w(px(132.))
                        .text_size(px(11.))
                        .text_color(palette::muted(cx.theme()))
                        .child(
                            div()
                                .text_color(palette::text_strong(cx.theme()))
                                .child("Context"),
                        )
                        .child(input_line.clone())
                        .child(output_line.clone())
                })
                .build(window, cx)
            })
    }

    fn render_toolbar_button(
        &self,
        id: &'static str,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(id)
            .ghost()
            .xsmall()
            .text_color(palette::muted(cx.theme()))
            .child(
                div()
                    .w(px(22.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .hover(|style| style.bg(palette::hover(cx.theme())))
                    .child(Icon::new(icon).size_4()),
            )
    }

    fn render_mode_button(
        &self,
        label: &'static str,
        dropdown: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app_theme = cx.theme();

        h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .h(px(24.))
            .rounded(px(4.))
            .text_size(px(11.))
            .text_color(palette::muted(app_theme))
            .hover(|style| style.bg(palette::hover(app_theme)))
            .child(label)
            .when(dropdown, |parent| {
                parent.child(
                    Icon::new(IconName::ChevronDown)
                        .size_3()
                        .text_color(palette::muted(app_theme)),
                )
            })
    }

    fn render_model_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .max_w(px(240.))
            .flex_none()
            .child(
                Button::new("model-picker")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model_picker_open = !this.model_picker_open;
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .h(px(24.))
                            .px_2()
                            .rounded(px(4.))
                            .max_w(px(240.))
                            .text_size(px(12.))
                            .text_color(palette::text_strong(cx.theme()))
                            .hover(|style| style.bg(palette::hover(cx.theme())))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(self.selected_model.model.clone()),
                            )
                            .child(
                                Icon::new(IconName::ChevronDown)
                                    .size_3()
                                    .text_color(palette::muted(cx.theme())),
                            ),
                    ),
            )
            .when(self.model_picker_open, |parent| {
                parent.child(self.render_model_picker(cx))
            })
    }

    fn render_model_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let groups = self.filtered_model_groups(cx);
        let rows = groups
            .into_iter()
            .flat_map(|(title, items)| {
                let title_el = div()
                    .px_2()
                    .py_1()
                    .text_size(px(13.))
                    .text_color(palette::muted(cx.theme()))
                    .child(title)
                    .into_any_element();
                std::iter::once(title_el)
                    .chain(
                        items
                            .into_iter()
                            .map(|item| self.render_model_item(item, cx)),
                    )
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let background = cx.theme().background;
        let border = palette::border(cx.theme());

        v_flex()
            .absolute()
            .right_0()
            .bottom(px(30.))
            .w(px(320.))
            .max_h(px(360.))
            .bg(background)
            .border_1()
            .border_color(border)
            .rounded(px(6.))
            .shadow_md()
            .child(
                div()
                    .h(px(40.))
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(border)
                    .child(Input::new(&self.model_search).appearance(false)),
            )
            .child(
                v_flex()
                    .max_h(px(268.))
                    .overflow_y_scrollbar()
                    .p_1()
                    .children(rows),
            )
            .child(
                div()
                    .px_1()
                    .pt_1()
                    .pb_1()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .id("configure-ai-providers")
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .h(px(26.))
                            .w_full()
                            .rounded(px(4.))
                            .bg(cx.theme().muted.opacity(0.12))
                            .hover(|style| style.bg(cx.theme().muted.opacity(0.20)))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.model_picker_open = false;
                                    cx.emit(AiChatEvent::OpenProviders);
                                    window.dispatch_action(Box::new(OpenAiProviders), cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .min_w(px(76.))
                                    .text_right()
                                    .text_size(px(13.))
                                    .text_color(palette::text_strong(cx.theme()))
                                    .child("Configure"),
                            )
                            .child(
                                div()
                                    .min_w(px(76.))
                                    .text_size(px(11.))
                                    .text_color(palette::muted(cx.theme()))
                                    .child("Alt-Shift-C"),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_model_item(&self, item: ModelSelectItem, cx: &mut Context<Self>) -> AnyElement {
        let selected = item.selection == self.selected_model;
        let selection = item.selection.clone();

        h_flex()
            .items_center()
            .gap_2()
            .h(px(28.))
            .px_2()
            .rounded(px(4.))
            .bg(if selected {
                cx.theme().muted.opacity(0.26)
            } else {
                cx.theme().transparent
            })
            .hover(|style| style.bg(palette::hover(cx.theme())))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.selected_model = selection.clone();
                    this.ai_settings.agent.default_model = Some(selection.clone());
                    if let Err(err) = this.ai_settings.save(AiSettings::default_config_path()) {
                        this.settings_status = Some(err.to_string());
                    }
                    this.model_picker_open = false;
                    cx.notify();
                }),
            )
            .child(
                Icon::new(IconName::Bot)
                    .size_3()
                    .text_color(palette::muted(cx.theme())),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(13.))
                    .child(item.label),
            )
            .when(selected, |parent| {
                parent.child(
                    Icon::new(IconName::Check)
                        .size_3()
                        .text_color(cx.theme().success),
                )
            })
            .into_any_element()
    }

    fn filtered_model_groups(&self, cx: &mut Context<Self>) -> Vec<(String, Vec<ModelSelectItem>)> {
        let query = self.model_search.read(cx).text().to_string().to_lowercase();
        model_groups(&self.ai_settings)
            .into_iter()
            .filter_map(|(title, items)| {
                let items = items
                    .into_iter()
                    .filter(|item| {
                        query.is_empty()
                            || item.label.to_lowercase().contains(&query)
                            || item.selection.provider.to_lowercase().contains(&query)
                            || item.selection.model.to_lowercase().contains(&query)
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    None
                } else {
                    Some((title, items))
                }
            })
            .collect()
    }
}

impl EventEmitter<AiChatEvent> for AiChatPanel {}

impl Render for AiChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_panel(window, cx)
    }
}

fn model_groups(settings: &AiSettings) -> Vec<(String, Vec<ModelSelectItem>)> {
    settings
        .language_models
        .openai_compatible
        .iter()
        .filter_map(|(provider, settings)| {
            let items = settings
                .available_models
                .iter()
                .map(|model| ModelSelectItem {
                    selection: ModelSelection {
                        provider: provider.clone(),
                        model: model.name.clone(),
                    },
                    label: model
                        .display_name
                        .clone()
                        .unwrap_or_else(|| model.name.clone()),
                })
                .collect::<Vec<_>>();

            if items.is_empty() {
                None
            } else {
                Some((provider.clone(), items))
            }
        })
        .collect()
}

fn first_custom_model(settings: &AiSettings) -> Option<ModelSelectItem> {
    model_groups(settings)
        .into_iter()
        .flat_map(|(_, items)| items)
        .next()
}

fn custom_model_exists(settings: &AiSettings, selection: &ModelSelection) -> bool {
    settings
        .language_models
        .openai_compatible
        .get(&selection.provider)
        .is_some_and(|provider| {
            provider
                .available_models
                .iter()
                .any(|model| model.name == selection.model)
        })
}

fn estimate_tokens(text: &str) -> usize {
    let mut cjk_chars = 0usize;
    let mut ascii_chars = 0usize;
    let mut other_chars = 0usize;

    for ch in text.chars().filter(|ch| !ch.is_whitespace()) {
        let code = ch as u32;
        if (0x4E00..=0x9FFF).contains(&code)
            || (0x3400..=0x4DBF).contains(&code)
            || (0x3040..=0x30FF).contains(&code)
            || (0xAC00..=0xD7AF).contains(&code)
        {
            cjk_chars += 1;
        } else if ch.is_ascii() {
            ascii_chars += 1;
        } else {
            other_chars += 1;
        }
    }

    let estimated = cjk_chars + other_chars + ascii_chars.div_ceil(4);
    estimated.max(1)
}

fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let value = count as f64 / 1_000_000.0;
        format!("{value:.1}m").replace(".0m", "m")
    } else if count >= 1_000 {
        let value = count as f64 / 1_000.0;
        format!("{value:.1}k").replace(".0k", "k")
    } else {
        count.to_string()
    }
}

fn color_stop() -> Hsla {
    hsla(0.99, 0.46, 0.430, 1.0)
}

use crate::ui::palette;
use binance_tools::{
    ai::{
        AiSettings, ChatMessage as CoreChatMessage, ModelSelection,
        send_chat_with_model_streaming_blocking,
    },
    db::ai_threads::{
        AiChatThreadMetadata, delete_ai_chat_thread_blocking, list_ai_chat_threads_blocking,
        load_ai_chat_thread_blocking, save_ai_chat_thread_blocking,
    },
};
use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    spinner::Spinner,
    text::TextView,
    tooltip::Tooltip,
    v_flex,
};
use std::{
    sync::mpsc::{self, TryRecvError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

actions!(ai_chat, [ToggleAiChat, OpenAiProviders]);

const THINKING_MESSAGE: &str = "Thinking...";
const THREAD_DATA_VERSION: u8 = 1;
pub const THREADS_SIDEBAR_WIDTH: f32 = 252.0;
const AI_HEADER_HEIGHT: f32 = 34.0;
const ASK_MODE_SYSTEM_PROMPT: &str = "Mode: Ask. Answer the user's question or analysis request directly. Do not draft files, edit code, or perform write-oriented actions unless the user explicitly asks.";

pub enum AiChatEvent {
    OpenProviders,
    OpenRules,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ChatMessage {
    role: MessageRole,
    content: String,
    request_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule_snapshot: Option<RuleSnapshot>,
    feedback: Option<MessageFeedback>,
    failed: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RuleSnapshot {
    context_key: String,
    label: String,
    content: String,
    rule_updated_at: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    fn tooltip(self) -> &'static str {
        match self {
            Self::Like => "点赞",
            Self::Dislike => "点踩",
            Self::Copy => "复制",
            Self::Retry => "重新生成",
            Self::Continue => "继续生成",
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ThreadData {
    version: u8,
    messages: Vec<ChatMessage>,
}

pub struct AiChatPanel {
    visible: bool,
    active_thread_id: Option<String>,
    messages: Vec<ChatMessage>,
    input: Entity<InputState>,
    thread_search: Entity<InputState>,
    model_search: Entity<InputState>,
    selected_model: ModelSelection,
    threads: Vec<AiChatThreadMetadata>,
    threads_sidebar_visible: bool,
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
                .placeholder("Message the crypto Agent - @ to include context")
                .default_value("")
        });
        let model_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Select an option...")
                .default_value("")
        });
        let thread_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search threads...")
                .default_value("")
        });
        let _subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input_event),
            cx.subscribe_in(&thread_search, window, Self::on_thread_search_event),
        ];
        let (threads, thread_status) = match list_ai_chat_threads_blocking() {
            Ok(threads) => (threads, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let settings_status = settings_status.or(thread_status);

        Self {
            visible: false,
            active_thread_id: None,
            messages: Vec::new(),
            input,
            thread_search,
            model_search,
            selected_model,
            threads,
            threads_sidebar_visible: false,
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

    pub fn threads_sidebar_visible(&self) -> bool {
        self.threads_sidebar_visible
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
        rule_context: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }

        let (prompt, rule_snapshot) =
            prompt_with_rule_context(prompt.trim(), rule_context.as_ref());
        if prompt.is_empty() {
            return;
        }
        if !self.has_configured_selected_model() {
            self.clear_error_state();
            self.settings_status =
                Some("Configure a provider API key before starting a chat.".to_string());
            self.visible = true;
            cx.notify();
            return;
        }

        self.visible = true;
        self.load_latest_thread_if_empty(cx);
        self.ensure_active_thread();
        self.clear_error_state();
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: display_content,
            request_content: Some(prompt),
            rule_snapshot,
            feedback: None,
            failed: false,
        });
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: THINKING_MESSAGE.to_string(),
            request_content: None,
            rule_snapshot: None,
            feedback: None,
            failed: false,
        });
        self.save_current_thread(cx);

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

    fn on_thread_search_event(
        &mut self,
        _: &Entity<InputState>,
        _: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }

        let text = self.input.read(cx).text().to_string().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !self.has_configured_selected_model() {
            self.clear_error_state();
            self.settings_status =
                Some("Configure a provider API key before starting a chat.".to_string());
            cx.notify();
            return;
        }

        let selected_model = self.selected_model();
        let settings = self.ai_settings.clone();

        self.ensure_active_thread();
        self.clear_error_state();
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: text,
            request_content: None,
            rule_snapshot: None,
            feedback: None,
            failed: false,
        });
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: THINKING_MESSAGE.to_string(),
            request_content: None,
            rule_snapshot: None,
            feedback: None,
            failed: false,
        });
        self.save_current_thread(cx);

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
                                    this.settings_status = Some(error.clone());
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
                                this.save_current_thread(cx);
                                cx.notify();
                            });
                            break;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            done = true;
                            _ = this.update(cx, |this, cx| {
                                this.loading = false;
                                this.settings_status =
                                    Some("AI 流式响应已中断，请点击重试。".to_string());
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
        self.save_current_thread(cx);
        cx.notify();
    }

    fn retry_latest_failed_message(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self
            .messages
            .iter()
            .rposition(|message| message.role == MessageRole::Assistant && message.failed)
        else {
            return;
        };
        self.settings_status = None;
        self.retry_assistant_message(index, cx);
    }

    fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        if self.clear_error_state() {
            self.save_current_thread(cx);
        }
        cx.notify();
    }

    fn clear_error_state(&mut self) -> bool {
        let previous_len = self.messages.len();
        let had_status = self.settings_status.take().is_some();
        self.messages
            .retain(|message| !(message.role == MessageRole::Assistant && message.failed));
        had_status || self.messages.len() != previous_len
    }

    fn visible_error_status(&self) -> Option<String> {
        self.settings_status.clone().or_else(|| {
            self.messages
                .iter()
                .rev()
                .find(|message| message.role == MessageRole::Assistant && message.failed)
                .map(|message| message.content.clone())
        })
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
            rule_snapshot: None,
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
            rule_snapshot: None,
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

    fn stop_generation(&mut self, cx: &mut Context<Self>) {
        if !self.loading {
            return;
        }

        self._send_task = Task::ready(());
        self.loading = false;
        if self.messages.last().is_some_and(|message| {
            message.role == MessageRole::Assistant && message.content == THINKING_MESSAGE
        }) {
            self.messages.pop();
        }
        self.save_current_thread(cx);
        cx.notify();
    }

    fn new_thread(&mut self, cx: &mut Context<Self>) {
        self.save_current_thread(cx);
        self.active_thread_id = None;
        self.messages.clear();
        self.model_picker_open = false;
        self.composer_expanded = false;
        cx.notify();
    }

    fn toggle_threads_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opening = !self.threads_sidebar_visible;
        self.threads_sidebar_visible = opening;
        if self.threads_sidebar_visible {
            self.reload_threads();
        }
        resize_window_for_threads_sidebar(opening, window);
        cx.notify();
    }

    fn ensure_active_thread(&mut self) -> String {
        self.active_thread_id
            .get_or_insert_with(new_thread_id)
            .clone()
    }

    fn save_current_thread(&mut self, cx: &mut Context<Self>) {
        let Some(thread_id) = self.active_thread_id.clone() else {
            return;
        };
        let messages = self
            .messages
            .iter()
            .filter(|message| {
                !(message.role == MessageRole::Assistant && message.content == THINKING_MESSAGE)
            })
            .cloned()
            .collect::<Vec<_>>();
        if messages.is_empty() {
            return;
        }

        let title = thread_title(&messages);
        let selected_model_json = match serde_json::to_string(&self.selected_model) {
            Ok(json) => json,
            Err(err) => {
                self.settings_status = Some(err.to_string());
                cx.notify();
                return;
            }
        };
        let data = match serde_json::to_value(ThreadData {
            version: THREAD_DATA_VERSION,
            messages,
        }) {
            Ok(data) => data,
            Err(err) => {
                self.settings_status = Some(err.to_string());
                cx.notify();
                return;
            }
        };

        match save_ai_chat_thread_blocking(&thread_id, &title, &selected_model_json, &data) {
            Ok(()) => {
                self.settings_status = None;
                self.reload_threads();
            }
            Err(err) => {
                self.settings_status = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn reload_threads(&mut self) {
        match list_ai_chat_threads_blocking() {
            Ok(threads) => {
                self.threads = threads;
            }
            Err(err) => {
                self.settings_status = Some(err.to_string());
            }
        }
    }

    fn load_latest_thread_if_empty(&mut self, cx: &mut Context<Self>) {
        if self.active_thread_id.is_some() || !self.messages.is_empty() || self.loading {
            return;
        }

        self.reload_threads();
        let Some(thread_id) = self.threads.first().map(|thread| thread.id.clone()) else {
            return;
        };

        self.load_thread(thread_id, cx);
    }

    fn load_thread(&mut self, thread_id: String, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.save_current_thread(cx);

        match load_ai_chat_thread_blocking(&thread_id) {
            Ok(Some(thread)) => {
                match serde_json::from_value::<ThreadData>(thread.data) {
                    Ok(data) => {
                        self.messages = data.messages;
                        self.active_thread_id = Some(thread.metadata.id);
                    }
                    Err(err) => {
                        self.settings_status = Some(err.to_string());
                        cx.notify();
                        return;
                    }
                }

                match serde_json::from_str::<ModelSelection>(&thread.metadata.selected_model_json) {
                    Ok(selection) => {
                        self.selected_model = selection.clone();
                        self.ai_settings.agent.default_model = Some(selection);
                    }
                    Err(err) => {
                        self.settings_status = Some(err.to_string());
                    }
                }
                self.model_picker_open = false;
                self.visible = true;
                self.reload_threads();
            }
            Ok(None) => {
                self.settings_status = Some("Thread not found".to_string());
                self.reload_threads();
            }
            Err(err) => {
                self.settings_status = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn delete_thread(&mut self, thread_id: String, cx: &mut Context<Self>) {
        if let Err(err) = delete_ai_chat_thread_blocking(&thread_id) {
            self.settings_status = Some(err.to_string());
        }
        if self.active_thread_id.as_deref() == Some(thread_id.as_str()) {
            self.active_thread_id = None;
            self.messages.clear();
        }
        self.reload_threads();
        cx.notify();
    }

    fn selected_model(&self) -> ModelSelection {
        self.selected_model.clone()
    }

    fn has_configured_selected_model(&self) -> bool {
        custom_model_exists(&self.ai_settings, &self.selected_model)
    }

    fn core_history(&self) -> Vec<CoreChatMessage> {
        let mut history = vec![CoreChatMessage::system(ASK_MODE_SYSTEM_PROMPT)];
        history.extend(self.messages.iter().filter_map(|message| {
            match message.role {
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
            }
        }));
        history
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
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(palette::border(cx.theme()))
                    .when(self.threads_sidebar_visible, |parent| {
                        parent.child(self.render_threads_sidebar(cx))
                    })
                    .child(self.render_chat_column(window, cx)),
            )
            .into_any_element()
    }

    fn render_chat_column(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(self.render_header(cx))
            .child(self.render_chat_body(window, cx))
    }

    fn render_error_banner(&self, status: String, cx: &mut Context<Self>) -> impl IntoElement {
        let copied_status = status.clone();
        let has_retry = self
            .messages
            .iter()
            .any(|message| message.role == MessageRole::Assistant && message.failed);

        div().w_full().px_3().py_3().child(
            v_flex()
                .w_full()
                .px_3()
                .py_3()
                .gap_2()
                .rounded(px(2.))
                .border_1()
                .border_color(error_card_border())
                .bg(error_card_background())
                .text_size(px(12.))
                .text_color(error_card_text())
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .child(
                            Icon::new(IconName::CircleX)
                                .size_3()
                                .text_color(error_card_icon()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .text_size(px(12.))
                                .font_semibold()
                                .text_color(error_card_text())
                                .child("API Error"),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .when(has_retry, |parent| {
                                    parent.child(
                                        Button::new("retry-api-error")
                                            .ghost()
                                            .xsmall()
                                            .label("Retry")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.retry_latest_failed_message(cx);
                                            })),
                                    )
                                })
                                .child(
                                    Button::new("copy-api-error")
                                        .ghost()
                                        .xsmall()
                                        .child(
                                            div()
                                                .w(px(22.))
                                                .h(px(22.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .text_color(error_card_muted_text())
                                                .hover(|style| style.bg(error_card_hover()))
                                                .child(Icon::new(IconName::Copy).size_3()),
                                        )
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copied_status.clone(),
                                            ));
                                        }),
                                )
                                .child(
                                    Button::new("dismiss-api-error")
                                        .ghost()
                                        .xsmall()
                                        .child(
                                            div()
                                                .w(px(22.))
                                                .h(px(22.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .text_color(error_card_muted_text())
                                                .hover(|style| style.bg(error_card_hover()))
                                                .child(Icon::new(IconName::Close).size_3()),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.dismiss_error(cx);
                                        })),
                                ),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .pl(px(20.))
                        .pr_1()
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(error_card_muted_text())
                        .child(status),
                ),
        )
    }

    fn render_chat_body(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .when(self.messages.is_empty(), |parent| {
                parent.child(self.render_empty_stage(cx))
            })
            .when(!self.messages.is_empty(), |parent| {
                parent
                    .child(self.render_thread(window, cx))
                    .when_some(self.visible_error_status(), |parent, status| {
                        parent.child(self.render_error_banner(status, cx))
                    })
                    .child(self.render_composer(cx))
            })
    }

    fn render_threads_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self
            .thread_search
            .read(cx)
            .text()
            .to_string()
            .to_lowercase();
        let rows = self
            .threads
            .iter()
            .filter(|thread| query.is_empty() || thread.title.to_lowercase().contains(&query))
            .enumerate()
            .map(|(index, thread)| self.render_thread_row(index, thread, cx))
            .collect::<Vec<_>>();
        let app_theme = cx.theme();

        v_flex()
            .w(px(THREADS_SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(palette::border(app_theme))
            .bg(palette::surface(app_theme))
            .child(
                div()
                    .h(px(AI_HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .px_2()
                    .border_b_1()
                    .border_color(palette::border(app_theme))
                    .child(Input::new(&self.thread_search).appearance(false)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .p_1()
                    .when(rows.is_empty(), |parent| {
                        parent.child(
                            div()
                                .px_2()
                                .py_3()
                                .text_size(px(11.))
                                .text_color(palette::muted(app_theme))
                                .child("No threads"),
                        )
                    })
                    .children(rows),
            )
    }

    fn render_thread_row(
        &self,
        index: usize,
        thread: &AiChatThreadMetadata,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        let thread_id = thread.id.clone();
        let delete_thread_id = thread.id.clone();
        let active = self.active_thread_id.as_deref() == Some(thread.id.as_str());

        h_flex()
            .id(("ai-thread-row", index))
            .w_full()
            .items_start()
            .gap_1()
            .px_2()
            .py_2()
            .rounded(px(4.))
            .bg(if active {
                app_theme.muted.opacity(0.20)
            } else {
                app_theme.transparent
            })
            .hover(|style| style.bg(palette::hover(app_theme)))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.load_thread(thread_id.clone(), cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                Icon::new(IconName::Bot)
                    .size_3()
                    .text_color(palette::muted(app_theme)),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(palette::text_strong(app_theme))
                            .child(thread.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette::muted(app_theme))
                            .child(relative_time(&thread.updated_at)),
                    ),
            )
            .child(
                Button::new(("delete-thread", index))
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_thread(delete_thread_id.clone(), cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .w(px(18.))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .hover(|style| style.bg(palette::hover(cx.theme())))
                            .child(Icon::new(IconName::Delete).size_3()),
                    ),
            )
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
            .unwrap_or_else(|| "Crypto Agent".to_string());

        h_flex()
            .items_center()
            .justify_between()
            .h(px(AI_HEADER_HEIGHT))
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
                    .child(div().min_w_0().truncate().child(title)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when(has_messages, |parent| {
                        parent.child(self.render_header_button("new-thread", IconName::Plus, cx))
                    })
                    .child(self.render_header_button("expand-thread", IconName::ChevronUp, cx))
                    .child(self.render_more_menu_button(cx)),
            )
    }

    fn render_more_menu_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let panel_for_rules = panel.clone();
        let panel_for_settings = panel.clone();
        Button::new("more-thread")
            .ghost()
            .xsmall()
            .text_size(px(13.))
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
                    .child(Icon::new(IconName::Ellipsis).size_4()),
            )
            .dropdown_menu(move |menu, _, _| {
                menu.item(Self::menu_label_item("Rules").on_click({
                    let panel = panel_for_rules.clone();
                    move |_, _, cx| {
                        _ = panel.update(cx, |this, cx| {
                            this.model_picker_open = false;
                            cx.emit(AiChatEvent::OpenRules);
                        });
                    }
                }))
                .item(Self::menu_label_item("Profiles"))
                .item(Self::menu_label_item("Settings").on_click({
                    let panel = panel_for_settings.clone();
                    move |_, window, cx| {
                        _ = panel.update(cx, |this, cx| {
                            this.model_picker_open = false;
                            cx.emit(AiChatEvent::OpenProviders);
                        });
                        window.dispatch_action(Box::new(OpenAiProviders), cx);
                    }
                }))
                .separator()
                .item(Self::menu_label_item("Toggle Threads Sidebar").on_click({
                    let panel = panel.clone();
                    move |_, window, cx| {
                        _ = panel.update(cx, |this, cx| {
                            this.toggle_threads_sidebar(window, cx);
                        });
                    }
                }))
                .item(Self::menu_label_item("Reauthenticate"))
                .min_w(px(220.))
            })
    }

    fn menu_label_item(label: &'static str) -> PopupMenuItem {
        PopupMenuItem::element(move |_, _| {
            div()
                .h(px(28.))
                .flex()
                .items_center()
                .px_2()
                .text_size(px(12.))
                .child(label)
        })
    }

    fn render_header_button(
        &self,
        id: &'static str,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(id)
            .ghost()
            .xsmall()
            .text_size(px(13.))
            .text_color(palette::muted(cx.theme()))
            .when(id == "new-thread", |button| {
                button.on_click(cx.listener(|this, _, _, cx| {
                    this.new_thread(cx);
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
        let background = app_theme.background;
        let border = palette::border(app_theme);
        let muted = palette::muted(app_theme);

        v_flex()
            .flex_1()
            .bg(background)
            .child(div().flex_1())
            .when_some(self.visible_error_status(), |parent, status| {
                parent.child(self.render_error_banner(status, cx))
            })
            .child(
                div()
                    .h(px(120.))
                    .flex_none()
                    .border_t_1()
                    .border_color(border)
                    .px_3()
                    .pt_4()
                    .text_size(px(12.))
                    .text_color(muted)
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
                    .filter(|(_, message)| {
                        !(message.role == MessageRole::Assistant && message.failed)
                    })
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
        let failed = self.messages[index].failed;

        v_flex()
            .id(("ai-assistant-message", index))
            .px_3()
            .mt_1()
            .gap_4()
            .child(self.render_assistant_content(index, content, window, cx))
            .when(is_finished && !failed, |parent| {
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
        if self.messages[index].failed {
            return div().into_any_element();
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
            .tooltip(action.tooltip())
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
        let background = app_theme.background;
        let border = palette::border(app_theme);
        let text = palette::text(app_theme);

        v_flex()
            .flex_basis(relative(if self.composer_expanded { 0.8 } else { 0.2 }))
            .min_h(px(120.))
            .border_t_1()
            .border_color(border)
            .bg(background)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .m_2()
                    .px_3()
                    .py_3()
                    .pr_8()
                    .bg(background)
                    .text_size(px(12.))
                    .text_color(text)
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
        let can_send = active_thread && !self.input.read(cx).text().to_string().trim().is_empty();

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
            .child(self.render_mode_button("Ask", true, cx))
            .child(self.render_model_button(cx))
            .child(
                Button::new("send-message")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.loading {
                            this.stop_generation(cx);
                        } else {
                            this.send_message(window, cx);
                        }
                    }))
                    .child(
                        div()
                            .w(px(22.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .bg(if self.loading {
                                cx.theme().transparent
                            } else if can_send {
                                color_stop()
                            } else {
                                cx.theme().muted.opacity(0.45)
                            })
                            .text_color(if self.loading {
                                color_stop()
                            } else {
                                palette::text(cx.theme())
                            })
                            .when(self.loading, |parent| {
                                parent.child(
                                    Spinner::new()
                                        .icon(Icon::new(IconName::LoaderCircle))
                                        .small()
                                        .color(color_stop()),
                                )
                            })
                            .when(!self.loading, |parent| {
                                parent.child(Icon::new(IconName::ArrowUp).size_4())
                            }),
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
        let has_model = self.has_configured_selected_model();
        let label = if has_model {
            self.selected_model.model.clone()
        } else {
            "Select a model".to_string()
        };

        div()
            .relative()
            .max_w(px(180.))
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
                            .text_color(if has_model {
                                palette::text_strong(cx.theme())
                            } else {
                                palette::muted(cx.theme())
                            })
                            .hover(|style| style.bg(palette::hover(cx.theme())))
                            .child(div().min_w_0().truncate().child(label))
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
        let rows = self
            .filtered_model_groups(cx)
            .into_iter()
            .flat_map(|(_, items)| items)
            .map(|item| self.render_model_item(item, cx))
            .collect::<Vec<_>>();
        let background = cx.theme().background;
        let border = palette::border(cx.theme());

        v_flex()
            .absolute()
            .right_0()
            .bottom(px(32.))
            .w(px(240.))
            .max_h(px(286.))
            .bg(background)
            .border_1()
            .border_color(border)
            .rounded(px(6.))
            .child(
                div()
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .px_2()
                    .border_b_1()
                    .border_color(border)
                    .child(Input::new(&self.model_search).appearance(false)),
            )
            .child(
                v_flex()
                    .max_h(px(240.))
                    .min_h(px(32.))
                    .overflow_y_scrollbar()
                    .px_1()
                    .py_1()
                    .children(rows),
            )
            .child(
                div().border_t_1().border_color(border).child(
                    h_flex()
                        .id("add-ai-model")
                        .items_center()
                        .gap_2()
                        .h(px(32.))
                        .w_full()
                        .px_2()
                        .hover(|style| style.bg(palette::hover(cx.theme())))
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
                            Icon::new(IconName::Plus)
                                .size_3()
                                .text_color(palette::muted(cx.theme())),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_size(px(13.))
                                .text_color(palette::text_strong(cx.theme()))
                                .child("Add model"),
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
            .gap_1()
            .h(px(32.))
            .px_2()
            .rounded(px(4.))
            .bg(if selected {
                cx.theme().muted.opacity(0.32)
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
                    this.save_current_thread(cx);
                    this.model_picker_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(14.))
                    .text_color(palette::text_strong(cx.theme()))
                    .child(item.label),
            )
            .when(selected, |parent| {
                parent.child(
                    Icon::new(IconName::Check)
                        .size_3()
                        .text_color(cx.theme().accent),
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
        .providers()
        .into_iter()
        .filter(|(provider, provider_settings)| {
            provider_has_configured_key(provider, provider_settings)
        })
        .filter_map(|(provider, settings)| {
            let items = settings
                .available_models
                .iter()
                .map(|model| ModelSelectItem {
                    selection: ModelSelection {
                        provider: provider.to_string(),
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
                Some((provider.to_string(), items))
            }
        })
        .collect()
}

fn provider_has_configured_key(
    provider_id: &str,
    _settings: &binance_tools::ai::ProviderSettings,
) -> bool {
    let stored_key = binance_tools::db::ai::load_ai_provider_key_blocking(provider_id)
        .ok()
        .flatten();
    stored_key.is_some_and(|key| {
        key.enabled
            && matches!(
                key.key_source,
                binance_tools::db::ai::AiProviderKeySource::Db
            )
            && key
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    })
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
        .provider(&selection.provider)
        .is_some_and(|provider| {
            provider_has_configured_key(&selection.provider, provider)
                && provider
                    .available_models
                    .iter()
                    .any(|model| model.name == selection.model)
        })
}

fn prompt_with_rule_context(
    prompt: &str,
    rule_context: Option<&(String, String)>,
) -> (String, Option<RuleSnapshot>) {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return (String::new(), None);
    }

    let Some((context_key, label)) = rule_context else {
        return (prompt.to_string(), None);
    };

    match binance_tools::db::ai_rules::load_ai_rule_blocking(context_key) {
        Ok(Some(rule)) if rule.enabled && !rule.content.trim().is_empty() => {
            let content = rule.content.trim().to_string();
            let label = if rule.label.trim().is_empty() {
                label.clone()
            } else {
                rule.label.clone()
            };
            let prompt =
                format!("{prompt}\n\nCurrent page custom AI analysis rules ({label}):\n{content}");
            let snapshot = RuleSnapshot {
                context_key: rule.context_key,
                label,
                content,
                rule_updated_at: Some(rule.updated_at),
            };
            (prompt, Some(snapshot))
        }
        Ok(_) => (prompt.to_string(), None),
        Err(err) => (
            format!(
                "{prompt}\n\nCurrent page custom AI analysis rules ({label}) failed to load. Ignore custom rules and continue analysis: {err}"
            ),
            None,
        ),
    }
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

fn new_thread_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("thread-{millis}")
}

fn thread_title(messages: &[ChatMessage]) -> String {
    let title = messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .unwrap_or("Untitled thread");

    let mut clipped = title.chars().take(40).collect::<String>();
    if title.chars().count() > 40 {
        clipped.push_str("...");
    }
    clipped
}

fn relative_time(timestamp: &str) -> String {
    let parsed = NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|naive| Utc.from_utc_datetime(&naive))
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(timestamp)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });
    let Some(updated_at) = parsed else {
        return timestamp.to_string();
    };

    let elapsed = Local::now().with_timezone(&Utc) - updated_at;
    let minutes = elapsed.num_minutes().max(0);
    if minutes < 1 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 60 * 24 {
        format!("{}h", minutes / 60)
    } else if minutes < 60 * 24 * 7 {
        format!("{}d", minutes / (60 * 24))
    } else {
        format!("{}w", minutes / (60 * 24 * 7))
    }
}

fn resize_window_for_threads_sidebar(opening: bool, window: &mut Window) {
    let mut viewport_size = window.viewport_size();
    if opening {
        viewport_size.width += px(THREADS_SIDEBAR_WIDTH);
    } else {
        viewport_size.width = (viewport_size.width - px(THREADS_SIDEBAR_WIDTH)).max(px(640.));
    }
    window.resize(viewport_size);
}

fn color_stop() -> Hsla {
    hsla(0.99, 0.46, 0.430, 1.0)
}

fn error_card_background() -> Hsla {
    hsla(0.0, 0.72, 0.94, 1.0)
}

fn error_card_border() -> Hsla {
    hsla(0.0, 0.62, 0.78, 1.0)
}

fn error_card_text() -> Hsla {
    hsla(0.0, 0.18, 0.18, 1.0)
}

fn error_card_muted_text() -> Hsla {
    hsla(0.0, 0.12, 0.28, 1.0)
}

fn error_card_icon() -> Hsla {
    hsla(0.0, 0.72, 0.48, 1.0)
}

fn error_card_hover() -> Hsla {
    hsla(0.0, 0.58, 0.86, 1.0)
}

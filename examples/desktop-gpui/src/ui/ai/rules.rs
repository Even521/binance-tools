use crate::ui::palette;
use binance_tools::db::ai_rules::{
    AiRuleMetadata, RULE_FORMAT_MARKDOWN, RULE_FORMAT_TEXT, list_ai_rules_blocking,
    load_ai_rule_blocking, save_ai_rule_with_format_blocking, set_ai_rule_enabled_blocking,
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Theme,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};

pub enum AiRulesEvent {
    Close,
}

pub struct AiRulesPage {
    rules: Vec<AiRuleMetadata>,
    selected_key: Option<String>,
    saved_content: String,
    enabled: bool,
    rule_format: RuleFormat,
    status: Option<String>,
    error: Option<String>,
    search_input: Entity<InputState>,
    key_input: Entity<InputState>,
    label_input: Entity<InputState>,
    content_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleFormat {
    Text,
    Markdown,
}

impl RuleFormat {
    fn as_db(self) -> &'static str {
        match self {
            Self::Text => RULE_FORMAT_TEXT,
            Self::Markdown => RULE_FORMAT_MARKDOWN,
        }
    }

    fn from_db(value: &str) -> Self {
        if value.eq_ignore_ascii_case(RULE_FORMAT_MARKDOWN) || value.eq_ignore_ascii_case("md") {
            Self::Markdown
        } else {
            Self::Text
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Markdown => "Markdown",
        }
    }
}

impl AiRulesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (rules, error) = match list_ai_rules_blocking() {
            Ok(rules) => (rules, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let selected_key = rules.first().map(|rule| rule.context_key.clone());
        let loaded_rule = selected_key
            .as_deref()
            .and_then(|key| load_ai_rule_blocking(key).ok().flatten());

        let key_value = loaded_rule
            .as_ref()
            .map(|rule| rule.context_key.clone())
            .unwrap_or_default();
        let label_value = loaded_rule
            .as_ref()
            .map(|rule| rule.label.clone())
            .unwrap_or_default();
        let content_value = loaded_rule
            .as_ref()
            .map(|rule| rule.content.clone())
            .unwrap_or_default();
        let enabled = loaded_rule.as_ref().is_none_or(|rule| rule.enabled);
        let rule_format = loaded_rule
            .as_ref()
            .map(|rule| RuleFormat::from_db(&rule.format))
            .unwrap_or(RuleFormat::Text);

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search...")
                .default_value("")
        });
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("context_key")
                .default_value(key_value)
        });
        let label_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Rule name")
                .default_value(label_value)
        });
        let content_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(16, 48)
                .placeholder("Write rule instructions...")
                .default_value(content_value.clone())
        });
        let _subscriptions = vec![
            cx.subscribe_in(&search_input, window, Self::on_input_event),
            cx.subscribe_in(&key_input, window, Self::on_input_event),
            cx.subscribe_in(&label_input, window, Self::on_input_event),
            cx.subscribe_in(&content_input, window, Self::on_input_event),
        ];

        Self {
            rules,
            selected_key,
            saved_content: content_value,
            enabled,
            rule_format,
            status: None,
            error,
            search_input,
            key_input,
            label_input,
            content_input,
            _subscriptions,
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        _: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }

    fn new_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = unique_rule_key(&self.rules);
        self.selected_key = None;
        self.saved_content.clear();
        self.enabled = true;
        self.rule_format = RuleFormat::Markdown;
        self.error = None;
        self.status = Some("New rule".to_string());
        self.key_input
            .update(cx, |input, cx| input.set_value(key, window, cx));
        self.label_input
            .update(cx, |input, cx| input.set_value("New Rule", window, cx));
        self.content_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn select_rule(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        match load_ai_rule_blocking(&key) {
            Ok(Some(rule)) => {
                self.selected_key = Some(rule.context_key.clone());
                self.saved_content = rule.content.clone();
                self.enabled = rule.enabled;
                self.rule_format = RuleFormat::from_db(&rule.format);
                self.error = None;
                self.status = None;
                self.key_input.update(cx, |input, cx| {
                    input.set_value(rule.context_key, window, cx)
                });
                self.label_input
                    .update(cx, |input, cx| input.set_value(rule.label, window, cx));
                self.content_input
                    .update(cx, |input, cx| input.set_value(rule.content, window, cx));
            }
            Ok(None) => {
                self.error = Some("Rule not found".to_string());
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn save_rule(&mut self, cx: &mut Context<Self>) {
        let context_key = input_text(&self.key_input, cx);
        let label = input_text(&self.label_input, cx);
        let content = self.content_input.read(cx).text().to_string();

        if context_key.is_empty() {
            self.error = Some("Context key cannot be empty".to_string());
            cx.notify();
            return;
        }
        if label.is_empty() {
            self.error = Some("Rule name cannot be empty".to_string());
            cx.notify();
            return;
        }
        if self
            .selected_key
            .as_ref()
            .is_some_and(|selected| selected != &context_key)
        {
            self.error = Some("Existing rule context key cannot be changed".to_string());
            cx.notify();
            return;
        }

        if let Err(err) = save_ai_rule_with_format_blocking(
            &context_key,
            &label,
            self.rule_format.as_db(),
            &content,
        ) {
            self.error = Some(err.to_string());
            cx.notify();
            return;
        }
        if let Err(err) = set_ai_rule_enabled_blocking(&context_key, self.enabled) {
            self.error = Some(err.to_string());
            cx.notify();
            return;
        }

        self.reload_rules();
        self.selected_key = Some(context_key);
        self.saved_content = content;
        self.error = None;
        self.status = Some("Saved".to_string());
        cx.notify();
    }

    fn reload_rules(&mut self) {
        match list_ai_rules_blocking() {
            Ok(rules) => {
                self.rules = rules;
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_rules();
        if let Some(key) = self.selected_key.clone() {
            self.select_rule(key, window, cx);
        } else {
            cx.notify();
        }
    }

    fn reset_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.saved_content.clone();
        self.content_input
            .update(cx, |input, cx| input.set_value(content, window, cx));
        self.status = Some("Reverted unsaved text".to_string());
        cx.notify();
    }

    fn toggle_enabled(&mut self, cx: &mut Context<Self>) {
        self.enabled = !self.enabled;
        if let Some(key) = self.selected_key.clone() {
            match set_ai_rule_enabled_blocking(&key, self.enabled) {
                Ok(()) => {
                    self.reload_rules();
                    self.status = Some(if self.enabled { "Enabled" } else { "Disabled" }.into());
                    self.error = None;
                }
                Err(err) => {
                    self.error = Some(err.to_string());
                }
            }
        }
        cx.notify();
    }

    fn set_rule_format(&mut self, rule_format: RuleFormat, cx: &mut Context<Self>) {
        self.rule_format = rule_format;
        cx.notify();
    }

    fn filtered_rules(&self, cx: &mut Context<Self>) -> Vec<AiRuleMetadata> {
        let query = self
            .search_input
            .read(cx)
            .text()
            .to_string()
            .trim()
            .to_lowercase();
        self.rules
            .iter()
            .filter(|rule| {
                query.is_empty()
                    || rule.context_key.to_lowercase().contains(&query)
                    || rule.label.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();

        h_flex()
            .items_center()
            .h(px(34.))
            .px_2()
            .border_b_1()
            .border_color(palette::border(app_theme))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_size(px(13.)).child("Rules")),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("copy-rule")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Copy).size_4())
                            .tooltip("复制规则")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let content = this.content_input.read(cx).text().to_string();
                                cx.write_to_clipboard(ClipboardItem::new_string(content));
                            })),
                    )
                    .child(
                        Button::new("refresh-rules")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Redo2).size_4())
                            .tooltip("刷新")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.refresh(window, cx);
                            })),
                    ),
            )
            .child(
                Button::new("close-ai-rules")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Close).size_4())
                    .tooltip("Close")
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(AiRulesEvent::Close);
                        cx.stop_propagation();
                    })),
            )
            .into_any_element()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let rules = self.filtered_rules(cx);
        let app_theme = cx.theme();
        let (built_in, custom): (Vec<_>, Vec<_>) = rules
            .into_iter()
            .partition(|rule| is_builtin_rule(&rule.context_key));
        let selected_key = self.selected_key.clone();

        v_flex()
            .w(px(258.))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(palette::border(app_theme))
            .bg(app_theme.group_box.opacity(0.45))
            .p_2()
            .gap_2()
            .child(
                Button::new("new-ai-rule")
                    .outline()
                    .xsmall()
                    .icon(IconName::Plus)
                    .label("New Rule")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_rule(window, cx);
                    })),
            )
            .child(
                h_flex()
                    .items_center()
                    .h(px(32.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .child(Input::new(&self.search_input).appearance(false)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(self.render_rule_group(
                        "Built-in Rules",
                        built_in,
                        selected_key.clone(),
                        true,
                        cx,
                    ))
                    .child(self.render_rule_group("Custom Rules", custom, selected_key, false, cx)),
            )
            .into_any_element()
    }

    fn render_rule_group(
        &self,
        title: &'static str,
        rules: Vec<AiRuleMetadata>,
        selected_key: Option<String>,
        show_info: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .w_full()
            .when(!rules.is_empty() || show_info, |parent| {
                parent.child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .h(px(24.))
                        .px_1()
                        .text_size(px(11.))
                        .text_color(palette::muted(app_theme))
                        .child(title)
                        .when(show_info, |parent| {
                            parent.child(
                                Button::new("built-in-rules-info")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::Info).size_3())
                                    .tooltip(
                                        "内置规则由程序初始化，可编辑内容，不会被启动迁移覆盖",
                                    ),
                            )
                        }),
                )
            })
            .children(
                rules
                    .into_iter()
                    .map(|rule| self.render_rule_row(rule, selected_key.clone(), cx)),
            )
            .into_any_element()
    }

    fn render_rule_row(
        &self,
        rule: AiRuleMetadata,
        selected_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        let selected = selected_key.as_deref() == Some(rule.context_key.as_str());
        let key = rule.context_key.clone();
        let label = if rule.label.trim().is_empty() {
            rule.context_key.clone()
        } else {
            rule.label.clone()
        };

        h_flex()
            .items_center()
            .gap_2()
            .h(px(32.))
            .px_2()
            .rounded(px(4.))
            .cursor_pointer()
            .bg(if selected {
                app_theme.muted.opacity(0.38)
            } else {
                app_theme.transparent
            })
            .hover(|style| style.bg(palette::hover(app_theme)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.select_rule(key.clone(), window, cx);
                }),
            )
            .child(
                div()
                    .w(px(7.))
                    .h(px(7.))
                    .rounded_full()
                    .bg(if rule.enabled {
                        app_theme.success
                    } else {
                        palette::muted_soft(app_theme)
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(palette::text(app_theme))
                    .child(label),
            )
            .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected_title = input_text(&self.label_input, cx);
        let format_selector = self.render_format_selector(cx);
        let app_theme = cx.theme();
        let title = if selected_title.is_empty() {
            "New Rule".to_string()
        } else {
            selected_title
        };

        v_flex()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .h(px(42.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(18.))
                            .text_color(palette::text_strong(app_theme))
                            .child(title),
                    )
                    .child(
                        Button::new("toggle-rule-enabled")
                            .outline()
                            .xsmall()
                            .label(if self.enabled { "Enabled" } else { "Disabled" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_enabled(cx))),
                    )
                    .child(
                        Button::new("save-ai-rule")
                            .primary()
                            .xsmall()
                            .label("Save")
                            .on_click(cx.listener(|this, _, _, cx| this.save_rule(cx))),
                    ),
            )
            .when_some(self.error.clone(), |parent, error| {
                parent.child(self.render_status_card(error, true, app_theme))
            })
            .when_some(self.status.clone(), |parent, status| {
                parent.child(self.render_status_card(status, false, app_theme))
            })
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .child(self.render_labeled_input("Name", &self.label_input, app_theme))
                    .child(self.render_labeled_input("Context key", &self.key_input, app_theme))
                    .child(format_selector)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(app_theme))
                                    .child("Rule"),
                            )
                            .child(
                                div()
                                    .min_h(px(420.))
                                    .w_full()
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(palette::border(app_theme))
                                    .bg(app_theme.background)
                                    .p_2()
                                    .child(Input::new(&self.content_input).appearance(false)),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("reset-ai-rule")
                                    .outline()
                                    .xsmall()
                                    .label("Revert")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reset_editor(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("copy-ai-rule")
                                    .outline()
                                    .xsmall()
                                    .label("Copy")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let content =
                                            this.content_input.read(cx).text().to_string();
                                        cx.write_to_clipboard(ClipboardItem::new_string(content));
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_format_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child("Format"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(self.render_format_option(RuleFormat::Text, cx))
                    .child(self.render_format_option(RuleFormat::Markdown, cx)),
            )
            .into_any_element()
    }

    fn render_format_option(&self, rule_format: RuleFormat, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let selected = self.rule_format == rule_format;

        Button::new(match rule_format {
            RuleFormat::Text => "rule-format-text",
            RuleFormat::Markdown => "rule-format-markdown",
        })
        .outline()
        .xsmall()
        .label(rule_format.label())
        .bg(if selected {
            app_theme.muted.opacity(0.36)
        } else {
            app_theme.transparent
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_rule_format(rule_format, cx);
        }))
        .into_any_element()
    }

    fn render_labeled_input(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        app_theme: &Theme,
    ) -> AnyElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child(label),
            )
            .child(
                h_flex()
                    .items_center()
                    .h(px(32.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .child(Input::new(input).appearance(false)),
                    ),
            )
            .into_any_element()
    }

    fn render_status_card(&self, message: String, error: bool, app_theme: &Theme) -> AnyElement {
        div()
            .mx_3()
            .mb_2()
            .p_2()
            .rounded(px(4.))
            .border_1()
            .border_color(if error {
                palette::error_border()
            } else {
                app_theme.success.opacity(0.35)
            })
            .bg(if error {
                palette::error_background()
            } else {
                app_theme.success.opacity(0.10)
            })
            .text_size(px(12.))
            .text_color(if error {
                palette::error_text()
            } else {
                palette::text(app_theme)
            })
            .child(message)
            .into_any_element()
    }
}

impl EventEmitter<AiRulesEvent> for AiRulesPage {}

impl Render for AiRulesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();

        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .occlude()
            .bg(gpui::black().opacity(0.24))
            .child(
                v_flex()
                    .absolute()
                    .top(px(42.))
                    .left(px(70.))
                    .right(px(70.))
                    .bottom(px(42.))
                    .min_w(px(760.))
                    .min_h(px(520.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .shadow_md()
                    .bg(app_theme.background)
                    .child(self.render_header(cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .overflow_hidden()
                            .child(self.render_sidebar(cx))
                            .child(self.render_editor(cx)),
                    ),
            )
    }
}

fn is_builtin_rule(key: &str) -> bool {
    matches!(
        key,
        "market_products" | "square_ai_market" | "square_fallback_reasons"
    )
}

fn unique_rule_key(rules: &[AiRuleMetadata]) -> String {
    let mut index = 1;
    loop {
        let key = format!("custom_rule_{index}");
        if !rules.iter().any(|rule| rule.context_key == key) {
            return key;
        }
        index += 1;
    }
}

fn input_text(input: &Entity<InputState>, cx: &mut Context<AiRulesPage>) -> String {
    input.read(cx).text().to_string().trim().to_string()
}

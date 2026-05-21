use crate::ui::palette;
use binance_tools::ai::{
    AiSettings, ApiFormat, ModelCapabilities, ModelDefinition, ProviderSettings,
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::env;

actions!(ai_providers, [CloseAiProviders, ToggleAiProvidersMaximized]);

pub enum AiProvidersEvent {
    Close,
    Saved,
    ToggleMaximized,
}

pub struct AiProvidersPage {
    settings: AiSettings,
    error: Option<String>,
    form_error: Option<String>,
    expanded_provider: Option<String>,
    editing_provider: Option<String>,
    view: AiProvidersView,
    maximized: bool,
    provider_name_input: Entity<InputState>,
    api_url_input: Entity<InputState>,
    api_key_input: Entity<InputState>,
    model_name_input: Entity<InputState>,
    max_completion_tokens_input: Entity<InputState>,
    max_output_tokens_input: Entity<InputState>,
    max_tokens_input: Entity<InputState>,
    supports_tools: bool,
    supports_images: bool,
    supports_parallel_tool_calls: bool,
    supports_prompt_cache_key: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiProvidersView {
    List,
    AddProvider,
}

impl AiProvidersPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (settings, error) = match AiSettings::load_default() {
            Ok(settings) => (settings, None),
            Err(err) => (AiSettings::default(), Some(err.to_string())),
        };
        let provider_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("OpenAI")
                .default_value("OpenAI")
        });
        let api_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.openai.com/v1")
                .default_value("https://api.openai.com/v1")
        });
        let api_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API Key")
                .default_value("")
        });
        let model_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. gpt-5, claude-opus-4, gemini-2.5-pro")
                .default_value("")
        });
        let max_completion_tokens_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("200000")
                .default_value("200000")
        });
        let max_output_tokens_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("32000")
                .default_value("32000")
        });
        let max_tokens_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("200000")
                .default_value("200000")
        });

        Self {
            settings,
            error,
            form_error: None,
            expanded_provider: None,
            editing_provider: None,
            view: AiProvidersView::List,
            maximized: false,
            provider_name_input,
            api_url_input,
            api_key_input,
            model_name_input,
            max_completion_tokens_input,
            max_output_tokens_input,
            max_tokens_input,
            supports_tools: true,
            supports_images: false,
            supports_parallel_tool_calls: false,
            supports_prompt_cache_key: false,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        match AiSettings::load_default() {
            Ok(settings) => {
                self.settings = settings;
                self.error = None;
            }
            Err(err) => {
                self.settings = AiSettings::default();
                self.error = Some(err.to_string());
            }
        }
        cx.notify();
    }

    pub fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.maximized = maximized;
        cx.notify();
    }

    fn toggle_provider(&mut self, provider: String, cx: &mut Context<Self>) {
        if self.expanded_provider.as_deref() == Some(provider.as_str()) {
            self.expanded_provider = None;
        } else {
            self.expanded_provider = Some(provider);
        }
        cx.notify();
    }

    fn open_add_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form_error = None;
        self.editing_provider = None;
        self.provider_name_input
            .update(cx, |input, cx| input.set_value("OpenAI", window, cx));
        self.api_url_input.update(cx, |input, cx| {
            input.set_value("https://api.openai.com/v1", window, cx)
        });
        self.api_key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.model_name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.max_completion_tokens_input
            .update(cx, |input, cx| input.set_value("200000", window, cx));
        self.max_output_tokens_input
            .update(cx, |input, cx| input.set_value("32000", window, cx));
        self.max_tokens_input
            .update(cx, |input, cx| input.set_value("200000", window, cx));
        self.supports_tools = true;
        self.supports_images = false;
        self.supports_parallel_tool_calls = false;
        self.supports_prompt_cache_key = false;
        self.view = AiProvidersView::AddProvider;
        cx.notify();
    }

    fn close_add_provider(&mut self, cx: &mut Context<Self>) {
        self.form_error = None;
        self.editing_provider = None;
        self.view = AiProvidersView::List;
        cx.notify();
    }

    fn edit_provider(&mut self, provider_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self
            .settings
            .language_models
            .openai_compatible
            .get(&provider_id)
            .cloned()
        else {
            self.error = Some(format!("Provider '{provider_id}' does not exist"));
            cx.notify();
            return;
        };

        let model = settings.available_models.first();
        let api_url = settings.api_url.clone().unwrap_or_default();
        let api_key = strip_default_zero_padding(&settings.api_key.clone().unwrap_or_default());
        let model_name = model.map(|model| model.name.clone()).unwrap_or_default();
        let max_tokens = model
            .map(|model| model.max_tokens)
            .unwrap_or(settings.context_window.unwrap_or(200000))
            .to_string();
        let max_output_tokens = model
            .and_then(|model| model.max_output_tokens)
            .unwrap_or(32000)
            .to_string();
        let max_completion_tokens = model
            .and_then(|model| model.max_completion_tokens)
            .unwrap_or(200000)
            .to_string();

        self.provider_name_input
            .update(cx, |input, cx| input.set_value(&provider_id, window, cx));
        self.api_url_input
            .update(cx, |input, cx| input.set_value(api_url, window, cx));
        self.api_key_input
            .update(cx, |input, cx| input.set_value(api_key, window, cx));
        self.model_name_input
            .update(cx, |input, cx| input.set_value(model_name, window, cx));
        self.max_tokens_input
            .update(cx, |input, cx| input.set_value(max_tokens, window, cx));
        self.max_output_tokens_input.update(cx, |input, cx| {
            input.set_value(max_output_tokens, window, cx)
        });
        self.max_completion_tokens_input.update(cx, |input, cx| {
            input.set_value(max_completion_tokens, window, cx)
        });

        self.supports_tools = model.is_none_or(|model| model.supports_tools);
        self.supports_images = model.is_some_and(|model| model.supports_images);
        self.supports_parallel_tool_calls =
            model.is_some_and(|model| model.capabilities.parallel_tool_calls);
        self.supports_prompt_cache_key = model.is_some_and(|model| model.supports_prompt_cache_key);
        self.editing_provider = Some(provider_id);
        self.form_error = None;
        self.view = AiProvidersView::AddProvider;
        cx.notify();
    }

    fn delete_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let mut settings = self.settings.clone();
        if settings
            .language_models
            .openai_compatible
            .remove(&provider_id)
            .is_none()
        {
            self.error = Some(format!("Provider '{provider_id}' does not exist"));
            cx.notify();
            return;
        }

        if settings
            .agent
            .default_model
            .as_ref()
            .is_some_and(|selection| {
                normalized_provider_name(&selection.provider)
                    == normalized_provider_name(&provider_id)
            })
        {
            settings.agent.default_model = None;
        }

        if let Err(err) = settings.save(AiSettings::default_config_path()) {
            self.error = Some(err.to_string());
            cx.notify();
            return;
        }

        self.settings = settings;
        self.expanded_provider = None;
        self.error = None;
        cx.emit(AiProvidersEvent::Saved);
        cx.notify();
    }

    fn save_provider(&mut self, cx: &mut Context<Self>) {
        let provider_name = input_text(&self.provider_name_input, cx);
        let api_url = input_text(&self.api_url_input, cx);
        let api_key = strip_default_zero_padding(&input_text(&self.api_key_input, cx));
        let model_name = input_text(&self.model_name_input, cx);

        if provider_name.is_empty() {
            self.form_error = Some("Provider Name cannot be empty".to_string());
            cx.notify();
            return;
        }
        if api_url.is_empty() {
            self.form_error = Some("API URL cannot be empty".to_string());
            cx.notify();
            return;
        }
        if api_key.is_empty() {
            self.form_error = Some("API Key cannot be empty".to_string());
            cx.notify();
            return;
        }
        if model_name.is_empty() {
            self.form_error = Some("Model Name cannot be empty".to_string());
            cx.notify();
            return;
        }
        let is_same_edit = self.editing_provider.as_ref().is_some_and(|editing| {
            normalized_provider_name(editing) == normalized_provider_name(&provider_name)
        });
        if !is_same_edit && self.provider_name_exists(&provider_name) {
            self.form_error = Some(format!("Provider Name '{provider_name}' already exists"));
            cx.notify();
            return;
        }

        let max_tokens = match parse_u32_input("Max Tokens", &self.max_tokens_input, cx) {
            Ok(value) => value,
            Err(error) => {
                self.form_error = Some(error);
                cx.notify();
                return;
            }
        };
        let max_output_tokens =
            match parse_u32_input("Max Output Tokens", &self.max_output_tokens_input, cx) {
                Ok(value) => value,
                Err(error) => {
                    self.form_error = Some(error);
                    cx.notify();
                    return;
                }
            };
        let max_completion_tokens = match parse_u32_input(
            "Max Completion Tokens",
            &self.max_completion_tokens_input,
            cx,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.form_error = Some(error);
                cx.notify();
                return;
            }
        };

        let mut settings = self.settings.clone();
        if let Some(editing_provider) = &self.editing_provider {
            if normalized_provider_name(editing_provider)
                != normalized_provider_name(&provider_name)
            {
                settings
                    .language_models
                    .openai_compatible
                    .remove(editing_provider);
            }
        }
        let available_models = vec![ModelDefinition {
            name: model_name.clone(),
            display_name: Some(model_name),
            max_tokens,
            max_output_tokens: Some(max_output_tokens),
            max_completion_tokens: Some(max_completion_tokens),
            supports_tools: self.supports_tools,
            supports_images: self.supports_images,
            supports_thinking: false,
            supports_prompt_cache_key: self.supports_prompt_cache_key,
            capabilities: ModelCapabilities {
                chat_completions: true,
                tools: self.supports_tools,
                images: self.supports_images,
                parallel_tool_calls: self.supports_parallel_tool_calls,
            },
        }];

        settings.language_models.openai_compatible.insert(
            provider_name.clone(),
            ProviderSettings {
                api_url: Some(api_url),
                api_key: Some(api_key),
                api_key_env: None,
                api_format: ApiFormat::OpenAiChat,
                available_models,
                ..ProviderSettings::default()
            },
        );

        if let Err(err) = settings.save(AiSettings::default_config_path()) {
            self.form_error = Some(err.to_string());
            cx.notify();
            return;
        }

        self.settings = settings;
        self.form_error = None;
        self.error = None;
        self.expanded_provider = Some(provider_name);
        self.editing_provider = None;
        self.view = AiProvidersView::List;
        cx.emit(AiProvidersEvent::Saved);
        cx.notify();
    }

    fn provider_name_exists(&self, name: &str) -> bool {
        let candidate = normalized_provider_name(name);
        self.provider_entries().into_iter().any(|entry| {
            normalized_provider_name(&entry.id) == candidate
                || normalized_provider_name(&entry.name) == candidate
        })
    }

    fn toggle_supports_tools(&mut self, cx: &mut Context<Self>) {
        self.supports_tools = !self.supports_tools;
        cx.notify();
    }

    fn toggle_supports_images(&mut self, cx: &mut Context<Self>) {
        self.supports_images = !self.supports_images;
        cx.notify();
    }

    fn toggle_supports_parallel_tool_calls(&mut self, cx: &mut Context<Self>) {
        self.supports_parallel_tool_calls = !self.supports_parallel_tool_calls;
        cx.notify();
    }

    fn toggle_supports_prompt_cache_key(&mut self, cx: &mut Context<Self>) {
        self.supports_prompt_cache_key = !self.supports_prompt_cache_key;
        cx.notify();
    }

    fn provider_entries(&self) -> Vec<ProviderEntry> {
        self.settings
            .language_models
            .openai_compatible
            .iter()
            .map(|(name, settings)| ProviderEntry::from_settings(name, name, settings))
            .collect()
    }

    fn render_provider_row(&self, entry: ProviderEntry, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let expanded = self.expanded_provider.as_deref() == Some(entry.id.as_str());
        let provider_id = entry.id.clone();

        v_flex()
            .w_full()
            .border_b_1()
            .border_color(app_theme.border.opacity(0.65))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .h(px(44.))
                    .px_1()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_provider(provider_id.clone(), cx);
                        }),
                    )
                    .child(
                        Icon::new(entry.icon.clone())
                            .size_4()
                            .text_color(palette::muted(app_theme)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .flex_1()
                            .child(div().text_size(px(14.)).child(entry.name.clone()))
                            .when_some(entry.badge.clone(), |parent, badge| {
                                parent.child(
                                    div()
                                        .px_1()
                                        .rounded(px(3.))
                                        .border_1()
                                        .border_color(palette::border(app_theme))
                                        .text_size(px(10.))
                                        .text_color(palette::muted(app_theme))
                                        .child(badge),
                                )
                            })
                            .when(entry.connected, |parent| {
                                parent.child(
                                    Icon::new(IconName::Check)
                                        .size_3()
                                        .text_color(app_theme.success),
                                )
                            }),
                    )
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        })
                        .size_4()
                        .text_color(palette::muted(app_theme)),
                    ),
            )
            .when(expanded, |parent| {
                parent.child(self.render_provider_details(&entry, cx))
            })
            .into_any_element()
    }

    fn render_provider_details(&self, entry: &ProviderEntry, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let api_url = entry
            .settings
            .as_ref()
            .and_then(|settings| settings.api_url.clone())
            .unwrap_or_else(|| "Managed by provider".to_string());
        let api_key_env = entry
            .settings
            .as_ref()
            .and_then(|settings| settings.api_key_env.clone())
            .unwrap_or_else(|| "None".to_string());
        let models = entry
            .settings
            .as_ref()
            .map(|settings| {
                settings
                    .available_models
                    .iter()
                    .map(|model| {
                        model
                            .display_name
                            .clone()
                            .unwrap_or_else(|| model.name.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|models| !models.is_empty())
            .unwrap_or_else(|| "No local model list".to_string());
        let edit_provider_id = entry.id.clone();
        let delete_provider_id = entry.id.clone();

        v_flex()
            .gap_2()
            .px_6()
            .pb_3()
            .text_size(px(12.))
            .text_color(palette::muted(app_theme))
            .child(format!("API URL: {api_url}"))
            .child(format!("API Key Env: {api_key_env}"))
            .child(format!("Models: {models}"))
            .child(format!(
                "Config file: {}",
                AiSettings::default_config_path().display()
            ))
            .child(
                h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new("edit-ai-provider")
                            .outline()
                            .xsmall()
                            .label("修改")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_provider(edit_provider_id.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new("delete-ai-provider")
                            .outline()
                            .xsmall()
                            .label("删除")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.delete_provider(delete_provider_id.clone(), cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_settings_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();

        h_flex()
            .items_center()
            .h(px(30.))
            .px_2()
            .border_b_1()
            .border_color(app_theme.border.opacity(0.65))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("close-ai-providers")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::ChevronLeft).size_4())
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(AiProvidersEvent::Close);
                                cx.stop_propagation();
                            })),
                    )
                    .child(div().text_size(px(13.)).child("Settings")),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("header-add-ai-provider")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Plus).size_4())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_add_provider(window, cx);
                            })),
                    )
                    .child(
                        Button::new("toggle-ai-providers-maximized")
                            .ghost()
                            .xsmall()
                            .icon(
                                Icon::new(if self.maximized {
                                    IconName::Minimize
                                } else {
                                    IconName::Maximize
                                })
                                .size_4(),
                            )
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(AiProvidersEvent::ToggleMaximized);
                                cx.stop_propagation();
                            })),
                    )
                    .child(
                        Button::new("ai-providers-more")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Ellipsis).size_4()),
                    ),
            )
            .into_any_element()
    }

    fn render_external_agents(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .px_5()
            .py_4()
            .gap_3()
            .border_b_1()
            .border_color(app_theme.border.opacity(0.65))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(div().text_size(px(18.)).child("External Agents"))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(palette::muted(app_theme))
                                    .child(
                                        "All agents connected through the Agent Client Protocol.",
                                    ),
                            ),
                    )
                    .child(
                        h_flex().w_full().justify_end().child(
                            Button::new("add-agent")
                                .outline()
                                .xsmall()
                                .icon(IconName::Plus)
                                .label("Add Agent"),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .h(px(34.))
                    .child(
                        Icon::new(IconName::SquareTerminal)
                            .size_4()
                            .text_color(app_theme.success),
                    )
                    .child(div().text_size(px(13.)).child("Codex CLI"))
                    .child(
                        div()
                            .px_1()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(palette::border(app_theme))
                            .text_size(px(10.))
                            .text_color(palette::muted(app_theme))
                            .child("A"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(app_theme))
                            .child("0.14.0"),
                    )
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .gap_3()
                            .text_color(palette::muted(app_theme))
                            .child(Icon::new(IconName::Redo2).size_4())
                            .child(Icon::new(IconName::Delete).size_4()),
                    ),
            )
            .into_any_element()
    }

    fn render_mcp_servers(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .px_5()
            .py_4()
            .gap_3()
            .border_b_1()
            .border_color(app_theme.border.opacity(0.65))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .child("Model Context Protocol (MCP) Servers"),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(palette::muted(app_theme))
                                    .child(
                                        "All MCP servers connected directly or via a Zed extension.",
                                    ),
                            ),
                    )
                    .child(
                        h_flex().w_full().justify_end().child(
                            Button::new("add-mcp-server")
                                .outline()
                                .xsmall()
                                .icon(IconName::Plus)
                                .label("Add Server"),
                        ),
                    ),
            )
            .child(
                div()
                    .h(px(52.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(app_theme.border.opacity(0.65))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .text_color(palette::muted(app_theme))
                    .child("No MCP servers added yet."),
            )
            .into_any_element()
    }

    fn render_llm_providers(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted_foreground = palette::muted(cx.theme());
        let border = cx.theme().border;
        let danger = cx.theme().danger;
        let danger_foreground = cx.theme().danger_foreground;
        let rows = self
            .provider_entries()
            .into_iter()
            .map(|entry| self.render_provider_row(entry, cx))
            .collect::<Vec<_>>();
        let has_rows = !rows.is_empty();

        v_flex()
            .px_5()
            .py_4()
            .gap_3()
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(div().text_size(px(18.)).child("LLM Providers"))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(muted_foreground)
                                    .child(
                                        "Add at least one provider to use AI-powered features with Zed's native agent.",
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("reload-ai-providers")
                                    .outline()
                                    .xsmall()
                                    .icon(IconName::Redo2)
                                    .label("Reload")
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            )
                            .child(
                                Button::new("add-provider")
                                    .outline()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .label("Add Provider")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_add_provider(window, cx)
                                    })),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .bg(danger.opacity(0.12))
                        .text_color(danger_foreground)
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .w_full()
                    .mt_2()
                    .border_t_1()
                    .border_color(border.opacity(0.65))
                    .when(!has_rows, |parent| {
                        parent.child(
                            div()
                                .h(px(52.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_b_1()
                                .border_color(border.opacity(0.65))
                                .text_size(px(12.))
                                .text_color(muted_foreground)
                                .child("No LLM providers added yet."),
                        )
                    })
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_settings_list(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .child(self.render_settings_header(cx))
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(self.render_external_agents(cx))
                    .child(self.render_mcp_servers(cx))
                    .child(self.render_llm_providers(cx)),
            )
            .into_any_element()
    }

    fn render_add_provider_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let background = cx.theme().background;
        let border = cx.theme().border;
        let muted_foreground = palette::muted(cx.theme());
        let danger = cx.theme().danger;
        let danger_foreground = cx.theme().danger_foreground;
        let editing = self.editing_provider.is_some();

        v_flex()
            .absolute()
            .top(px(18.))
            .left(px(20.))
            .w(px(540.))
            .max_h(px(560.))
            .rounded(px(6.))
            .bg(background)
            .border_1()
            .border_color(border.opacity(0.9))
            .shadow_md()
            .occlude()
            .child(
                v_flex()
                    .max_h(px(518.))
                    .overflow_y_scrollbar()
                    .p_4()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(div().text_size(px(17.)).child(if editing {
                                "Edit LLM Provider"
                            } else {
                                "Add LLM Provider"
                            }))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(muted_foreground)
                                    .child("This provider will use an OpenAI compatible API."),
                            ),
                    )
                    .when_some(self.form_error.clone(), |parent, error| {
                        parent.child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .p_3()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(danger.opacity(0.65))
                                .bg(danger.opacity(0.15))
                                .text_color(danger_foreground)
                                .child(Icon::new(IconName::TriangleAlert).size_4())
                                .child(div().text_size(px(12.)).child(error)),
                        )
                    })
                    .child(self.render_labeled_input(
                        "Provider Name",
                        &self.provider_name_input,
                        false,
                        cx,
                    ))
                    .child(self.render_labeled_input("API URL", &self.api_url_input, false, cx))
                    .child(self.render_labeled_input("API Key", &self.api_key_input, true, cx))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(div().text_size(px(13.)).child("Models"))
                                    .child(
                                        Button::new("add-provider-model")
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Plus)
                                            .label("Add Model"),
                                    ),
                            )
                            .child(self.render_model_form(cx)),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_start()
                    .gap_3()
                    .h(px(42.))
                    .px_4()
                    .border_t_1()
                    .border_color(border.opacity(0.7))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.close_add_provider(cx);
                                }),
                            )
                            .child(div().text_size(px(13.)).child("Cancel"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted_foreground)
                                    .child("Escape"),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.save_provider(cx);
                                }),
                            )
                            .child(div().text_size(px(13.)).child(if editing {
                                "Update Provider"
                            } else {
                                "Save Provider"
                            }))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted_foreground)
                                    .child("Enter"),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_labeled_input(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        secret: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .gap_1()
            .child(div().text_size(px(12.)).child(label))
            .child(
                h_flex()
                    .items_center()
                    .h(px(32.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(app_theme.border.opacity(0.8))
                    .bg(app_theme.background.opacity(0.65))
                    .child(
                        div()
                            .flex_1()
                            .px_2()
                            .child(Input::new(input).appearance(false)),
                    )
                    .when(secret, |parent| {
                        parent.child(
                            div()
                                .px_2()
                                .text_color(palette::muted(app_theme))
                                .child(Icon::new(IconName::Eye).size_4()),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_model_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();

        v_flex()
            .gap_3()
            .p_2()
            .rounded(px(4.))
            .border_1()
            .border_color(app_theme.border.opacity(0.8))
            .bg(app_theme.group_box.opacity(0.35))
            .child(self.render_labeled_input("Model Name", &self.model_name_input, false, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(self.render_labeled_input(
                        "Max Completion Tokens",
                        &self.max_completion_tokens_input,
                        false,
                        cx,
                    )))
                    .child(div().flex_1().child(self.render_labeled_input(
                        "Max Output Tokens",
                        &self.max_output_tokens_input,
                        false,
                        cx,
                    ))),
            )
            .child(self.render_labeled_input("Max Tokens", &self.max_tokens_input, false, cx))
            .child(self.render_checkbox(
                "Supports tools",
                self.supports_tools,
                |this, cx| this.toggle_supports_tools(cx),
                cx,
            ))
            .child(self.render_checkbox(
                "Supports images",
                self.supports_images,
                |this, cx| this.toggle_supports_images(cx),
                cx,
            ))
            .child(self.render_checkbox(
                "Supports parallel_tool_calls",
                self.supports_parallel_tool_calls,
                |this, cx| this.toggle_supports_parallel_tool_calls(cx),
                cx,
            ))
            .child(self.render_checkbox(
                "Supports prompt_cache_key",
                self.supports_prompt_cache_key,
                |this, cx| this.toggle_supports_prompt_cache_key(cx),
                cx,
            ))
            .into_any_element()
    }

    fn render_checkbox(
        &self,
        label: &'static str,
        checked: bool,
        on_toggle: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();

        h_flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    on_toggle(this, cx);
                }),
            )
            .child(
                div()
                    .w(px(14.))
                    .h(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(if checked {
                        app_theme.primary.opacity(0.18)
                    } else {
                        app_theme.transparent
                    })
                    .when(checked, |parent| {
                        parent.child(
                            Icon::new(IconName::Check)
                                .size_3()
                                .text_color(app_theme.primary),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(palette::muted(app_theme))
                    .child(label),
            )
            .into_any_element()
    }
}

impl EventEmitter<AiProvidersEvent> for AiProvidersPage {}

impl Render for AiProvidersPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(self.render_settings_list(cx))
            .when(self.view == AiProvidersView::AddProvider, |parent| {
                parent.child(self.render_add_provider_modal(cx))
            })
    }
}

#[derive(Clone)]
struct ProviderEntry {
    id: String,
    name: String,
    icon: IconName,
    badge: Option<String>,
    connected: bool,
    settings: Option<ProviderSettings>,
}

impl ProviderEntry {
    fn new(
        id: &str,
        name: &str,
        settings: Option<ProviderSettings>,
        badge: Option<&str>,
        connected: bool,
    ) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            icon: provider_icon(id),
            badge: badge.map(str::to_string),
            connected,
            settings,
        }
    }

    fn from_settings(id: &str, name: &str, settings: &ProviderSettings) -> Self {
        let connected = settings
            .api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || settings
                .api_key_env
                .as_deref()
                .is_some_and(|key| env::var(key).is_ok_and(|value| !value.trim().is_empty()))
            || settings
                .api_url
                .as_deref()
                .is_some_and(|url| url.contains("localhost") || url.contains("127.0.0.1"));

        Self::new(id, name, Some(settings.clone()), None, connected)
    }
}

fn provider_icon(id: &str) -> IconName {
    match id {
        "copilot_chat" => IconName::GitHub,
        "google" | "open_router" => IconName::Globe,
        "lmstudio" | "ollama" => IconName::Bot,
        "zed.dev" => IconName::Bot,
        _ => IconName::Bot,
    }
}

fn input_text(input: &Entity<InputState>, cx: &mut Context<AiProvidersPage>) -> String {
    input.read(cx).text().to_string().trim().to_string()
}

fn parse_u32_input(
    label: &'static str,
    input: &Entity<InputState>,
    cx: &mut Context<AiProvidersPage>,
) -> Result<u32, String> {
    let value = input_text(input, cx);
    value
        .parse::<u32>()
        .map_err(|_| format!("{label} must be a valid positive number"))
}

fn normalized_provider_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn strip_default_zero_padding(value: &str) -> String {
    let trimmed = value.trim();
    strip_long_zero_run(strip_long_zero_run(trimmed, true).as_str(), false)
}

fn strip_long_zero_run(value: &str, leading: bool) -> String {
    const MIN_PADDING_ZEROS: usize = 8;

    let zero_count = if leading {
        value.chars().take_while(|ch| *ch == '0').count()
    } else {
        value.chars().rev().take_while(|ch| *ch == '0').count()
    };

    if zero_count < MIN_PADDING_ZEROS || zero_count == value.chars().count() {
        return value.to_string();
    }

    if leading {
        value.chars().skip(zero_count).collect()
    } else {
        value
            .chars()
            .take(value.chars().count() - zero_count)
            .collect()
    }
}

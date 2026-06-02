use crate::ui::palette;
use binance_tools::ai::{
    AiSettings, ApiFormat, LanguageModelsSettings, ModelCapabilities, ModelDefinition,
    ProviderSettings,
};
use gpui::{actions, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Theme,
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
        self.provider_name_input.update(cx, |input, cx| {
            input.set_value("Custom Provider", window, cx)
        });
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
            .provider(&provider_id)
            .cloned()
        else {
            self.error = Some(format!("Provider '{provider_id}' does not exist"));
            cx.notify();
            return;
        };

        let model = settings.available_models.first();
        let api_url = settings.api_url.clone().unwrap_or_default();
        let api_key = binance_tools::db::ai::load_ai_provider_key_blocking(&provider_id)
            .ok()
            .flatten()
            .and_then(|key| key.api_key)
            .or_else(|| settings.api_key.clone())
            .map(|key| strip_default_zero_padding(&key))
            .unwrap_or_default();
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
        let is_builtin = is_builtin_provider(&provider_id);
        if !is_builtin
            && settings
                .language_models
                .openai_compatible
                .remove(&provider_id)
                .is_none()
        {
            self.error = Some(format!("Provider '{provider_id}' does not exist"));
            cx.notify();
            return;
        }

        if !is_builtin
            && settings
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

        let key_result = if is_builtin {
            binance_tools::db::ai::save_ai_provider_key_none_blocking(
                &provider_id,
                provider_display_name(&provider_id),
            )
        } else {
            binance_tools::db::ai::delete_ai_provider_key_blocking(&provider_id)
        };
        if let Err(err) = key_result {
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
        if self
            .editing_provider
            .as_ref()
            .is_some_and(|editing| is_builtin_provider(editing))
            && !is_same_edit
        {
            self.form_error = Some("Built-in Provider Name cannot be changed".to_string());
            cx.notify();
            return;
        }
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

        let provider_id = self
            .editing_provider
            .clone()
            .filter(|provider| is_builtin_provider(provider))
            .unwrap_or_else(|| provider_name.clone());
        let provider_settings = ProviderSettings {
            api_url: Some(api_url),
            api_key: None,
            api_key_env: None,
            api_format: self
                .settings
                .language_models
                .provider(&provider_id)
                .map(|settings| settings.api_format)
                .unwrap_or(ApiFormat::OpenAiChat),
            available_models,
            ..ProviderSettings::default()
        };
        if !set_builtin_provider_settings(&mut settings, &provider_id, provider_settings.clone()) {
            settings
                .language_models
                .openai_compatible
                .insert(provider_id.clone(), provider_settings);
        }

        if let Err(err) = settings.save(AiSettings::default_config_path()) {
            self.form_error = Some(err.to_string());
            cx.notify();
            return;
        }

        if let Err(err) = binance_tools::db::ai::save_ai_provider_api_key_blocking(
            &provider_id,
            &provider_name,
            &api_key,
        ) {
            self.form_error = Some(err.to_string());
            cx.notify();
            return;
        }

        self.settings = settings;
        self.form_error = None;
        self.error = None;
        self.expanded_provider = Some(provider_id);
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
            .providers()
            .into_iter()
            .map(|(name, settings)| {
                ProviderEntry::from_settings(name, provider_display_name(name), settings)
            })
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
        let stored_key = binance_tools::db::ai::load_ai_provider_key_blocking(&entry.id)
            .ok()
            .flatten();
        let credential = stored_key
            .as_ref()
            .map(|key| match key.key_source {
                binance_tools::db::ai::AiProviderKeySource::Db => {
                    if key
                        .api_key
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        "Configured in local database".to_string()
                    } else {
                        "Not configured".to_string()
                    }
                }
                binance_tools::db::ai::AiProviderKeySource::Env => key
                    .api_key_env
                    .as_ref()
                    .map(|env| {
                        if env::var(env).is_ok_and(|value| !value.trim().is_empty()) {
                            format!("Configured from {env}")
                        } else {
                            format!("Not configured ({env})")
                        }
                    })
                    .unwrap_or_else(|| "Not configured".to_string()),
                binance_tools::db::ai::AiProviderKeySource::None => "Not configured".to_string(),
            })
            .or_else(|| match stored_key {
                Some(_) => None,
                None => entry
                    .settings
                    .as_ref()
                    .and_then(|settings| settings.api_key_env.clone())
                    .map(|env| {
                        if env::var(&env).is_ok_and(|value| !value.trim().is_empty()) {
                            format!("Configured from {env}")
                        } else {
                            format!("Not configured ({env})")
                        }
                    }),
            })
            .unwrap_or_else(|| "Not configured".to_string());
        let key_store = if stored_key.is_some() {
            binance_tools::db::DEFAULT_DATABASE_PATH.to_string()
        } else {
            "Environment variable fallback".to_string()
        };
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
        let delete_label = if entry.built_in {
            "Clear Key"
        } else {
            "Delete"
        };

        v_flex()
            .gap_3()
            .px_6()
            .py_3()
            .text_size(px(12.))
            .text_color(palette::muted(app_theme))
            .child(provider_detail_row("API URL", api_url, app_theme))
            .child(provider_detail_row("Credential", credential, app_theme))
            .child(provider_detail_row("Models", models, app_theme))
            .child(provider_detail_row(
                "Config",
                AiSettings::default_config_path().display().to_string(),
                app_theme,
            ))
            .child(provider_detail_row("Key Store", key_store, app_theme))
            .child(
                h_flex()
                    .gap_2()
                    .pt_0()
                    .child(
                        Button::new("edit-ai-provider")
                            .outline()
                            .xsmall()
                            .label("Edit")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_provider(edit_provider_id.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new("delete-ai-provider")
                            .outline()
                            .xsmall()
                            .label(delete_label)
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
                            .disabled(true)
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
                                .label("Add Agent")
                                .disabled(true),
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
                                .label("Add Server")
                                .disabled(true),
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
            .top(px(36.))
            .left(px(12.))
            .right(px(12.))
            .max_h(px(600.))
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
                    .p_3()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_size(px(16.)).child(if editing {
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
                                    .child(div().text_size(px(13.)).child("Models")),
                            )
                            .child(self.render_model_form(cx)),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .min_h(px(46.))
                    .px_3()
                    .border_t_1()
                    .border_color(border.opacity(0.7))
                    .child(
                        Button::new("cancel-ai-provider")
                            .outline()
                            .xsmall()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_add_provider(cx);
                            })),
                    )
                    .child(
                        Button::new("save-ai-provider")
                            .primary()
                            .xsmall()
                            .label(if editing {
                                "Update Provider"
                            } else {
                                "Save Provider"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_provider(cx);
                            })),
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
            .w_full()
            .gap_1()
            .child(div().text_size(px(12.)).child(label))
            .child(
                h_flex()
                    .w_full()
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
            .child(self.render_labeled_input(
                "Max Completion Tokens",
                &self.max_completion_tokens_input,
                false,
                cx,
            ))
            .child(self.render_labeled_input(
                "Max Output Tokens",
                &self.max_output_tokens_input,
                false,
                cx,
            ))
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
    built_in: bool,
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
            built_in: is_builtin_provider(id),
            settings,
        }
    }

    fn from_settings(id: &str, name: &str, settings: &ProviderSettings) -> Self {
        let connected = provider_has_configured_key(id, settings);

        let badge = is_builtin_provider(id).then_some("Built-in");
        Self::new(id, name, Some(settings.clone()), badge, connected)
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

fn provider_has_configured_key(id: &str, _settings: &ProviderSettings) -> bool {
    let stored_key = binance_tools::db::ai::load_ai_provider_key_blocking(id)
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

fn provider_detail_row(label: &'static str, value: String, app_theme: &Theme) -> AnyElement {
    h_flex()
        .w_full()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(72.))
                .flex_none()
                .text_color(palette::muted_soft(app_theme))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .text_color(palette::muted(app_theme))
                .child(value),
        )
        .into_any_element()
}

fn provider_display_name(id: &str) -> &str {
    match id {
        "deepseek" => "DeepSeek",
        "openai" => "OpenAI",
        "open_router" => "OpenRouter",
        "ollama" => "Ollama",
        "lmstudio" => "LM Studio",
        "anthropic" => "Anthropic",
        "google" => "Google",
        name => name,
    }
}

fn is_builtin_provider(id: &str) -> bool {
    matches!(
        id,
        "deepseek" | "openai" | "open_router" | "ollama" | "lmstudio" | "anthropic" | "google"
    )
}

fn set_builtin_provider_settings(
    settings: &mut AiSettings,
    provider_id: &str,
    provider_settings: ProviderSettings,
) -> bool {
    let language_models: &mut LanguageModelsSettings = &mut settings.language_models;
    match provider_id {
        "deepseek" => language_models.deepseek = provider_settings,
        "openai" => language_models.openai = provider_settings,
        "open_router" => language_models.open_router = provider_settings,
        "ollama" => language_models.ollama = provider_settings,
        "lmstudio" => language_models.lmstudio = provider_settings,
        "anthropic" => language_models.anthropic = provider_settings,
        "google" => language_models.google = provider_settings,
        _ => return false,
    }
    true
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

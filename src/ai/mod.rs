//! Zed-style AI agent configuration and chat client.
//!
//! The configuration shape intentionally mirrors Zed's `agent` and
//! `language_models` settings enough for this project to share the same mental
//! model: select a provider/model, tune per-model parameters, and keep provider
//! endpoints separate from agent behavior.

use anyhow::{Context, anyhow};
use reqwest::{StatusCode, blocking::Client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Duration,
};

pub const DEFAULT_AI_CONFIG_PATH: &str = "config/ai.json";
const AI_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const AI_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub agent: AgentSettings,
    #[serde(default)]
    pub language_models: LanguageModelsSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            agent: AgentSettings::default(),
            language_models: LanguageModelsSettings::default(),
        }
    }
}

impl AiSettings {
    pub fn default_config_path() -> PathBuf {
        PathBuf::from(DEFAULT_AI_CONFIG_PATH)
    }

    pub fn load_default() -> anyhow::Result<Self> {
        let path = Self::default_config_path();
        let mut settings = Self::load(&path)?;
        if settings.migrate_inline_api_keys_to_default_db()? {
            settings.save(path)?;
        }
        Ok(settings)
    }

    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let text = fs::read_to_string(path)
            .with_context(|| format!("read AI config failed: {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parse AI config JSON failed: {}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create AI config directory failed: {}", parent.display())
            })?;
        }

        let text = serde_json::to_string_pretty(self).context("serialize AI config failed")?;
        fs::write(path, format!("{text}\n"))
            .with_context(|| format!("write AI config failed: {}", path.display()))
    }

    fn migrate_inline_api_keys_to_default_db(&mut self) -> anyhow::Result<bool> {
        let mut migrated = false;

        for (provider_id, provider_name, settings) in self.language_models.providers_mut() {
            let Some(api_key) = settings
                .api_key
                .as_deref()
                .map(strip_default_zero_padding)
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };

            crate::db::ai::save_ai_provider_api_key_blocking(
                &provider_id,
                &provider_name,
                &api_key,
            )?;
            settings.api_key = None;
            migrated = true;
        }

        Ok(migrated)
    }

    pub fn selected_model(&self) -> &ModelSelection {
        self.agent
            .default_model
            .as_ref()
            .unwrap_or(&self.agent.fallback_model)
    }

    pub fn selected_provider(&self) -> Option<&ProviderSettings> {
        self.provider_for_model(self.selected_model())
    }

    pub fn provider_for_model(&self, selection: &ModelSelection) -> Option<&ProviderSettings> {
        if let Some(provider) = self
            .language_models
            .openai_compatible
            .get(&selection.provider)
        {
            let model_matches = provider.available_models.is_empty()
                || provider
                    .available_models
                    .iter()
                    .any(|model| model.name == selection.model);
            if model_matches {
                return Some(provider);
            }
        }

        self.language_models.provider(&selection.provider)
    }

    pub fn configured_models(&self) -> Vec<ModelOption> {
        self.language_models
            .providers()
            .into_iter()
            .flat_map(|(provider, settings)| {
                settings
                    .available_models
                    .iter()
                    .map(move |model| ModelOption {
                        provider: provider.to_string(),
                        model: model.name.clone(),
                        display_name: model
                            .display_name
                            .clone()
                            .unwrap_or_else(|| model.name.clone()),
                    })
            })
            .collect()
    }

    fn model_parameters(&self, selection: &ModelSelection) -> ModelParameters {
        let mut merged = ModelParameters::default();

        for params in &self.agent.model_parameters {
            let provider_matches = params
                .provider
                .as_ref()
                .is_none_or(|provider| provider == &selection.provider);
            let model_matches = params
                .model
                .as_ref()
                .is_none_or(|model| model == &selection.model);

            if provider_matches && model_matches {
                if params.temperature.is_some() {
                    merged.temperature = params.temperature;
                }
                if params.max_output_tokens.is_some() {
                    merged.max_output_tokens = params.max_output_tokens;
                }
            }
        }

        merged
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSettings {
    #[serde(default = "default_agent_enabled")]
    pub enabled: bool,
    #[serde(default = "default_agent_version")]
    pub version: u8,
    #[serde(default)]
    pub default_model: Option<ModelSelection>,
    #[serde(default)]
    pub inline_assistant_model: Option<ModelSelection>,
    #[serde(default)]
    pub commit_message_model: Option<ModelSelection>,
    #[serde(default)]
    pub thread_summary_model: Option<ModelSelection>,
    #[serde(default)]
    pub subagent_model: Option<ModelSelection>,
    #[serde(default)]
    pub inline_alternatives: Vec<ModelSelection>,
    #[serde(default)]
    pub model_parameters: Vec<ModelParameterRule>,
    #[serde(default)]
    pub tool_permissions: ToolPermissions,
    #[serde(default = "default_message_editor_min_lines")]
    pub message_editor_min_lines: u8,
    #[serde(default)]
    pub use_modifier_to_send: bool,
    #[serde(default = "default_true")]
    pub expand_edit_card: bool,
    #[serde(default = "default_true")]
    pub expand_terminal_card: bool,
    #[serde(default = "default_true")]
    pub enable_feedback: bool,
    #[serde(skip)]
    fallback_model: ModelSelection,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            version: 2,
            default_model: Some(ModelSelection {
                provider: "deepseek".to_string(),
                model: "deepseek-chat".to_string(),
            }),
            inline_assistant_model: None,
            commit_message_model: None,
            thread_summary_model: None,
            subagent_model: None,
            inline_alternatives: Vec::new(),
            model_parameters: vec![ModelParameterRule {
                provider: None,
                model: None,
                temperature: Some(0.2),
                max_output_tokens: Some(2048),
            }],
            tool_permissions: ToolPermissions::default(),
            message_editor_min_lines: 4,
            use_modifier_to_send: false,
            expand_edit_card: true,
            expand_terminal_card: true,
            enable_feedback: true,
            fallback_model: ModelSelection::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
}

impl Default for ModelSelection {
    fn default() -> Self {
        Self {
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelParameterRule {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct ModelParameters {
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPermissions {
    #[serde(default)]
    pub default: ToolPermissionMode,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolPermissionRule>,
}

impl Default for ToolPermissions {
    fn default() -> Self {
        Self {
            default: ToolPermissionMode::Confirm,
            tools: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionMode {
    Allow,
    Deny,
    #[default]
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolPermissionRule {
    #[serde(default)]
    pub default: Option<ToolPermissionMode>,
    #[serde(default)]
    pub always_allow: Vec<PatternRule>,
    #[serde(default)]
    pub always_deny: Vec<PatternRule>,
    #[serde(default)]
    pub always_confirm: Vec<PatternRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternRule {
    pub pattern: String,
    #[serde(default)]
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageModelsSettings {
    #[serde(default)]
    pub openai: ProviderSettings,
    #[serde(default)]
    pub deepseek: ProviderSettings,
    #[serde(default)]
    pub open_router: ProviderSettings,
    #[serde(default)]
    pub ollama: ProviderSettings,
    #[serde(default)]
    pub lmstudio: ProviderSettings,
    #[serde(default)]
    pub anthropic: ProviderSettings,
    #[serde(default)]
    pub google: ProviderSettings,
    #[serde(default)]
    pub openai_compatible: BTreeMap<String, ProviderSettings>,
}

impl Default for LanguageModelsSettings {
    fn default() -> Self {
        Self {
            openai: ProviderSettings::openai(),
            deepseek: ProviderSettings::deepseek(),
            open_router: ProviderSettings::open_router(),
            ollama: ProviderSettings::ollama(),
            lmstudio: ProviderSettings::lmstudio(),
            anthropic: ProviderSettings::anthropic(),
            google: ProviderSettings::google(),
            openai_compatible: BTreeMap::new(),
        }
    }
}

impl LanguageModelsSettings {
    pub fn provider(&self, id: &str) -> Option<&ProviderSettings> {
        match id {
            "openai" => Some(&self.openai),
            "deepseek" => Some(&self.deepseek),
            "open_router" => Some(&self.open_router),
            "ollama" => Some(&self.ollama),
            "lmstudio" => Some(&self.lmstudio),
            "anthropic" => Some(&self.anthropic),
            "google" => Some(&self.google),
            provider => self.openai_compatible.get(provider),
        }
    }

    pub fn providers(&self) -> Vec<(&str, &ProviderSettings)> {
        let mut providers = vec![
            ("deepseek", &self.deepseek),
            ("openai", &self.openai),
            ("open_router", &self.open_router),
            ("ollama", &self.ollama),
            ("lmstudio", &self.lmstudio),
            ("anthropic", &self.anthropic),
            ("google", &self.google),
        ];
        providers.extend(
            self.openai_compatible
                .iter()
                .map(|(name, settings)| (name.as_str(), settings)),
        );
        providers
    }

    fn providers_mut(&mut self) -> Vec<(String, String, &mut ProviderSettings)> {
        let mut providers = vec![
            (
                "deepseek".to_string(),
                "DeepSeek".to_string(),
                &mut self.deepseek,
            ),
            ("openai".to_string(), "OpenAI".to_string(), &mut self.openai),
            (
                "open_router".to_string(),
                "OpenRouter".to_string(),
                &mut self.open_router,
            ),
            ("ollama".to_string(), "Ollama".to_string(), &mut self.ollama),
            (
                "lmstudio".to_string(),
                "LM Studio".to_string(),
                &mut self.lmstudio,
            ),
            (
                "anthropic".to_string(),
                "Anthropic".to_string(),
                &mut self.anthropic,
            ),
            ("google".to_string(), "Google".to_string(), &mut self.google),
        ];
        providers.extend(
            self.openai_compatible
                .iter_mut()
                .map(|(name, settings)| (name.clone(), name.clone(), settings)),
        );
        providers
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettings {
    #[serde(default = "default_provider_version")]
    pub version: u8,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub api_format: ApiFormat,
    #[serde(default)]
    pub available_models: Vec<ModelDefinition>,
    #[serde(default)]
    pub auto_discover: bool,
    #[serde(default)]
    pub context_window: Option<u32>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            version: 1,
            api_url: None,
            api_key: None,
            api_key_env: None,
            api_format: ApiFormat::OpenAiChat,
            available_models: Vec::new(),
            auto_discover: false,
            context_window: None,
        }
    }
}

impl ProviderSettings {
    fn openai() -> Self {
        Self {
            api_url: Some("https://api.openai.com/v1".to_string()),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            available_models: vec![
                ModelDefinition::new("gpt-4o", "GPT-4o", 128000),
                ModelDefinition::new("gpt-4o-mini", "GPT-4o Mini", 128000),
            ],
            ..Self::default()
        }
    }

    fn deepseek() -> Self {
        Self {
            api_url: Some("https://api.deepseek.com/v1".to_string()),
            api_key_env: Some("DEEPSEEK_API_KEY".to_string()),
            available_models: vec![
                ModelDefinition::new("deepseek-chat", "DeepSeek Chat", 64000),
                ModelDefinition::new("deepseek-reasoner", "DeepSeek Reasoner", 64000),
            ],
            ..Self::default()
        }
    }

    fn open_router() -> Self {
        Self {
            api_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
            available_models: vec![ModelDefinition::new(
                "openai/gpt-4o-mini",
                "OpenRouter GPT-4o Mini",
                128000,
            )],
            ..Self::default()
        }
    }

    fn ollama() -> Self {
        Self {
            api_url: Some("http://localhost:11434/v1".to_string()),
            api_key_env: Some("OLLAMA_API_KEY".to_string()),
            available_models: vec![ModelDefinition::new(
                "qwen2.5-coder",
                "qwen2.5-coder",
                32768,
            )],
            auto_discover: true,
            ..Self::default()
        }
    }

    fn lmstudio() -> Self {
        Self {
            api_url: Some("http://localhost:1234/v1".to_string()),
            available_models: vec![ModelDefinition::new("local-model", "LM Studio", 32768)],
            ..Self::default()
        }
    }

    fn anthropic() -> Self {
        Self {
            api_url: Some("https://api.anthropic.com".to_string()),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            api_format: ApiFormat::Anthropic,
            available_models: vec![ModelDefinition::new(
                "claude-3-5-sonnet-20240620",
                "Claude 3.5 Sonnet",
                200000,
            )],
            ..Self::default()
        }
    }

    fn google() -> Self {
        Self {
            api_url: Some("https://generativelanguage.googleapis.com".to_string()),
            api_key_env: Some("GEMINI_API_KEY".to_string()),
            api_format: ApiFormat::Google,
            available_models: vec![ModelDefinition::new(
                "gemini-2.0-flash",
                "Gemini 2.0 Flash",
                1000000,
            )],
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    #[default]
    OpenAiChat,
    Anthropic,
    Google,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub max_tokens: u32,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub max_completion_tokens: Option<u32>,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub supports_thinking: bool,
    #[serde(default)]
    pub supports_prompt_cache_key: bool,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
}

impl ModelDefinition {
    pub fn new(name: &str, display_name: &str, max_tokens: u32) -> Self {
        Self {
            name: name.to_string(),
            display_name: Some(display_name.to_string()),
            max_tokens,
            max_output_tokens: None,
            max_completion_tokens: None,
            supports_tools: true,
            supports_images: false,
            supports_thinking: false,
            supports_prompt_cache_key: false,
            capabilities: ModelCapabilities::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default = "default_true")]
    pub chat_completions: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub images: bool,
    #[serde(default)]
    pub parallel_tool_calls: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            chat_completions: true,
            tools: true,
            images: false,
            parallel_tool_calls: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub provider: String,
    pub model: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResponse {
    pub content: String,
    pub provider: String,
    pub model: String,
}

pub fn send_chat_blocking(messages: &[ChatMessage]) -> anyhow::Result<ChatResponse> {
    let settings = AiSettings::load_default()?;
    send_chat_with_settings_blocking(&settings, messages)
}

pub fn send_chat_with_settings_blocking(
    settings: &AiSettings,
    messages: &[ChatMessage],
) -> anyhow::Result<ChatResponse> {
    let selection = settings.selected_model().clone();
    send_chat_with_model_blocking(settings, selection, messages)
}

pub fn send_chat_with_model_blocking(
    settings: &AiSettings,
    selection: ModelSelection,
    messages: &[ChatMessage],
) -> anyhow::Result<ChatResponse> {
    send_chat_with_model_timeout_blocking(settings, selection, messages, AI_REQUEST_TIMEOUT)
}

pub fn send_chat_with_model_timeout_blocking(
    settings: &AiSettings,
    selection: ModelSelection,
    messages: &[ChatMessage],
    request_timeout: Duration,
) -> anyhow::Result<ChatResponse> {
    if !settings.agent.enabled {
        return Err(anyhow!("AI agent is disabled in config"));
    }

    let provider = settings
        .provider_for_model(&selection)
        .with_context(|| format!("AI provider is not configured: {}", selection.provider))?;
    let provider = provider_with_stored_key(&selection.provider, provider)?;

    match provider.api_format {
        ApiFormat::OpenAiChat => {
            send_openai_compatible_chat(settings, &provider, &selection, messages, request_timeout)
        }
        ApiFormat::Anthropic => {
            send_anthropic_chat(settings, &provider, &selection, messages, request_timeout)
        }
        ApiFormat::Google => {
            send_google_chat(settings, &provider, &selection, messages, request_timeout)
        }
    }
}

pub fn send_chat_with_model_streaming_blocking<F>(
    settings: &AiSettings,
    selection: ModelSelection,
    messages: &[ChatMessage],
    mut on_delta: F,
) -> anyhow::Result<ChatResponse>
where
    F: FnMut(&str),
{
    if !settings.agent.enabled {
        return Err(anyhow!("AI agent is disabled in config"));
    }

    let provider = settings
        .provider_for_model(&selection)
        .ok_or_else(|| anyhow!("AI provider '{}' is not configured", selection.provider))?;
    let provider = provider_with_stored_key(&selection.provider, provider)?;

    match provider.api_format {
        ApiFormat::OpenAiChat => send_openai_compatible_chat_streaming(
            settings, &provider, &selection, messages, on_delta,
        ),
        ApiFormat::Anthropic | ApiFormat::Google => {
            let response = send_chat_with_model_blocking(settings, selection, messages)?;
            on_delta(&response.content);
            Ok(response)
        }
    }
}

fn send_openai_compatible_chat(
    settings: &AiSettings,
    provider: &ProviderSettings,
    selection: &ModelSelection,
    messages: &[ChatMessage],
    request_timeout: Duration,
) -> anyhow::Result<ChatResponse> {
    let api_url = provider
        .api_url
        .as_deref()
        .ok_or_else(|| anyhow!("AI provider '{}' does not have api_url", selection.provider))?;
    let endpoint = openai_chat_completions_endpoint(api_url);
    let params = settings.model_parameters(selection);

    let mut body = json!({
        "model": selection.model,
        "messages": messages.iter().map(openai_message).collect::<Vec<_>>(),
    });

    if let Some(temperature) = params.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(max_tokens) = params.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    let mut request = ai_http_client_with_timeout(request_timeout)
        .build()
        .context("创建 AI HTTP 客户端失败")?
        .post(endpoint)
        .json(&body);

    if let Some(api_key) = provider.api_key() {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .send()
        .map_err(|err| anyhow!(format_ai_transport_error(&selection.provider, &err)))?;
    let status = response.status();
    let response_body = response.text().context("读取 AI 响应失败")?;

    if !status.is_success() {
        return Err(anyhow!(format_ai_http_error(
            &selection.provider,
            status,
            &response_body
        )));
    }

    let value: Value = serde_json::from_str(&response_body).context("解析 AI 响应失败")?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .pointer("/output/0/content/0/text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| anyhow!("AI 响应中没有返回消息内容"))?;

    Ok(ChatResponse {
        content,
        provider: selection.provider.clone(),
        model: selection.model.clone(),
    })
}

fn send_openai_compatible_chat_streaming<F>(
    settings: &AiSettings,
    provider: &ProviderSettings,
    selection: &ModelSelection,
    messages: &[ChatMessage],
    mut on_delta: F,
) -> anyhow::Result<ChatResponse>
where
    F: FnMut(&str),
{
    let api_url = provider
        .api_url
        .as_deref()
        .ok_or_else(|| anyhow!("AI provider '{}' does not have api_url", selection.provider))?;
    let endpoint = openai_chat_completions_endpoint(api_url);
    let params = settings.model_parameters(selection);

    let mut body = json!({
        "model": selection.model,
        "messages": messages.iter().map(openai_message).collect::<Vec<_>>(),
        "stream": true,
    });

    if let Some(temperature) = params.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(max_tokens) = params.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    let mut request = ai_http_client()
        .build()
        .context("创建 AI HTTP 客户端失败")?
        .post(endpoint)
        .json(&body);

    if let Some(api_key) = provider.api_key() {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .send()
        .map_err(|err| anyhow!(format_ai_transport_error(&selection.provider, &err)))?;
    let status = response.status();

    if !status.is_success() {
        let response_body = response.text().context("读取 AI 响应失败")?;
        return Err(anyhow!(format_ai_http_error(
            &selection.provider,
            status,
            &response_body
        )));
    }

    let mut content = String::new();
    let mut reader = BufReader::new(response);
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .context("读取 AI 流式响应失败")?;
        if read == 0 {
            break;
        }

        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }

        let value: Value = serde_json::from_str(data).context("解析 AI 流式响应失败")?;
        if let Some(delta) = openai_stream_delta(&value) {
            content.push_str(delta);
            on_delta(delta);
        }
    }

    if content.is_empty() {
        return Err(anyhow!("AI 响应中没有返回消息内容"));
    }

    Ok(ChatResponse {
        content,
        provider: selection.provider.clone(),
        model: selection.model.clone(),
    })
}

fn openai_chat_completions_endpoint(api_url: &str) -> String {
    let api_url = api_url.trim().trim_end_matches('/');
    if api_url.ends_with("/chat/completions") {
        api_url.to_string()
    } else {
        format!("{api_url}/chat/completions")
    }
}

fn ai_http_client() -> reqwest::blocking::ClientBuilder {
    ai_http_client_with_timeout(AI_REQUEST_TIMEOUT)
}

fn ai_http_client_with_timeout(request_timeout: Duration) -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .connect_timeout(AI_CONNECT_TIMEOUT)
        .timeout(request_timeout)
}

fn send_anthropic_chat(
    settings: &AiSettings,
    provider: &ProviderSettings,
    selection: &ModelSelection,
    messages: &[ChatMessage],
    request_timeout: Duration,
) -> anyhow::Result<ChatResponse> {
    let api_url = provider
        .api_url
        .as_deref()
        .ok_or_else(|| anyhow!("AI provider '{}' does not have api_url", selection.provider))?;
    let api_key = provider
        .api_key()
        .ok_or_else(|| anyhow!("missing API key env for provider '{}'", selection.provider))?;
    let endpoint = format!("{}/v1/messages", api_url.trim_end_matches('/'));
    let params = settings.model_parameters(selection);
    let max_tokens = params.max_output_tokens.unwrap_or(2048);
    let system = messages
        .iter()
        .filter(|message| message.role == ChatRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let anthropic_messages = messages
        .iter()
        .filter(|message| message.role != ChatRole::System)
        .map(|message| {
            let role = match message.role {
                ChatRole::Assistant => "assistant",
                ChatRole::System | ChatRole::User => "user",
            };
            json!({
                "role": role,
                "content": message.content,
            })
        })
        .collect::<Vec<_>>();

    let mut body = json!({
        "model": selection.model,
        "max_tokens": max_tokens,
        "messages": anthropic_messages,
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if let Some(temperature) = params.temperature {
        body["temperature"] = json!(temperature);
    }

    let response = ai_http_client_with_timeout(request_timeout)
        .build()
        .context("创建 AI HTTP 客户端失败")?
        .post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .map_err(|err| anyhow!(format_ai_transport_error(&selection.provider, &err)))?;
    let status = response.status();
    let response_body = response.text().context("读取 Anthropic 响应失败")?;

    if !status.is_success() {
        return Err(anyhow!(format_ai_http_error(
            &selection.provider,
            status,
            &response_body
        )));
    }

    let value: Value = serde_json::from_str(&response_body).context("解析 Anthropic 响应失败")?;
    let content = value
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Anthropic 响应中没有返回消息内容"))?;

    Ok(ChatResponse {
        content,
        provider: selection.provider.clone(),
        model: selection.model.clone(),
    })
}

fn send_google_chat(
    settings: &AiSettings,
    provider: &ProviderSettings,
    selection: &ModelSelection,
    messages: &[ChatMessage],
    request_timeout: Duration,
) -> anyhow::Result<ChatResponse> {
    let api_url = provider
        .api_url
        .as_deref()
        .ok_or_else(|| anyhow!("AI provider '{}' does not have api_url", selection.provider))?;
    let api_key = provider
        .api_key()
        .ok_or_else(|| anyhow!("missing API key env for provider '{}'", selection.provider))?;
    let endpoint = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        api_url.trim_end_matches('/'),
        selection.model,
        api_key
    );
    let params = settings.model_parameters(selection);
    let system = messages
        .iter()
        .filter(|message| message.role == ChatRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let contents = messages
        .iter()
        .filter(|message| message.role != ChatRole::System)
        .map(|message| {
            let role = match message.role {
                ChatRole::Assistant => "model",
                ChatRole::System | ChatRole::User => "user",
            };
            json!({
                "role": role,
                "parts": [{ "text": message.content }],
            })
        })
        .collect::<Vec<_>>();

    let mut body = json!({
        "contents": contents,
    });
    if !system.is_empty() {
        body["system_instruction"] = json!({
            "parts": [{ "text": system }]
        });
    }
    if params.temperature.is_some() || params.max_output_tokens.is_some() {
        body["generationConfig"] = json!({});
        if let Some(temperature) = params.temperature {
            body["generationConfig"]["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = params.max_output_tokens {
            body["generationConfig"]["maxOutputTokens"] = json!(max_tokens);
        }
    }

    let response = ai_http_client_with_timeout(request_timeout)
        .build()
        .context("创建 AI HTTP 客户端失败")?
        .post(endpoint)
        .json(&body)
        .send()
        .map_err(|err| anyhow!(format_ai_transport_error(&selection.provider, &err)))?;
    let status = response.status();
    let response_body = response.text().context("读取 Google AI 响应失败")?;

    if !status.is_success() {
        return Err(anyhow!(format_ai_http_error(
            &selection.provider,
            status,
            &response_body
        )));
    }

    let value: Value = serde_json::from_str(&response_body).context("解析 Google AI 响应失败")?;
    let content = value
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Google AI 响应中没有返回消息内容"))?;

    Ok(ChatResponse {
        content,
        provider: selection.provider.clone(),
        model: selection.model.clone(),
    })
}

fn openai_message(message: &ChatMessage) -> Value {
    let role = match message.role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    };
    json!({
        "role": role,
        "content": message.content,
    })
}

fn openai_stream_delta(value: &Value) -> Option<&str> {
    value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value
                .pointer("/output/0/content/0/text")
                .and_then(Value::as_str)
        })
}

fn format_ai_http_error(provider: &str, status: StatusCode, response_body: &str) -> String {
    let detail = extract_ai_error_detail(response_body);
    let suffix = detail
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(" 服务返回：{value}"))
        .unwrap_or_default();

    let message = match status {
        StatusCode::UNAUTHORIZED => {
            "AI 请求未授权。请检查 API Key 是否正确、是否过期，或环境变量/配置是否生效。"
        }
        StatusCode::FORBIDDEN => {
            "AI 请求被拒绝。请检查 API Key 权限、模型访问权限、账户余额、区域限制，或稍后重试。"
        }
        StatusCode::NOT_FOUND => "AI 模型或接口不存在。请检查模型名称和 API URL 是否匹配。",
        StatusCode::TOO_MANY_REQUESTS => {
            "AI 请求过于频繁。请稍后重试，或检查服务商限流、余额和套餐额度。"
        }
        status if status.is_server_error() => {
            "AI 服务暂时不可用。服务商返回了服务器错误，请稍后点击重试。"
        }
        _ => "AI 请求失败。请检查模型配置、API 地址和服务商返回信息。",
    };

    format!(
        "{message} 服务商：{provider}，状态码：{} {}。{suffix}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("Unknown")
    )
}

fn format_ai_transport_error(provider: &str, err: &reqwest::Error) -> String {
    let message = if err.is_timeout() {
        "AI 请求超时。请检查网络或稍后点击重试。"
    } else if err.is_connect() {
        "AI 网络连接失败。请检查代理、DNS、API 地址或本地模型服务是否启动。"
    } else if err.is_request() {
        "AI 请求无法发送。请检查 API 地址和请求配置。"
    } else {
        "AI 网络请求失败。请稍后点击重试。"
    };

    format!("{message} provider={provider}. 错误：{err}")
}

fn extract_ai_error_detail(response_body: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(response_body).ok()?;

    [
        "/error/message",
        "/message",
        "/msg",
        "/error",
        "/detail",
        "/details",
    ]
    .iter()
    .find_map(|pointer| {
        value.pointer(pointer).and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Object(_) | Value::Array(_) => Some(value.to_string()),
            Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
            Value::Null => None,
        })
    })
}

impl ProviderSettings {
    fn api_key(&self) -> Option<String> {
        self.api_key
            .as_deref()
            .map(str::to_string)
            .map(|value| strip_default_zero_padding(&value))
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                self.api_key_env
                    .as_deref()
                    .and_then(|key| env::var(key).ok())
                    .filter(|value| !value.trim().is_empty())
            })
    }
}

fn provider_with_stored_key(
    provider_id: &str,
    provider: &ProviderSettings,
) -> anyhow::Result<ProviderSettings> {
    let mut provider = provider.clone();

    let Some(key) = crate::db::ai::load_ai_provider_key_blocking(provider_id)? else {
        return Ok(provider);
    };
    if !key.enabled {
        return Ok(provider);
    }

    match key.key_source {
        crate::db::ai::AiProviderKeySource::Db => {
            provider.api_key = key.api_key;
            provider.api_key_env = None;
            let _ = crate::db::ai::touch_ai_provider_key_last_used_blocking(provider_id);
        }
        crate::db::ai::AiProviderKeySource::Env => {
            provider.api_key = None;
            provider.api_key_env = key.api_key_env;
        }
        crate::db::ai::AiProviderKeySource::None => {
            provider.api_key = None;
            provider.api_key_env = None;
        }
    }

    Ok(provider)
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

fn default_agent_enabled() -> bool {
    true
}

fn default_agent_version() -> u8 {
    2
}

fn default_provider_version() -> u8 {
    1
}

fn default_message_editor_min_lines() -> u8 {
    4
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_include_zed_style_agent_model() {
        let settings = AiSettings::default();

        assert_eq!(settings.selected_model().provider, "deepseek");
        assert_eq!(settings.selected_model().model, "deepseek-chat");
        assert!(settings.selected_provider().is_some());
    }

    #[test]
    fn merges_model_parameters_from_general_to_specific() {
        let settings = AiSettings {
            agent: AgentSettings {
                model_parameters: vec![
                    ModelParameterRule {
                        provider: None,
                        model: None,
                        temperature: Some(0.1),
                        max_output_tokens: Some(1000),
                    },
                    ModelParameterRule {
                        provider: Some("openai".to_string()),
                        model: None,
                        temperature: Some(0.4),
                        max_output_tokens: None,
                    },
                ],
                default_model: Some(ModelSelection {
                    provider: "openai".to_string(),
                    model: "gpt-4o".to_string(),
                }),
                ..AgentSettings::default()
            },
            ..AiSettings::default()
        };

        let params = settings.model_parameters(settings.selected_model());

        assert_eq!(params.temperature, Some(0.4));
        assert_eq!(params.max_output_tokens, Some(1000));
    }

    #[test]
    fn formats_forbidden_ai_error_with_provider_and_detail() {
        let message = format_ai_http_error(
            "open_router",
            StatusCode::FORBIDDEN,
            r#"{"message":"An unknown error occurred","success":false}"#,
        );

        assert!(message.contains("AI 请求被拒绝"));
        assert!(message.contains("服务商：open_router"));
        assert!(message.contains("状态码：403 Forbidden"));
        assert!(message.contains("An unknown error occurred"));
        assert!(!message.contains("AI request failed"));
        assert!(!message.contains("AI chat request failed"));
    }

    #[test]
    fn extracts_nested_ai_error_message() {
        assert_eq!(
            extract_ai_error_detail(r#"{"error":{"message":"invalid key"}}"#).as_deref(),
            Some("invalid key")
        );
    }

    #[test]
    fn strips_default_zero_padding_from_api_keys() {
        assert_eq!(
            strip_default_zero_padding("000000000000nvapi-real-key0000000000000000"),
            "nvapi-real-key"
        );
        assert_eq!(strip_default_zero_padding("sk-key-with-0"), "sk-key-with-0");
    }

    #[test]
    fn builds_openai_chat_completions_endpoint_from_base_or_full_url() {
        assert_eq!(
            openai_chat_completions_endpoint("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_completions_endpoint("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            openai_chat_completions_endpoint("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn extracts_openai_stream_delta_content() {
        let value = json!({
            "choices": [
                {
                    "delta": {
                        "content": "hello"
                    }
                }
            ]
        });

        assert_eq!(openai_stream_delta(&value), Some("hello"));
    }

    #[test]
    fn custom_provider_with_same_name_matches_by_model() {
        let mut settings = AiSettings::default();
        settings.language_models.openai_compatible.insert(
            "deepseek".to_string(),
            ProviderSettings {
                api_url: Some("https://custom.example/v1".to_string()),
                api_key: Some("custom-key".to_string()),
                available_models: vec![ModelDefinition::new(
                    "custom/deepseek-model",
                    "Custom DeepSeek",
                    128000,
                )],
                ..ProviderSettings::default()
            },
        );

        let provider = settings
            .provider_for_model(&ModelSelection {
                provider: "deepseek".to_string(),
                model: "custom/deepseek-model".to_string(),
            })
            .expect("custom provider should be selected");
        assert_eq!(
            provider.api_url.as_deref(),
            Some("https://custom.example/v1")
        );

        let builtin = settings
            .provider_for_model(&ModelSelection {
                provider: "deepseek".to_string(),
                model: "deepseek-chat".to_string(),
            })
            .expect("builtin provider should remain available");
        assert_eq!(
            builtin.api_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
    }
}

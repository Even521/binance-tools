use std::{
    collections::HashMap,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};

static THINK_MODE_ENABLED: AtomicBool = AtomicBool::new(false);

// 按请求超时时间缓存 HTTP 客户端，复用连接池。
static HTTP_CLIENTS: OnceLock<Mutex<HashMap<u128, reqwest::blocking::Client>>> = OnceLock::new();

fn http_client(timeout: Duration) -> anyhow::Result<reqwest::blocking::Client> {
    let clients = HTTP_CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut clients = clients
        .lock()
        .map_err(|_| anyhow!("HTTP 客户端缓存锁已损坏"))?;

    let key = timeout.as_nanos();

    if let Some(client) = clients.get(&key) {
        return Ok(client.clone());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .context("创建文本 AI 请求客户端失败")?;

    clients.insert(key, client.clone());

    Ok(client)
}

pub(super) fn send_openai_text_cancellable(
    api_url: &str,
    model: &str,
    system: &str,
    prompt: &str,
    request_timeout: Duration,
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    if !THINK_MODE_ENABLED.load(Ordering::Relaxed) {
        return send_text_request(
            api_url,
            model,
            system,
            prompt,
            request_timeout,
            cancel,
            false,
        );
    }

    match send_text_request(
        api_url,
        model,
        system,
        prompt,
        request_timeout,
        cancel,
        true,
    ) {
        Ok(content) => Ok(content),
        Err(error) if error.to_string().contains("结束原因：length") => {
            THINK_MODE_ENABLED.store(false, Ordering::Relaxed);

            super::super::automation::log_event(
                "think 模式耗尽生成上限且没有正文；本次关闭 think 重试，后续回复直接关闭 think。",
            );

            send_text_request(
                api_url,
                model,
                system,
                prompt,
                request_timeout,
                cancel,
                false,
            )
            .context("think 模式未生成正文，关闭 think 重试仍失败")
        }
        Err(error) => Err(error),
    }
}

fn send_text_request(
    api_url: &str,
    model: &str,
    system: &str,
    prompt: &str,
    request_timeout: Duration,
    cancel: &AtomicBool,
    think: bool,
) -> anyhow::Result<String> {
    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "已停止");

    let mut base = api_url.trim().trim_end_matches('/').to_string();

    for suffix in ["/chat/completions", "/v1"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.trim_end_matches('/').to_string();
        }
    }

    let endpoint = format!("{base}/api/chat");

    let body = json!({
        "model": model,
        "stream": false,
        "think": think,
        "options": {
            "temperature": 0.55,
            "top_p": 0.85,
            "top_k": 30,
            "repeat_penalty": 1.05,
            "num_predict": 128,
            "num_ctx": 8192
        },
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": prompt}
        ]
    });

    // 使用缓存的客户端，避免重复初始化连接池。
    let client = http_client(request_timeout)?;

    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .context("发送文本 AI 请求失败")?;

    anyhow::ensure!(
        response.status().is_success(),
        "文本 AI 请求失败：HTTP {}",
        response.status()
    );

    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "已停止");

    let value: Value = response.json().context("解析文本 AI 响应失败")?;

    let content = value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|content| !content.is_empty());

    content.map(str::to_string).ok_or_else(|| {
        let done_reason = value
            .get("done_reason")
            .and_then(Value::as_str)
            .unwrap_or("未知");

        let thinking_chars = value
            .pointer("/message/thinking")
            .and_then(Value::as_str)
            .map(|thinking| thinking.chars().count())
            .unwrap_or(0);

        anyhow!(
            "文本 AI 响应正文为空（结束原因：{done_reason}，thinking 长度：{thinking_chars} 字）；请检查模型是否在生成正文前耗尽 token 上限"
        )
    })
}

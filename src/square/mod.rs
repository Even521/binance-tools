//! Binance Square message sender, task executor and AI task generator.

use crate::{
    ai::{AiSettings, ChatMessage, send_chat_with_model_timeout_blocking},
    binance::market::MarketProduct,
    db::square::{
        BinanceSquareTask, NewBinanceSquareAiLog, NewBinanceSquareSendLog,
        SQUARE_TASK_STATUS_FAILED, SQUARE_TASK_STATUS_SENDING, SQUARE_TASK_STATUS_SENT,
        SQUARE_TASK_STATUS_SKIPPED,
    },
};
use anyhow::{Context, anyhow};
use reqwest::{
    StatusCode,
    blocking::Client,
    header::{CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, thread::sleep, time::Duration};

const SQUARE_CONTENT_ADD_URL: &str =
    "https://www.binance.com/bapi/composite/v1/public/pgc/openApi/content/add";
const MAX_NETWORK_RETRIES: u32 = 3;
const SQUARE_AI_MARKET_QUOTE: &str = "USDT";
const SQUARE_AI_MARKET_LIMIT: usize = 50;
const SQUARE_AI_DAILY_LIMIT: usize = 12;
const SQUARE_AI_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SquareSendStatus {
    Success,
    Skipped,
    Failed,
    DailyLimit,
    KeyExpired,
}

impl SquareSendStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::DailyLimit => "daily_limit",
            Self::KeyExpired => "key_expired",
        }
    }

    fn should_stop_executor(&self) -> bool {
        matches!(self, Self::DailyLimit | Self::KeyExpired)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquareSendResult {
    pub status: SquareSendStatus,
    pub response_code: Option<String>,
    pub error_message: Option<String>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquareTaskRunSummary {
    pub processed: usize,
    pub success: usize,
    pub skipped: usize,
    pub failed: usize,
    pub stopped: bool,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquareAiGenerationSummary {
    pub enabled: bool,
    pub due: bool,
    pub generated: bool,
    pub title: Option<String>,
    pub message: Option<String>,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquareAutomationSummary {
    pub ai: SquareAiGenerationSummary,
    pub tasks: SquareTaskRunSummary,
}

#[derive(Debug, Serialize)]
struct SquareContentAddRequest<'a> {
    #[serde(rename = "bodyTextOnly")]
    body_text_only: &'a str,
}

#[derive(Debug, Deserialize)]
struct SquareApiResponse {
    code: Option<Value>,
    message: Option<String>,
    msg: Option<String>,
}

pub fn send_square_message_blocking(
    api_key: &str,
    body_text_only: &str,
) -> anyhow::Result<SquareSendResult> {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build Square API HTTP client failed")?;

    send_square_message_with_client(&client, api_key, body_text_only)
}

pub fn run_due_square_tasks_blocking() -> anyhow::Result<SquareTaskRunSummary> {
    let Some(key) = crate::db::square::load_square_api_key_blocking()? else {
        return Err(anyhow!("Binance Square API Key is not configured"));
    };
    let tasks = crate::db::square::claim_due_square_tasks_blocking()?;
    run_square_tasks_blocking(&key.api_key, tasks)
}

pub fn run_square_automation_blocking() -> anyhow::Result<SquareAutomationSummary> {
    let ai = run_square_ai_generation_if_due_blocking()?;
    let tasks = run_due_square_tasks_blocking()?;
    Ok(SquareAutomationSummary { ai, tasks })
}

pub fn run_square_ai_generation_if_due_blocking() -> anyhow::Result<SquareAiGenerationSummary> {
    let settings = crate::db::square::load_square_ai_settings_blocking()?;
    if !settings.enabled {
        return Ok(SquareAiGenerationSummary {
            enabled: false,
            due: false,
            generated: false,
            title: None,
            message: None,
            skipped_reason: Some("AI 分析开关未开启".to_string()),
        });
    }

    if !crate::db::square::square_ai_generation_due_blocking()? {
        return Ok(SquareAiGenerationSummary {
            enabled: true,
            due: false,
            generated: false,
            title: None,
            message: None,
            skipped_reason: Some("未到下次 AI 分析时间".to_string()),
        });
    }

    match generate_square_ai_task_blocking() {
        Ok(summary) => {
            crate::db::square::mark_square_ai_next_run_blocking("'+1 hour'")?;
            Ok(summary)
        }
        Err(err) => {
            crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
                status: "failed".to_string(),
                title: None,
                message: None,
                error_message: Some(err.to_string()),
                created_task_id: None,
            })?;
            crate::db::square::mark_square_ai_next_run_blocking("'+1 hour'")?;
            Err(err)
        }
    }
}

pub fn run_square_ai_generation_now_blocking() -> anyhow::Result<SquareAiGenerationSummary> {
    match generate_square_ai_task_blocking() {
        Ok(summary) => {
            crate::db::square::mark_square_ai_next_run_blocking("'+1 hour'")?;
            Ok(summary)
        }
        Err(err) => {
            crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
                status: "failed".to_string(),
                title: None,
                message: None,
                error_message: Some(err.to_string()),
                created_task_id: None,
            })?;
            Err(err)
        }
    }
}

pub fn send_square_message_now_blocking(
    body_text_only: String,
) -> anyhow::Result<SquareSendResult> {
    let Some(key) = crate::db::square::load_square_api_key_blocking()? else {
        return Err(anyhow!("Binance Square API Key is not configured"));
    };

    let result = send_square_message_blocking(&key.api_key, &body_text_only)?;
    crate::db::square::record_square_send_log_blocking(NewBinanceSquareSendLog {
        task_id: None,
        status: result.status.as_str().to_string(),
        response_code: result.response_code.clone(),
        message_digest: message_digest(&body_text_only),
        error_message: result.error_message.clone(),
        retry_count: result.retry_count,
    })?;

    Ok(result)
}

pub fn send_square_task_message_now_blocking(
    task_id: i64,
    body_text_only: String,
) -> anyhow::Result<SquareSendResult> {
    let Some(key) = crate::db::square::load_square_api_key_blocking()? else {
        return Err(anyhow!("Binance Square API Key is not configured"));
    };

    crate::db::square::mark_square_task_status_blocking(task_id, SQUARE_TASK_STATUS_SENDING)?;
    let result = match send_square_message_blocking(&key.api_key, &body_text_only) {
        Ok(result) => result,
        Err(err) => {
            crate::db::square::mark_square_task_status_blocking(
                task_id,
                SQUARE_TASK_STATUS_FAILED,
            )?;
            crate::db::square::record_square_send_log_blocking(NewBinanceSquareSendLog {
                task_id: Some(task_id),
                status: SquareSendStatus::Failed.as_str().to_string(),
                response_code: None,
                message_digest: message_digest(&body_text_only),
                error_message: Some(err.to_string()),
                retry_count: 0,
            })?;
            return Err(err);
        }
    };
    crate::db::square::record_square_send_log_blocking(NewBinanceSquareSendLog {
        task_id: Some(task_id),
        status: result.status.as_str().to_string(),
        response_code: result.response_code.clone(),
        message_digest: message_digest(&body_text_only),
        error_message: result.error_message.clone(),
        retry_count: result.retry_count,
    })?;
    crate::db::square::mark_square_task_status_blocking(
        task_id,
        task_status_for_send_status(&result.status),
    )?;

    Ok(result)
}

fn run_square_tasks_blocking(
    api_key: &str,
    tasks: Vec<BinanceSquareTask>,
) -> anyhow::Result<SquareTaskRunSummary> {
    let mut summary = SquareTaskRunSummary {
        processed: 0,
        success: 0,
        skipped: 0,
        failed: 0,
        stopped: false,
        stop_reason: None,
    };

    for task in tasks {
        let body_text_only = render_task_message(&task.message);
        let result = send_square_message_blocking(api_key, &body_text_only)?;
        let status = result.status.clone();
        crate::db::square::record_square_send_log_blocking(NewBinanceSquareSendLog {
            task_id: Some(task.id),
            status: status.as_str().to_string(),
            response_code: result.response_code.clone(),
            message_digest: message_digest(&body_text_only),
            error_message: result.error_message.clone(),
            retry_count: result.retry_count,
        })?;

        summary.processed += 1;
        match status {
            SquareSendStatus::Success => {
                summary.success += 1;
                crate::db::square::mark_square_task_status_blocking(
                    task.id,
                    SQUARE_TASK_STATUS_SENT,
                )?;
            }
            SquareSendStatus::Skipped => {
                summary.skipped += 1;
                crate::db::square::mark_square_task_status_blocking(
                    task.id,
                    SQUARE_TASK_STATUS_SKIPPED,
                )?;
            }
            SquareSendStatus::Failed => {
                summary.failed += 1;
                crate::db::square::mark_square_task_status_blocking(
                    task.id,
                    SQUARE_TASK_STATUS_FAILED,
                )?;
            }
            SquareSendStatus::DailyLimit | SquareSendStatus::KeyExpired => {
                summary.failed += 1;
                summary.stopped = true;
                summary.stop_reason = result.error_message;
                crate::db::square::mark_square_task_status_blocking(
                    task.id,
                    SQUARE_TASK_STATUS_FAILED,
                )?;
            }
        }

        if status.should_stop_executor() {
            break;
        }
    }

    Ok(summary)
}

fn task_status_for_send_status(status: &SquareSendStatus) -> &'static str {
    match status {
        SquareSendStatus::Success => SQUARE_TASK_STATUS_SENT,
        SquareSendStatus::Skipped => SQUARE_TASK_STATUS_SKIPPED,
        SquareSendStatus::Failed | SquareSendStatus::DailyLimit | SquareSendStatus::KeyExpired => {
            SQUARE_TASK_STATUS_FAILED
        }
    }
}

fn generate_square_ai_task_blocking() -> anyhow::Result<SquareAiGenerationSummary> {
    let used_titles = crate::db::square::list_today_ai_titles_blocking()?;
    if used_titles.len() >= SQUARE_AI_DAILY_LIMIT {
        let reason = format!("今天 AI 任务已达到上限 {SQUARE_AI_DAILY_LIMIT} 条");
        crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
            status: "skipped".to_string(),
            title: None,
            message: None,
            error_message: Some(reason.clone()),
            created_task_id: None,
        })?;
        return Ok(SquareAiGenerationSummary {
            enabled: true,
            due: true,
            generated: false,
            title: None,
            message: None,
            skipped_reason: Some(reason),
        });
    }

    let products = square_ai_candidate_products(&used_titles)?;
    if products.is_empty() {
        let reason = "过滤当天已发布币种后，没有可用于 AI 分析的市场数据".to_string();
        crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
            status: "skipped".to_string(),
            title: None,
            message: None,
            error_message: Some(reason.clone()),
            created_task_id: None,
        })?;
        return Ok(SquareAiGenerationSummary {
            enabled: true,
            due: true,
            generated: false,
            title: None,
            message: None,
            skipped_reason: Some(reason),
        });
    }

    let prompt = build_square_ai_market_prompt(&products, &used_titles);
    let settings = AiSettings::load_default()?;
    let selection = settings.selected_model().clone();
    let mut last_error = None;
    let mut last_invalid_message: Option<String> = None;
    let mut last_invalid_title: Option<String> = None;

    for attempt in 0..2 {
        let response = match send_chat_with_model_timeout_blocking(
            &settings,
            selection.clone(),
            &[ChatMessage::user(prompt.clone())],
            SQUARE_AI_REQUEST_TIMEOUT,
        ) {
            Ok(response) => response,
            Err(err) => {
                last_error = Some(if attempt == 0 {
                    format!("{err}；已重试一次")
                } else {
                    err.to_string()
                });
                continue;
            }
        };
        let message =
            avoid_repetitive_square_phrases(&normalize_ai_square_message(&response.content));

        match validate_square_ai_message(&message) {
            Ok(title) => {
                if crate::db::square::ai_title_exists_today_blocking(&title)? {
                    last_error = Some(format!("AI 返回了今天已使用的标题：{title}"));
                    continue;
                }

                let task_id = crate::db::square::save_ai_square_task_blocking(
                    title.clone(),
                    message.clone(),
                    None,
                )?;
                crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
                    status: "success".to_string(),
                    title: Some(title.clone()),
                    message: Some(message.clone()),
                    error_message: None,
                    created_task_id: Some(task_id),
                })?;
                return Ok(SquareAiGenerationSummary {
                    enabled: true,
                    due: true,
                    generated: true,
                    title: Some(title),
                    message: Some(message),
                    skipped_reason: None,
                });
            }
            Err(err) => {
                if !message.trim().is_empty() {
                    last_invalid_title = extract_square_title(&message);
                    last_invalid_message = Some(message);
                }
                last_error = Some(if attempt == 0 {
                    format!("{err}；已重试一次")
                } else {
                    err.to_string()
                });
            }
        }
    }

    if let (Some(title), Some(message)) = (last_invalid_title, last_invalid_message) {
        if !crate::db::square::ai_title_exists_today_blocking(&title)? {
            let error_message =
                last_error.unwrap_or_else(|| "AI 输出不符合币安广场任务规则".to_string());
            let task_id = crate::db::square::save_ai_square_task_blocking(
                title.clone(),
                message.clone(),
                None,
            )?;
            crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
                status: "draft_invalid".to_string(),
                title: Some(title.clone()),
                message: Some(message.clone()),
                error_message: Some(error_message.clone()),
                created_task_id: Some(task_id),
            })?;
            return Ok(SquareAiGenerationSummary {
                enabled: true,
                due: true,
                generated: true,
                title: Some(title),
                message: Some(message),
                skipped_reason: Some(error_message),
            });
        }
    }

    let error_message = last_error.unwrap_or_else(|| "AI 输出不符合币安广场任务规则".to_string());
    save_fallback_square_ai_task(&products, error_message)
}

fn square_ai_candidate_products(used_titles: &[String]) -> anyhow::Result<Vec<MarketProduct>> {
    let used_assets = used_titles
        .iter()
        .filter_map(|title| title.strip_prefix('$'))
        .map(|asset| asset.to_ascii_uppercase())
        .collect::<HashSet<_>>();

    let mut products = crate::db::market::load_or_fetch_market_products_blocking()?
        .into_iter()
        .filter(|product| product.quote_asset == SQUARE_AI_MARKET_QUOTE && product.is_trading)
        .filter(|product| !used_assets.contains(&product.base_asset.to_ascii_uppercase()))
        .collect::<Vec<_>>();

    products.sort_by(|a, b| {
        b.price_change_percent
            .partial_cmp(&a.price_change_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    products.truncate(SQUARE_AI_MARKET_LIMIT);
    Ok(products)
}

fn save_fallback_square_ai_task(
    products: &[MarketProduct],
    error_message: String,
) -> anyhow::Result<SquareAiGenerationSummary> {
    let product = products
        .first()
        .ok_or_else(|| anyhow!("没有可用于兜底生成的市场数据：{error_message}"))?;
    let title = format!("${}", product.base_asset.to_ascii_uppercase());
    let message = format!("{title} {}", fallback_square_reason(product));
    validate_square_ai_message(&message)
        .with_context(|| format!("本地兜底文案不符合币安广场任务规则：{message}"))?;

    let task_id =
        crate::db::square::save_ai_square_task_blocking(title.clone(), message.clone(), None)?;
    crate::db::square::record_square_ai_log_blocking(NewBinanceSquareAiLog {
        status: "fallback".to_string(),
        title: Some(title.clone()),
        message: Some(message.clone()),
        error_message: Some(error_message.clone()),
        created_task_id: Some(task_id),
    })?;

    Ok(SquareAiGenerationSummary {
        enabled: true,
        due: true,
        generated: true,
        title: Some(title),
        message: Some(message),
        skipped_reason: Some(format!(
            "AI 请求失败，已使用本地市场数据生成草稿：{error_message}"
        )),
    })
}

fn fallback_square_reason(product: &MarketProduct) -> &'static str {
    let volume = product.quote_volume.unwrap_or_default();
    let market_cap = product.market_cap.unwrap_or_default();
    let tag_count = product.tags.len();
    let asset_seed = product
        .base_asset
        .bytes()
        .fold(0usize, |acc, byte| acc.wrapping_add(byte as usize));

    const VOLUME_REASONS: &[&str] = &[
        "成交额排位比较靠前，资金承接显得更主动，盘面热度不算单薄，后续可以观察换手能否维持",
        "量能表现明显更醒目，短线关注正在聚集，若换手节奏保持，板块讨论可能继续围绕它展开",
        "成交活跃度给人印象较深，资金流动没有明显冷清，当前更像是盘面里辨识度较高的标的",
        "交易额和流动性同时靠前，说明市场参与面不窄，短评角度看更适合放在核心观察池里",
        "盘口热度主要由成交承接支撑，不只是题材情绪推动，后续强弱要看资金是否继续停留",
    ];
    const CAP_REASONS: &[&str] = &[
        "流通体量相对扎实，成交配合度也能跟上，整体不是单纯情绪带动，后面要看热度延续性",
        "体量和流动性搭配较均衡，市场承接没有明显断层，短线表现比普通轮动标的更有辨识度",
        "市值基础不算薄弱，交易节奏也没有掉队，整体观感偏稳，适合用更长一点的周期观察",
        "流通规模给了盘面一定厚度，成交活跃度又能配合，说明当前关注并非只停留在概念层面",
        "资金容量和市场热度匹配度不错，波动里仍有承接，后续如果量能不散表现会更清晰",
    ];
    const TAG_REASONS: &[&str] = &[
        "标签覆盖的叙事较清晰，板块辨识度比较高，叠加成交不冷清，容易成为讨论里的焦点",
        "所属概念有一定记忆点，近期资金参与感增强，若板块情绪延续，热度可能继续停留",
        "赛道标签比较鲜明，市场容易形成共同认知，配合当前交易活跃度，后续反馈可以多看",
        "概念标签和资金活跃度形成呼应，不像孤立拉动，短线更需要观察板块内扩散效果",
        "叙事辨识度比较直接，成交也能支撑关注度，若同赛道继续活跃，它的位置会更突出",
    ];

    let reasons = if volume >= 10_000_000.0 {
        VOLUME_REASONS
    } else if market_cap >= 100_000_000.0 {
        CAP_REASONS
    } else if tag_count >= 3 {
        TAG_REASONS
    } else {
        &[
            "近期市场关注开始升温，成交和流动性都有一定改善，虽然仍需观察，但已经露出一些看点",
            "短线热度有抬头迹象，资金参与并不算弱，后续重点看盘面反馈是否能继续确认",
            "当前交易活跃度有改善，市场讨论逐步增加，整体节奏不算沉闷，后面可以多留意变化",
            "数据层面没有特别突兀，但成交和热度都有修复迹象，更适合用小周期继续观察强弱",
            "盘面表现不算高调，胜在资金参与逐步改善，若后续承接稳定，关注度可能慢慢打开",
        ]
    };

    reasons[asset_seed % reasons.len()]
}

fn build_square_ai_market_prompt(products: &[MarketProduct], used_titles: &[String]) -> String {
    let data = products
        .iter()
        .map(square_ai_product_json)
        .collect::<Vec<_>>()
        .join(",\n");
    let used = if used_titles.is_empty() {
        "无".to_string()
    } else {
        used_titles.join(", ")
    };

    format!(
        r#"你是币安广场内容助手。请只基于下面 JSON 数据筛选，不要编造外部行情、新闻或实时价格。
当前市场：{SQUARE_AI_MARKET_QUOTE}
今天已经发布过的标题：{used}

输出要求：
- 只选出最值得关注的 1 个币种，必须从 JSON products 中选择。
- 禁止选择今天已经发布过的标题。
- 必须严格使用一行格式：$币种 极短理由
- 中文汉字数量必须不少于 40 个且不超过 50 个；$币种代码、空格、标点和英文字母不计入这 40 到 50 个汉字。
- 不要标题，不要 Markdown 表格，不要风险提示，不要解释数据来源。
- 不要使用买入、梭哈、暴涨、翻倍、稳赚、必涨、带单、喊单、内幕、财富自由、保证收益、合约、杠杆、冲、无脑买、抄底、逃顶、目标价、止盈止损等容易触发币安广场风控或举报的词。
- 不要承诺收益，不要诱导交易，不要写价格预测。
- 不要输出涨幅、涨了多少、价格、目标位、百分比或任何具体数字行情。
- 理由必须围绕 JSON 里的 1 到 2 个重点指标写，例如成交额、换手活跃、流通市值、标签/赛道、流动性或市场热度。
- 表达要像人工观察后的短评，可以带一点主观判断，例如“承接更主动”“辨识度更高”“热度没有散”，但不要夸张。
- 每次措辞要自然变化，禁止反复套用“板块关注度持续提升、成交活跃、流动性良好、市场热度持续走高”这类固定模板。
- 禁止使用“值得跟踪”“值得追踪”“更值得跟踪”“值得留意”“值得关注”作为结尾，结尾要根据指标自然收束。
- 优先写出一个具体观察角度，例如成交承接、资金容量、赛道扩散、流动性改善、市场讨论焦点，不要只写泛泛热度。

分析数据 JSON：{{
  "quote_asset": "{SQUARE_AI_MARKET_QUOTE}",
  "limit": {SQUARE_AI_MARKET_LIMIT},
  "products": [
{data}
  ]
}}"#
    )
}

fn square_ai_product_json(product: &MarketProduct) -> String {
    format!(
        r#"    {{
      "symbol": {symbol},
      "base_asset": {base_asset},
      "asset_name": {asset_name},
      "price": {price},
      "change_24h_percent": {change},
      "quote_volume": {quote_volume},
      "market_cap": {market_cap},
      "tags": {tags}
    }}"#,
        symbol = json_string(&product.symbol),
        base_asset = json_string(&product.base_asset),
        asset_name = json_string(&product.asset_name),
        price = json_number(product.last_price),
        change = json_number(product.price_change_percent),
        quote_volume = json_number(product.quote_volume),
        market_cap = json_number(product.market_cap),
        tags = json_string_array(&product.tags),
    )
}

fn normalize_ai_square_message(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches('`')
        .trim()
        .to_string()
}

fn avoid_repetitive_square_phrases(message: &str) -> String {
    [
        ("更值得跟踪", "后续多看"),
        ("值得继续跟踪", "后续节奏可看"),
        ("值得继续追踪", "后续节奏可看"),
        ("值得跟踪", "后续多看"),
        ("值得追踪", "继续观察"),
        ("值得留意", "可以多看"),
        ("值得关注", "可以多看"),
        ("适合继续观察", "后续反馈可看"),
        ("适合持续观察", "后续反馈可看"),
    ]
    .into_iter()
    .fold(message.to_string(), |text, (from, to)| {
        text.replace(from, to)
    })
}

fn validate_square_ai_message(message: &str) -> anyhow::Result<String> {
    let title = extract_square_title(message).ok_or_else(|| anyhow!("AI 输出缺少 $币种标题"))?;
    let reason = message.strip_prefix(&title).unwrap_or(message).trim();
    if reason.is_empty() {
        return Err(anyhow!("AI 输出缺少理由"));
    }

    let chinese_count = reason.chars().filter(|ch| is_cjk(*ch)).count();
    if !(40..=50).contains(&chinese_count) {
        return Err(anyhow!("AI 输出中文汉字数量不在 40 到 50 个之间"));
    }

    let forbidden = [
        "买入",
        "梭哈",
        "暴涨",
        "翻倍",
        "稳赚",
        "必涨",
        "带单",
        "喊单",
        "内幕",
        "财富自由",
        "保证收益",
        "合约",
        "杠杆",
        "无脑买",
        "抄底",
        "逃顶",
        "目标价",
        "止盈",
        "止损",
    ];
    if let Some(word) = forbidden.iter().find(|word| message.contains(**word)) {
        return Err(anyhow!("AI 输出包含敏感词：{word}"));
    }
    if reason
        .chars()
        .any(|ch| ch.is_ascii_digit() || matches!(ch, '%' | '％' | '$'))
    {
        return Err(anyhow!("AI 输出包含具体数字行情或价格符号"));
    }

    Ok(title)
}

fn extract_square_title(message: &str) -> Option<String> {
    let token = message.split_whitespace().next()?;
    let title = token.trim_matches(|ch: char| matches!(ch, ',' | '，' | ':' | '：'));
    let asset = title.strip_prefix('$')?;
    if asset.len() < 2 || asset.len() > 20 {
        return None;
    }
    if !asset
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("${asset}"))
}

fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn json_string_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

fn json_number(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| {
            let text = format!("{value:.8}");
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        })
        .unwrap_or_else(|| "null".to_string())
}

fn send_square_message_with_client(
    client: &Client,
    api_key: &str,
    body_text_only: &str,
) -> anyhow::Result<SquareSendResult> {
    let mut retry_count = 0;
    loop {
        match send_once(client, api_key, body_text_only) {
            Ok(result) => return Ok(result.with_retry_count(retry_count)),
            Err(err) if retry_count < MAX_NETWORK_RETRIES => {
                retry_count += 1;
                sleep(Duration::from_secs(2_u64.pow(retry_count - 1)));
                if retry_count == MAX_NETWORK_RETRIES {
                    let final_result =
                        send_once(client, api_key, body_text_only).unwrap_or_else(|final_err| {
                            SquareSendResult {
                                status: SquareSendStatus::Failed,
                                response_code: None,
                                error_message: Some(final_err.to_string()),
                                retry_count,
                            }
                        });
                    return Ok(final_result.with_retry_count(retry_count));
                }
                let _ = err;
            }
            Err(err) => {
                return Ok(SquareSendResult {
                    status: SquareSendStatus::Failed,
                    response_code: None,
                    error_message: Some(err.to_string()),
                    retry_count,
                });
            }
        }
    }
}

fn send_once(
    client: &Client,
    api_key: &str,
    body_text_only: &str,
) -> anyhow::Result<SquareSendResult> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Square-OpenAPI-Key",
        HeaderValue::from_str(api_key).context("invalid Square API key header value")?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert("clienttype", HeaderValue::from_static("binanceSkill"));

    let response = client
        .post(SQUARE_CONTENT_ADD_URL)
        .headers(headers)
        .json(&SquareContentAddRequest { body_text_only })
        .send()
        .context("Square API request failed")?;
    let http_status = response.status();

    if is_retryable_http_status(http_status) {
        return Err(anyhow!("Square API temporary HTTP error: {http_status}"));
    }

    let body = response
        .text()
        .context("read Square API response body failed")?;
    let parsed = serde_json::from_str::<SquareApiResponse>(&body).unwrap_or(SquareApiResponse {
        code: None,
        message: Some(body.clone()),
        msg: None,
    });

    Ok(classify_response(http_status, parsed))
}

fn classify_response(http_status: StatusCode, response: SquareApiResponse) -> SquareSendResult {
    let code = response.code.as_ref().map(value_to_code);
    let message = response.message.or(response.msg);
    let message_lower = message.clone().unwrap_or_default().to_lowercase();

    match code.as_deref() {
        Some("20002" | "20022") => SquareSendResult {
            status: SquareSendStatus::Skipped,
            response_code: code,
            error_message: Some("敏感词，已跳过".to_string()),
            retry_count: 0,
        },
        Some("220009") => SquareSendResult {
            status: SquareSendStatus::DailyLimit,
            response_code: code,
            error_message: Some("达到每日发帖上限".to_string()),
            retry_count: 0,
        },
        Some("220004") => SquareSendResult {
            status: SquareSendStatus::KeyExpired,
            response_code: code,
            error_message: Some("Square API Key 已过期".to_string()),
            retry_count: 0,
        },
        _ if message_lower.contains("duplicate") || message_lower.contains("already") => {
            SquareSendResult {
                status: SquareSendStatus::Skipped,
                response_code: code,
                error_message: Some("重复发布，已跳过".to_string()),
                retry_count: 0,
            }
        }
        _ if http_status.is_success() => SquareSendResult {
            status: SquareSendStatus::Success,
            response_code: code,
            error_message: None,
            retry_count: 0,
        },
        _ => SquareSendResult {
            status: SquareSendStatus::Failed,
            response_code: code,
            error_message: message.or_else(|| Some(format!("Square API HTTP {http_status}"))),
            retry_count: 0,
        },
    }
}

fn is_retryable_http_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

fn value_to_code(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => value.to_string(),
    }
}

fn message_digest(message: &str) -> String {
    const MAX_CHARS: usize = 120;
    message.chars().take(MAX_CHARS).collect()
}

fn render_task_message(template: &str) -> String {
    template.trim().to_string()
}

impl SquareSendResult {
    fn with_retry_count(mut self, retry_count: u32) -> Self {
        self.retry_count = retry_count;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_sensitive_words_as_skipped() {
        let result = classify_response(
            StatusCode::OK,
            SquareApiResponse {
                code: Some(json!(20002)),
                message: None,
                msg: None,
            },
        );

        assert_eq!(result.status, SquareSendStatus::Skipped);
        assert_eq!(result.response_code.as_deref(), Some("20002"));
    }

    #[test]
    fn classifies_daily_limit_as_stop() {
        let result = classify_response(
            StatusCode::OK,
            SquareApiResponse {
                code: Some(json!(220009)),
                message: None,
                msg: None,
            },
        );

        assert_eq!(result.status, SquareSendStatus::DailyLimit);
        assert!(result.status.should_stop_executor());
    }

    #[test]
    fn truncates_message_digest() {
        assert_eq!(message_digest("abc"), "abc");
        assert_eq!("a".repeat(130).len(), 130);
        assert_eq!(message_digest(&"a".repeat(130)).len(), 120);
    }

    #[test]
    fn validates_square_ai_message_rules() {
        let message = "$RONIN 游戏赛道热度保持较高关注成交表现活跃社区讨论延续流动性相对稳定内容质量较稳适合继续观察";
        assert_eq!(
            validate_square_ai_message(message).unwrap(),
            "$RONIN".to_string()
        );
        assert!(validate_square_ai_message("$RONIN 涨了10%").is_err());
        assert!(validate_square_ai_message("$RONIN 买入热度很高").is_err());
    }
}

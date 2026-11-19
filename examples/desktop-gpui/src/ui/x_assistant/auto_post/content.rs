//! 新闻到 X 发帖内容准备。
//!
//! 本模块只生成并预留待发布内容，不操作浏览器、不点击发布。
//! 浏览器提交必须由 executor/submit 在统一 browser lease 下执行。

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{ensure, Context};
use binance_tools::{
    ai::{send_news_rewrite_chat_cancellable_blocking, AiSettings, ChatMessage},
    db::{
        open_default_connection,
        x_posting::{
            content_fingerprint, is_post_duplicate_blocking, is_post_similar_blocking,
            reserve_post_blocking,
        },
    },
};

const MAX_POST_CHARS: usize = 100;
const NEWS_SCAN_LIMIT: i64 = 100;
const MAX_COMPRESSION_ATTEMPTS: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewsKind {
    Crypto,
    Entertainment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewsFilter {
    Crypto,
    Entertainment,
    All,
}

impl NewsFilter {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "crypto" => Self::Crypto,
            "entertainment" => Self::Entertainment,
            _ => Self::All,
        }
    }

    fn includes(self, kind: NewsKind) -> bool {
        matches!(self, Self::All)
            || matches!(
                (self, kind),
                (Self::Crypto, NewsKind::Crypto) | (Self::Entertainment, NewsKind::Entertainment)
            )
    }
}

impl NewsKind {
    fn table(self) -> &'static str {
        match self {
            Self::Crypto => "crypto_news",
            Self::Entertainment => "entertainment_news",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Crypto => "币圈",
            Self::Entertainment => "娱乐",
        }
    }

    fn system_prompt(self) -> &'static str {
        match self {
            Self::Crypto => {
                "你是长期关注加密行业的中文 X 用户，不是新闻播报机器人。把一条新闻写成自然、克制、易读的短帖：先说最重要的事实，只有确实有助理解时才补充一项背景或数字。不要机械地重复标题，不要使用‘据消息’‘值得关注’‘引发关注’‘与此同时’等套话，不要堆砌机构名和数字，不要强行下结论、预测行情或添加个人观点。只能写一个核心事件；多事件、盘点或晚讯直接拒绝，不要拼成一帖。传闻保留传闻属性，不得补充来源没有的信息。格式固定为标题单独一行、空一行、正文；正文用完整自然句，必要的补充细节另起一行，不要把标题和正文挤成一段。不要使用感叹号、问号、话题标签或链接。控制在 100 个 Unicode 字符以内，只输出可直接发布的正文。"
            }
            Self::Entertainment => {
                "你是熟悉中文社交平台表达的娱乐资讯编辑，不是新闻播报机器人。把一条新闻写成自然、简洁、克制的中文短帖，优先讲清人物做了什么、发生了什么；只在有助理解时补充一项背景。不要机械重复标题，不用‘引发热议’‘值得关注’等空话，不写标题党、营销腔、强行互动或无依据评价。只写一个核心事件；多事件盘点或晚讯直接拒绝。传闻必须明确标注为传闻，不得造谣、影射或补充未经证实的信息。格式固定为标题单独一行、空一行、正文；正文用完整自然句，必要补充细节另起一行。不要使用感叹号、问号、话题标签或链接。控制在 100 个 Unicode 字符以内，只输出可直接发布的正文。"
            }
        }
    }
}

#[derive(Clone, Debug)]
struct NewsCandidate {
    kind: NewsKind,
    id: i64,
    title: String,
    body: String,
    url: Option<String>,
    published_at: Option<String>,
    collected_at: String,
    hot_score: i64,
}

#[derive(Clone, Debug)]
pub struct PreparedPost {
    pub post_id: i64,
    pub content: String,
}

fn hot_score(kind: NewsKind, title: &str, summary: &str, timestamp: &str) -> i64 {
    let text = format!("{title} {summary}").to_lowercase();
    let terms: &[(&str, i64)] = match kind {
        NewsKind::Crypto => &[
            ("比特币", 5),
            ("btc", 5),
            ("以太坊", 5),
            ("ethereum", 5),
            ("eth", 4),
            ("美联储", 5),
            ("降息", 5),
            ("加息", 4),
            ("etf", 5),
            ("sec", 4),
            ("监管", 4),
            ("黑客", 5),
            ("被盗", 5),
            ("漏洞", 4),
            ("清算", 5),
            ("暴涨", 4),
            ("暴跌", 4),
            ("大涨", 3),
            ("大跌", 3),
            ("突破", 3),
            ("历史新高", 5),
            ("创纪录", 4),
            ("资金流入", 4),
            ("资金流出", 4),
            ("交易所", 3),
            ("币安", 4),
            ("上币", 3),
            ("下架", 3),
            ("稳定币", 3),
            ("空投", 2),
            ("主网", 2),
            ("融资", 3),
            ("收购", 3),
            ("诉讼", 4),
            ("破产", 5),
            ("暂停提现", 5),
            ("清退", 3),
            ("政策", 3),
        ],
        NewsKind::Entertainment => &[
            ("官宣", 5),
            ("恋情", 5),
            ("分手", 5),
            ("结婚", 4),
            ("离婚", 5),
            ("出轨", 5),
            ("回应", 4),
            ("声明", 5),
            ("道歉", 4),
            ("塌房", 5),
            ("争议", 4),
            ("爆料", 4),
            ("辟谣", 5),
            ("工作室", 3),
            ("经纪人", 2),
            ("演员", 2),
            ("明星", 2),
            ("艺人", 2),
            ("综艺", 2),
            ("电视剧", 2),
            ("电影", 2),
            ("票房", 3),
            ("定档", 3),
            ("杀青", 2),
            ("开机", 2),
            ("红毯", 3),
            ("演唱会", 2),
            ("粉丝", 2),
            ("网红", 2),
            ("直播", 2),
        ],
    };
    let mut score = terms
        .iter()
        .filter(|(term, _)| text.contains(term))
        .map(|(_, weight)| *weight)
        .sum::<i64>();

    let title = title.to_lowercase();
    let title_terms: &[(&str, i64)] = match kind {
        NewsKind::Crypto => &[
            ("突发", 5),
            ("重磅", 4),
            ("紧急", 5),
            ("首次", 2),
            ("正式", 2),
            ("宣布", 2),
            ("巨鲸", 4),
            ("大额", 3),
            ("清算", 3),
        ],
        NewsKind::Entertainment => &[
            ("突发", 5),
            ("重磅", 4),
            ("紧急", 5),
            ("官宣", 5),
            ("回应", 3),
            ("吃瓜", 5),
            ("瓜", 2),
            ("爆料", 5),
            ("热搜", 4),
            ("离谱", 3),
            ("反转", 4),
            ("笑死", 3),
            ("搞笑", 3),
            ("段子", 3),
            ("声明", 4),
            ("首次", 2),
            ("正式", 2),
            ("曝光", 4),
        ],
    };
    for (term, weight) in title_terms {
        if title.contains(term) {
            score += weight;
        }
    }

    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        let age_hours = (app_log::beijing_now()
            - parsed.with_timezone(&app_log::beijing_timezone()))
        .num_hours()
        .max(0);
        score += match age_hours {
            0..=2 => 8,
            3..=6 => 6,
            7..=12 => 4,
            13..=24 => 2,
            25..=48 => 1,
            _ => 0,
        };
    }
    score
}

/// 复用新闻页面的币圈/娱乐关键词与新鲜度权重，跨来源统一按热点分排序。
fn load_recent_source_material() -> anyhow::Result<Vec<(String, String)>> {
    let connection = open_default_connection()?;
    let mut recent = Vec::new();
    for kind in [NewsKind::Crypto, NewsKind::Entertainment] {
        let sql = format!(
            "SELECT COALESCE(NULLIF(n.chinese_title, ''), n.title),\n                    COALESCE(NULLIF(n.chinese_summary, ''), n.summary, '')\n             FROM x_published_posts p\n             JOIN {} n ON p.news_id = '{}:' || n.id\n             WHERE p.status IN ('pending', 'submitting', 'published', 'unknown')\n             ORDER BY p.id DESC LIMIT 50",
            kind.table(),
            kind.table()
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        recent.extend(rows.collect::<Result<Vec<(String, String)>, _>>()?);
    }
    Ok(recent)
}

fn normalized_news_text(value: &str) -> Vec<char> {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn bigram_dice_similarity(left: &[char], right: &[char]) -> f64 {
    use std::collections::HashSet;

    if left.len() < 2 || right.len() < 2 {
        return 0.0;
    }
    let left = left
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<HashSet<_>>();
    let right = right
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<HashSet<_>>();
    let total = left.len() + right.len();
    if total == 0 {
        0.0
    } else {
        2.0 * left.intersection(&right).count() as f64 / total as f64
    }
}

fn is_likely_same_news(
    candidate_title: &str,
    candidate_body: &str,
    old_title: &str,
    old_body: &str,
) -> bool {
    let candidate_title = normalized_news_text(candidate_title);
    let old_title = normalized_news_text(old_title);
    if candidate_title.len() < 12 || old_title.len() < 12 {
        return false;
    }
    let title_similarity = bigram_dice_similarity(&candidate_title, &old_title);
    if title_similarity >= 0.62 {
        return true;
    }
    if title_similarity < 0.58 {
        return false;
    }
    bigram_dice_similarity(
        &normalized_news_text(candidate_body),
        &normalized_news_text(old_body),
    ) >= 0.62
}

fn source_contains_multiple_items(value: &str) -> bool {
    let mut plain = String::with_capacity(value.len());
    let mut inside_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => {
                inside_tag = false;
                plain.push('\n');
            }
            _ if !inside_tag => plain.push(ch),
            _ => {}
        }
    }
    contains_multiple_items(&plain)
}

fn load_candidates(filter: NewsFilter) -> anyhow::Result<Vec<NewsCandidate>> {
    app_log::post_info!(
        "X 自动发帖内容准备：开始从 SQLite 读取新闻，filter={:?}",
        filter
    );
    let connection = open_default_connection()?;
    let mut candidates = Vec::new();

    for kind in [NewsKind::Crypto, NewsKind::Entertainment] {
        if !filter.includes(kind) {
            continue;
        }
        let sql = format!(
            "SELECT id, title, COALESCE(NULLIF(chinese_title, ''), title),                     COALESCE(NULLIF(chinese_summary, ''), summary), url,                     published_at, collected_at              FROM {}              ORDER BY datetime(COALESCE(published_at, collected_at)) DESC              LIMIT ?1",
            kind.table()
        );
        let mut statement = connection
            .prepare(&sql)
            .with_context(|| format!("读取{}新闻候选失败", kind.label()))?;
        let rows = statement.query_map([NEWS_SCAN_LIMIT], |row| {
            let original_title: String = row.get(1)?;
            let display_title: String = row.get(2)?;
            let body: Option<String> = row.get(3)?;
            Ok(NewsCandidate {
                kind,
                id: row.get(0)?,
                title: if display_title.trim().is_empty() {
                    original_title
                } else {
                    display_title
                },
                body: body.unwrap_or_default(),
                url: row.get(4)?,
                published_at: row.get(5)?,
                collected_at: row.get(6)?,
                hot_score: 0,
            })
        })?;
        let source_candidates = rows.collect::<Result<Vec<_>, _>>()?;
        let raw_count = source_candidates.len();
        let now = app_log::beijing_now();
        let cutoff = now - chrono::Duration::hours(24);
        let mut accepted = 0usize;
        for mut candidate in source_candidates {
            // 发布时间优先；为空或格式异常时回退到采集时间。
            let timestamp = candidate
                .published_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .or_else(|| chrono::DateTime::parse_from_rfc3339(&candidate.collected_at).ok());
            let Some(timestamp) = timestamp else {
                app_log::post_warn!(
                    "X 自动发帖：排除时间无法解析的{}新闻，id={}",
                    candidate.kind.label(),
                    candidate.id
                );
                continue;
            };
            let beijing_timestamp = timestamp.with_timezone(&app_log::beijing_timezone());
            if beijing_timestamp < cutoff || beijing_timestamp > now {
                continue;
            }
            let timestamp_text = beijing_timestamp.to_rfc3339();
            candidate.hot_score = hot_score(
                candidate.kind,
                &candidate.title,
                &candidate.body,
                &timestamp_text,
            );
            accepted += 1;
            candidates.push(candidate);
        }
        app_log::post_info!(
            "X 自动发帖：{}新闻最近 24 小时筛选后保留 {} 条 / 原始 {} 条",
            kind.label(),
            accepted,
            raw_count
        );
    }

    app_log::post_info!(
        "X 自动发帖内容准备：数据库读取到 {} 条候选新闻",
        candidates.len()
    );
    candidates.sort_by(|left, right| {
        let left_time = left.published_at.as_deref().unwrap_or(&left.collected_at);
        let right_time = right.published_at.as_deref().unwrap_or(&right.collected_at);
        right
            .hot_score
            .cmp(&left.hot_score)
            .then_with(|| right_time.cmp(left_time))
    });
    Ok(candidates)
}

/// 清理新闻材料和 AI 输出中的 URL，包括 http(s)://、www. 和常见裸域名。
fn strip_urls(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut index = 0;

    while index < value.len() {
        let rest = &value[index..];
        let lower = rest.to_ascii_lowercase();
        if lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("www.")
        {
            while index < value.len() {
                let ch = value[index..].chars().next().expect("valid UTF-8 boundary");
                if ch.is_whitespace()
                    || matches!(
                        ch,
                        '，' | '。' | '；' | '、' | '！' | '？' | '）' | '】' | '>' | '"' | '\''
                    )
                {
                    break;
                }
                index += ch.len_utf8();
            }
            continue;
        }

        let ch = rest.chars().next().expect("valid UTF-8 boundary");
        if ch.is_ascii_alphanumeric() {
            let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let token = &rest[..token_end];
            let host = token.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | '['
                        | '{'
                        | '"'
                        | '\''
                        | '，'
                        | '。'
                        | '；'
                        | '、'
                        | '！'
                        | '？'
                        | ')'
                        | ']'
                        | '}'
                )
            });
            let domain = host.split('/').next().unwrap_or_default();
            let looks_like_domain = domain.rsplit_once('.').is_some_and(|(name, suffix)| {
                !name.is_empty()
                    && suffix.len() >= 2
                    && suffix.chars().all(|c| c.is_ascii_alphabetic())
            });
            if looks_like_domain {
                index += token_end;
                continue;
            }
        }

        output.push(ch);
        index += ch.len_utf8();
    }

    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 删除 AI 文案中的完整话题标签（#话题、#topic），保留标签前后的正文和段落。
fn strip_hashtags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '#' {
            while let Some(&next) = chars.peek() {
                if next.is_whitespace()
                    || matches!(
                        next,
                        '，' | '。'
                            | '；'
                            | '、'
                            | '！'
                            | '？'
                            | '：'
                            | ','
                            | '.'
                            | ';'
                            | '!'
                            | '?'
                            | ':'
                            | '）'
                            | ')'
                            | '】'
                            | ']'
                            | '}'
                            | '>'
                            | '"'
                            | '\''
                    )
                {
                    break;
                }
                chars.next();
            }
            continue;
        }
        output.push(ch);
    }

    output
}

/// 统一成“标题 + 空一行 + 正文”，并删除感叹号、问号和话题标签。
fn clean_ai_text(value: &str) -> String {
    let without_urls = strip_urls(value);
    let without_hashtags = strip_hashtags(&without_urls);
    let without_forbidden_punctuation = without_hashtags
        .chars()
        .filter(|ch| !matches!(ch, '！' | '!' | '？' | '?'))
        .collect::<String>();
    let cleaned = without_forbidden_punctuation
        .trim()
        .trim_matches('"')
        .trim_matches('“')
        .trim_matches('”')
        .trim();

    let paragraphs = cleaned
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    match paragraphs.split_first() {
        Some((title, body)) if !body.is_empty() => {
            format!("{}\n\n{}", title, body.join("\n"))
        }
        Some((only, _)) => (*only).to_string(),
        None => String::new(),
    }
}

fn contains_multiple_items(content: &str) -> bool {
    let list_items = content
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.chars()
                .next()
                .is_some_and(|ch| matches!(ch, '•' | '●' | '-' | '*'))
                || line.split_once('.').is_some_and(|(number, rest)| {
                    !rest.trim().is_empty()
                        && !number.is_empty()
                        && number.chars().all(|ch| ch.is_ascii_digit())
                })
                || line.split_once('、').is_some_and(|(number, rest)| {
                    !rest.trim().is_empty()
                        && !number.is_empty()
                        && number.chars().all(|ch| ch.is_ascii_digit())
                })
        })
        .count();
    list_items > 1
        || content.lines().any(|line| {
            line.trim_start()
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '•' | '●' | '-' | '*'))
        })
}

fn generate_post(candidate: &NewsCandidate, cancel: &AtomicBool) -> anyhow::Result<String> {
    ensure!(
        !cancel.load(Ordering::Acquire),
        "收到停止信号，取消 AI 文案生成"
    );
    app_log::post_info!(
        "X 自动发帖内容准备：开始调用 AI 改写新闻，类型={:?}，标题={}",
        candidate.kind,
        candidate.title
    );
    let settings = AiSettings::load_default().context("加载 AI 配置失败")?;
    ensure!(
        settings.agent.enabled,
        "AI agent 未启用，不能生成自动发帖内容"
    );
    let selection = settings.selected_model().clone();

    let source_title = strip_urls(&candidate.title);
    let source_body = if candidate.body.trim().is_empty() {
        source_title.clone()
    } else {
        strip_urls(&candidate.body)
    };
    let messages = [
        ChatMessage::system(candidate.kind.system_prompt()),
        ChatMessage::system(
            "写得像一个懂这条新闻的人在简洁转述，而不是把标题和摘要拼接起来。句式随事实变化，不要每条都用相同结构；只保留读者理解事件所需的信息。",
        ),
        ChatMessage::user(format!(
            "请将以下{}新闻整理成一条自然、简洁的中文 X 短帖。只写一个核心事件，不能把标题和摘要原样拼接；避免套话、空泛评价和新闻稿口吻。严格按格式输出：标题单独一行，空一行，再写正文；正文使用完整自然句，必要补充细节另起一行。多事件/盘点内容不要改写，直接输出‘跳过’。不得添加来源材料之外的信息。不要使用感叹号、问号和 #话题标签，不要输出网址、域名或链接。最多 100 个 Unicode 字符，只输出正文。\n\n标题：{}\n内容：{}",
            candidate.kind.label(),
            source_title,
            source_body
        )),
    ];
    let ai_started = std::time::Instant::now();
    app_log::post_info!(
        "X 自动发帖 AI：主改写请求开始，超时=180 秒，新闻标题={}",
        candidate.title
    );
    let response = match send_news_rewrite_chat_cancellable_blocking(
        &settings,
        selection.clone(),
        &messages,
        Duration::from_secs(180),
        cancel,
    ) {
        Ok(response) => {
            app_log::post_info!(
                "X 自动发帖 AI：主改写请求完成，耗时={} 秒，返回字符数={}",
                ai_started.elapsed().as_secs_f32(),
                response.content.chars().count()
            );
            response
        }
        Err(error) => {
            app_log::post_error!(
                "X 自动发帖 AI：主改写请求失败，耗时={} 秒，错误链：{error:#}",
                ai_started.elapsed().as_secs_f32()
            );
            return Err(error).context("AI 生成 X 发帖文案失败");
        }
    };
    let mut content = clean_ai_text(&response.content);
    ensure!(!content.is_empty(), "AI 返回的发帖文案为空");
    ensure!(
        !content.trim().starts_with("跳过"),
        "AI 判断该新闻包含多个事件或不适合发布，已跳过"
    );
    ensure!(
        !contains_multiple_items(&content),
        "AI 文案包含多条内容或多个事件，直接放弃该新闻，不进行压缩"
    );

    for attempt in 1..=MAX_COMPRESSION_ATTEMPTS {
        if content.chars().count() <= MAX_POST_CHARS {
            break;
        }
        let retry_messages = [
            ChatMessage::system(candidate.kind.system_prompt()),
            ChatMessage::user(format!(
                "将这条单一新闻自然地压缩到 100 个 Unicode 字符以内。保留标题、空行、正文的格式；正文不要重复标题。不要补充或推断信息，只留下最重要事实；若背景不是理解事件所必需就删掉。保持正常中文语序，避免关键词堆叠。当前字符数={}，本次是第 {attempt}/{MAX_COMPRESSION_ATTEMPTS} 次压缩。删除感叹号、问号和 #话题标签，只输出正文。\n\n新闻：{}\n当前文案：{}",
                content.chars().count(),
                candidate.title,
                content
            )),
        ];
        app_log::post_warn!(
            "X 自动发帖 AI：文案超过 {} 字，启动第 {attempt}/{MAX_COMPRESSION_ATTEMPTS} 次压缩，当前字符数={}",
            MAX_POST_CHARS,
            content.chars().count()
        );
        let retry_started = std::time::Instant::now();
        let retry = match send_news_rewrite_chat_cancellable_blocking(
            &settings,
            selection.clone(),
            &retry_messages,
            Duration::from_secs(180),
            cancel,
        ) {
            Ok(response) => {
                app_log::post_info!(
                    "X 自动发帖 AI：二次压缩完成，耗时={} 秒，返回字符数={}",
                    retry_started.elapsed().as_secs_f32(),
                    response.content.chars().count()
                );
                response
            }
            Err(error) => {
                app_log::post_error!(
                    "X 自动发帖 AI：二次压缩失败，耗时={} 秒，错误链：{error:#}",
                    retry_started.elapsed().as_secs_f32()
                );
                return Err(error).context("AI 二次压缩发帖文案失败");
            }
        };
        let compressed = clean_ai_text(&retry.content);
        ensure!(
            !contains_multiple_items(&compressed),
            "AI 压缩结果仍包含多条内容或多个事件，直接放弃该新闻"
        );
        app_log::post_info!(
            "X 自动发帖 AI：第 {attempt}/{MAX_COMPRESSION_ATTEMPTS} 次压缩后字符数={}",
            compressed.chars().count()
        );
        // 如果模型没有缩短文案，不再重复请求；交给候选遍历逻辑跳过这条新闻。
        if compressed.chars().count() >= content.chars().count() {
            break;
        }
        if !compressed.is_empty() {
            content = compressed;
        } else {
            break;
        }
    }

    ensure!(
        !content.is_empty() && content.chars().count() <= MAX_POST_CHARS,
        "AI 文案超过 100 字或为空，已阻止进入发布队列"
    );
    ensure!(
        !content.contains("作为人工智能") && !content.contains("以下是改写"),
        "AI 返回了说明性文字，不是可发布正文"
    );
    app_log::post_info!(
        "X 自动发帖内容准备：AI 文案生成完成，字符数={}",
        content.chars().count()
    );
    Ok(content)
}

/// 按所选新闻类型准备一条候选，先按热点分排序，再去重、AI 改写并持久化。
pub fn prepare_next_post_blocking(
    filter: NewsFilter,
    cancel: &AtomicBool,
) -> anyhow::Result<Option<PreparedPost>> {
    app_log::post_info!("X 自动发帖内容准备：进入候选准备流程");
    let selection_started = std::time::Instant::now();
    let candidates = load_candidates(filter)?;
    let recent_source_material = load_recent_source_material()?;
    app_log::post_info!(
        "X 自动发帖内容准备：候选读取与排序完成，数量={}，耗时={} 毫秒",
        candidates.len(),
        selection_started.elapsed().as_millis()
    );

    for candidate in candidates {
        ensure!(
            !cancel.load(Ordering::Acquire),
            "收到停止信号，取消候选新闻处理"
        );
        let news_id = format!("{}:{}", candidate.kind.table(), candidate.id);
        if source_contains_multiple_items(&candidate.body) {
            app_log::post_info!(
                "X 自动发帖内容准备：AI 改写前跳过列表/多条新闻素材，news_id={}，title={}",
                news_id,
                candidate.title
            );
            continue;
        }
        let original_material = format!(
            "{}\n{}\n{}",
            candidate.kind.table(),
            candidate.title.trim(),
            candidate.url.as_deref().unwrap_or_default().trim()
        );
        let original_hash = content_fingerprint(&original_material);

        if is_post_duplicate_blocking(&original_hash, "")? {
            app_log::post_info!("X 自动发帖内容准备：跳过重复新闻，news_id={}", news_id);
            continue;
        }

        if recent_source_material.iter().any(|(title, body)| {
            is_likely_same_news(&candidate.title, &candidate.body, title, body)
        }) {
            app_log::post_info!(
                "X 自动发帖内容准备：AI 改写前跳过与近期预留/发布新闻高度相似的来源，news_id={}，title={}",
                news_id,
                candidate.title
            );
            continue;
        }

        app_log::post_info!("X 自动发帖内容准备：选中候选新闻，news_id={}", news_id);
        // 最终入库前再次清理，防止 AI 输出或压缩重试结果夹带链接。
        let content = match generate_post(&candidate, cancel) {
            Ok(content) => strip_urls(&content),
            Err(error) => {
                app_log::post_warn!(
                    "X 自动发帖内容准备：跳过无法生成合规文案的候选，news_id={}，原因={error:#}",
                    news_id
                );
                continue;
            }
        };
        ensure!(
            !content.is_empty() && content.chars().count() <= MAX_POST_CHARS,
            "清理链接后 AI 文案为空或超过 100 字，已阻止发布"
        );

        let content_hash = content_fingerprint(&content);
        if is_post_similar_blocking(&content)? {
            app_log::post_info!(
                "X 自动发帖内容准备：跳过与近期发帖文案相似的候选，news_id={}",
                news_id
            );
            continue;
        }
        let post_id = reserve_post_blocking(
            Some(&news_id),
            candidate.url.as_deref(),
            &original_hash,
            &content_hash,
            &content,
        )
        .with_context(|| format!("预留{}新闻发帖记录失败", candidate.kind.label()))?;

        app_log::post_info!("X 自动发帖内容准备：已预留发帖记录，post_id={}", post_id);
        return Ok(Some(PreparedPost { post_id, content }));
    }

    app_log::post_warn!("X 自动发帖内容准备：遍历完所有候选，未找到可发布新闻");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{
        clean_ai_text, contains_multiple_items, is_likely_same_news,
        source_contains_multiple_items, NewsFilter, NewsKind,
    };

    #[test]
    fn cleans_wrapped_ai_output() {
        assert_eq!(clean_ai_text("  “一条新闻。”  "), "一条新闻。");
    }

    #[test]
    fn removes_complete_hashtags_from_ai_text() {
        assert_eq!(
            clean_ai_text("今天有新消息 #娱乐新闻，后续持续关注。"),
            "今天有新消息，后续持续关注。"
        );
        assert_eq!(
            clean_ai_text("市场出现变化 #Bitcoin #BTC。"),
            "市场出现变化。"
        );
        assert_eq!(
            clean_ai_text("没有话题标签的正文。"),
            "没有话题标签的正文。"
        );
    }

    #[test]
    fn keeps_core_content_and_detail_on_separate_lines() {
        assert_eq!(
            clean_ai_text("标题\n核心内容\n详情内容"),
            "标题\n\n核心内容\n详情内容"
        );
    }

    #[test]
    fn rejects_list_style_multi_item_posts() {
        assert!(contains_multiple_items("标题\n\n• 第一件事\n• 第二件事"));
        assert!(contains_multiple_items("标题\n\n1. 第一件事\n2. 第二件事"));
        assert!(!contains_multiple_items("标题\n\n核心内容\n一项必要详情"));
    }

    #[test]
    fn formats_title_and_body_and_removes_forbidden_punctuation() {
        assert_eq!(
            clean_ai_text(
                "《藏不住的好去处》新期打卡呼伦贝尔！\n主持人周雪梅携韩东霖手持“初始宝藏地图”，在辽阔天地间解锁草原城市魅力与人文温度?? #综艺日常#"
            ),
            "《藏不住的好去处》新期打卡呼伦贝尔\n\n主持人周雪梅携韩东霖手持“初始宝藏地图”，在辽阔天地间解锁草原城市魅力与人文温度"
        );
    }

    #[test]
    fn detects_same_event_reported_with_different_titles() {
        assert!(is_likely_same_news(
            "主网上线，Sui原生比特币金融协议Hashi开放使用",
            "BTC仍在比特币链上，按1:1铸造hBTC用于借贷",
            "比特币金融协议Hashi已在Sui主网上线",
            "Sui原生比特币金融协议Hashi现已上线主网，BTC按1:1铸造hBTC用于抵押借贷",
        ));
        assert!(!is_likely_same_news(
            "币安黑客将12000枚ETH转移并兑换为BTC",
            "通过94笔交易转入11个地址",
            "币安黑客将137600枚ETH分散转入12个钱包",
            "黑客转移约2.48亿美元ETH，目前未继续操作",
        ));
    }

    #[test]
    fn source_lists_are_rejected_before_rewriting() {
        assert!(source_contains_multiple_items(
            "<p>1. 第一条新闻</p><p>2. 第二条新闻</p>"
        ));
        assert!(!source_contains_multiple_items(
            "<p>单一事件的普通描述。</p>"
        ));
    }

    #[test]
    fn prompts_are_distinct_by_news_category() {
        assert_ne!(
            NewsKind::Crypto.system_prompt(),
            NewsKind::Entertainment.system_prompt()
        );
    }

    #[test]
    fn news_filter_only_includes_selected_category() {
        assert!(NewsFilter::Crypto.includes(NewsKind::Crypto));
        assert!(!NewsFilter::Crypto.includes(NewsKind::Entertainment));
        assert!(NewsFilter::Entertainment.includes(NewsKind::Entertainment));
        assert!(!NewsFilter::Entertainment.includes(NewsKind::Crypto));
        assert!(NewsFilter::All.includes(NewsKind::Crypto));
        assert!(NewsFilter::All.includes(NewsKind::Entertainment));
        assert_eq!(NewsFilter::from_setting("crypto"), NewsFilter::Crypto);
        assert_eq!(
            NewsFilter::from_setting("entertainment"),
            NewsFilter::Entertainment
        );
        assert_eq!(NewsFilter::from_setting("invalid"), NewsFilter::All);
    }

    #[test]
    fn source_tables_are_fixed_and_internal() {
        assert_eq!(NewsKind::Crypto.table(), "crypto_news");
        assert_eq!(NewsKind::Entertainment.table(), "entertainment_news");
    }
}

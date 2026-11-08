//! AI 回复质量过滤：语言一致性、结尾口头语清理及原有回复规则校验。

use std::{sync::atomic::AtomicBool, time::Duration};

use super::{automation, vision::text_ai};
use super::vision::VisionReplyOptions;

#[derive(Debug)]
pub(super) struct ReplyQualityRejected {
    reason: String,
}

impl std::fmt::Display for ReplyQualityRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "回复连续两次未通过质量过滤，已放弃该帖子：{}", self.reason)
    }
}

impl std::error::Error for ReplyQualityRejected {}

/// 生成并校验回复；首次生成不合格时重试一次，仍不合格就放弃该帖子。
pub(super) fn generate_compliant_reply(
    options: &VisionReplyOptions,
    initial_prompt: &str,
    source_post: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    // 首次生成失败后只允许再生成一次；仍不合格就放弃该条，绝不发送。
    const MAX_ATTEMPTS: usize = 2;
    let mut prompt = initial_prompt.to_string();
    let mut last_error = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let started = std::time::Instant::now();
        let generated = text_ai::send_openai_text_cancellable(
            &options.api_url,
            &options.model,
            "你是一名长期活跃在 X 的普通网友。先判断原帖的主要语言，必须使用原帖的语言回复，不得把外语帖子翻译成中文后回复。自然、简洁地接话，不要像客服、营销号或总结机器人；避免无意义的语气词、口头禅和夸张感叹。只输出回复正文。",
            &prompt,
            Duration::from_secs(120),
            cancel,
        )?;
        automation::log_event(&format!(
            "回复生成请求 {attempt}/{MAX_ATTEMPTS} 完成，耗时 {} ms。",
            started.elapsed().as_millis()
        ));

        match sanitize_reply(&generated, &options.reply_rules, source_post) {
            Ok(reply) => return Ok(reply),
            Err(error) => {
                automation::log_event(&format!(
                    "回复生成请求 {attempt}/{MAX_ATTEMPTS} 未通过质量过滤：{error:#}"
                ));
                let limit = reply_char_limit(&options.reply_rules).unwrap_or(280);
                let previous_reply = generated.trim();
                last_error = Some(error);
                prompt = format!(
                    "上一版回复未通过质量过滤，必须重新生成。严格使用原帖的主要语言，不得切换为中文；最多 {limit} 个字符；不要在结尾添加哈哈、啊、呀、吧、呢、真香啊等口头语，不要用感叹号收尾。违规原因：{}。上一版：{}。原帖：{}。回复规则：{}。只返回完整回复正文。",
                    last_error.as_ref().unwrap(),
                    previous_reply,
                    source_post,
                    options.reply_rules.trim()
                );
            }
        }
    }

    Err(anyhow::Error::new(ReplyQualityRejected {
        reason: last_error
            .map(|error| format!("{error:#}"))
            .unwrap_or_else(|| "AI 回复未通过质量过滤".to_string()),
    }))
}

/// 统一清理结尾并验证语言、长度和用户规则。
pub(super) fn sanitize_reply(
    text: &str,
    rules: &str,
    source_post: &str,
) -> anyhow::Result<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // 这些属于内容质量问题，必须让模型重写，不能简单删掉后把残句发出去。
    anyhow::ensure!(
        reply_quality_violation(&normalized).is_none(),
        "AI 回复包含低质量/看不懂类表达：{}；要求重新生成",
        reply_quality_violation(&normalized).unwrap_or_default()
    );
    let text = clean_reply_ending(&normalized);
    anyhow::ensure!(!text.is_empty(), "AI 生成的回复为空或清理后为空");

    ensure_reply_language(source_post, &text)?;

    let char_count = text.chars().count();
    let limit = reply_char_limit(rules).unwrap_or(280);
    anyhow::ensure!(
        char_count <= limit,
        "AI 生成的回复有 {char_count} 字，超过回复规则要求的 {limit} 字；已拦截，未输入或发送"
    );
    anyhow::ensure!(
        reply_quality_violation(&text).is_none(),
        "AI 回复包含不合适的口头表达或反问句式：{}；已拦截，要求 AI 重新生成",
        reply_quality_violation(&text).unwrap_or_default()
    );
    if rules.contains("不能有表情") || rules.contains("不要有表情") {
        anyhow::ensure!(
            !text.chars().any(is_emoji),
            "AI 生成的回复包含表情符号，不符合回复规则；已拦截，未输入或发送"
        );
    }
    if rules.contains("不能有感叹号") || rules.contains("不要有感叹号") {
        anyhow::ensure!(
            !text.contains('!') && !text.contains('！'),
            "AI 生成的回复包含感叹号，不符合回复规则；已拦截，未输入或发送"
        );
    }
    if rules.contains("不能有问号") || rules.contains("不要有问号") {
        anyhow::ensure!(
            !text.contains('?') && !text.contains('？'),
            "AI 生成的回复包含问号，不符合回复规则；已拦截，未输入或发送"
        );
    }
    Ok(text)
}

/// 只清理句子末尾的口头语和感叹号，不改动正文中间的表达。
fn clean_reply_ending(input: &str) -> String {
    let mut text = input.trim().to_string();
    loop {
        let before = text.clone();
        text = text.trim_end().to_string();

        while text.ends_with('!') || text.ends_with('！') {
            text.pop();
            text = text.trim_end().to_string();
        }

        const ENDINGS: &[&str] = &[
            "真香啊", "哈哈哈哈", "哈哈哈", "hhh", "哈哈", "呵呵",
            "啊", "呀", "哇", "吧", "呢", "哦", "诶", "欸",
        ];
        let lower = text.to_lowercase();
        if let Some(suffix) = ENDINGS.iter().find(|suffix| lower.ends_with(**suffix)) {
            let new_len = text.len() - suffix.len();
            text.truncate(new_len);
            text = text.trim_end().to_string();
        }

        if text == before {
            break;
        }
    }
    text.trim_end_matches(['，', ',', '。', ';', '；', ':', '：'])
        .trim()
        .to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Han,
    Japanese,
    Korean,
    Latin,
    Cyrillic,
    Arabic,
    Devanagari,
    Thai,
    Other,
    Unknown,
}

fn script_of(text: &str) -> Script {
    let mut counts = [0usize; 9];
    for ch in text.chars() {
        let code = ch as u32;
        let index = match code {
            0x3040..=0x30FF => 1, // Hiragana / Katakana
            0xAC00..=0xD7AF => 2, // Hangul
            0x4E00..=0x9FFF | 0x3400..=0x4DBF => 0, // Han
            0x0041..=0x024F => 3, // Latin
            0x0400..=0x052F => 4, // Cyrillic
            0x0600..=0x06FF | 0x0750..=0x077F => 5, // Arabic
            0x0900..=0x097F => 6, // Devanagari
            0x0E00..=0x0E7F => 7, // Thai
            _ if ch.is_alphabetic() => 8,
            _ => continue,
        };
        counts[index] += 1;
    }

    let (index, count) = counts.iter().enumerate().max_by_key(|(_, count)| *count).unwrap();
    if *count < 2 {
        return Script::Unknown;
    }
    match index {
        0 => Script::Han,
        1 => Script::Japanese,
        2 => Script::Korean,
        3 => Script::Latin,
        4 => Script::Cyrillic,
        5 => Script::Arabic,
        6 => Script::Devanagari,
        7 => Script::Thai,
        _ => Script::Other,
    }
}

/// 以原帖的主要文字系统作为最低语言约束，避免外语帖子收到中文“看不懂”的回复。
fn ensure_reply_language(source_post: &str, reply: &str) -> anyhow::Result<()> {
    let source_script = script_of(source_post);
    let reply_script = script_of(reply);
    if source_script == Script::Unknown || reply_script == Script::Unknown {
        return Ok(());
    }
    let matches = match source_script {
        Script::Japanese => reply_script == Script::Japanese,
        Script::Han => reply_script == Script::Han,
        other => other == reply_script,
    };
    anyhow::ensure!(
        matches,
        "AI 回复语言与原帖不一致（原帖主要文字类型：{source_script:?}，回复：{reply_script:?}）；已拦截，要求按原帖语言重新生成"
    );
    Ok(())
}

fn reply_quality_violation(text: &str) -> Option<&'static str> {
    let compact: String = text
        .chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_ascii_punctuation() && !ch.is_ascii_digit())
        .collect();

    let low_quality_patterns = [
        "撒意思", "啥意思", "撒意思啊", "啥意思啊",
        "真香啊", "字体模糊", "字体不清", "文字模糊", "字太模糊",
        "看不清", "看不懂", "看不明白", "看不出来",
    ];
    if low_quality_patterns.iter().any(|phrase| compact.contains(phrase)) {
        return Some("出现“真香啊/字体模糊/看不清/看不懂”等低质量套话");
    }
    if text.contains('?') || text.contains('？') {
        return Some("使用了疑问或反问标点");
    }
    let rhetorical_patterns = [
        "难道", "岂不是", "这不就是", "不就是", "不是吗", "不是么",
        "还用说吗", "你说是不是", "你觉得呢", "你说呢", "不觉得吗",
    ];
    if rhetorical_patterns.iter().any(|pattern| compact.contains(pattern)) {
        return Some("使用了反问句式");
    }
    None
}

fn reply_char_limit(rules: &str) -> Option<usize> {
    let parsed_limit = (|| {
        let chars: Vec<char> = rules.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            if !chars[index].is_ascii_digit() {
                index += 1;
                continue;
            }
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            if chars.get(index..index + 2) == Some(&['个', '字']) {
                if let Ok(limit) = chars[start..index].iter().collect::<String>().parse() {
                    return Some(limit);
                }
            }
        }
        None
    })();
    Some(parsed_limit.map_or(12, |limit: usize| limit.min(12)))
}

fn is_emoji(ch: char) -> bool {
    matches!(ch as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE00..=0xFE0F)
}

#[cfg(test)]
mod tests {
    use super::{clean_reply_ending, ensure_reply_language, reply_quality_violation, sanitize_reply};

    #[test]
    fn strips_repeated_filler_words_and_exclamation_from_ending() {
        assert_eq!(clean_reply_ending("这次真的很不错哈哈啊！！"), "这次真的很不错");
        assert_eq!(clean_reply_ending("这波真香啊！"), "这波");
        assert_eq!(clean_reply_ending("处理得很稳吧"), "处理得很稳");
        assert_eq!(clean_reply_ending("哈哈"), "");
        assert_eq!(clean_reply_ending("这件事啊，值得关注"), "这件事啊，值得关注");
    }

    #[test]
    fn rejects_low_quality_canned_replies_and_visual_excuses() {
        for reply in [
            "真香啊",
            "这个真香啊",
            "字体模糊看不清",
            "图片太糊了看不懂",
            "看不懂你在说什么",
        ] {
            assert!(reply_quality_violation(reply).is_some(), "未拦截：{reply}");
            assert!(sanitize_reply(reply, "12个字内", "这件事确实值得讨论").is_err());
        }
        assert!(reply_quality_violation("这次分析得很清楚").is_none());
    }

    #[test]
    fn rejects_chinese_reply_to_english_post() {
        assert!(ensure_reply_language("This is an amazing update from today.", "确实很不错").is_err());
        assert!(ensure_reply_language("This is an amazing update from today.", "That is a great update").is_ok());
    }

    #[test]
    fn rejects_chinese_reply_to_japanese_post() {
        assert!(ensure_reply_language("今日はとてもいい天気ですね。", "确实不错").is_err());
        assert!(ensure_reply_language("今日はとてもいい天気ですね。", "本当にいいですね").is_ok());
    }

    #[test]
    fn sanitizes_without_truncating_body_or_changing_internal_words() {
        let rules = "10个字内，回答自然";
        assert_eq!(sanitize_reply("  这次处理得很稳哈哈！ ", rules, "这次处理得很好").unwrap(), "这次处理得很稳");
        assert_eq!(sanitize_reply("这件事啊，值得关注", rules, "这次处理得很好").unwrap(), "这件事啊，值得关注");
    }

    #[test]
    fn keeps_existing_quality_and_rule_checks() {
        let rules = "10个字内，不能有表情符号，不能有感叹号，不能有问号。";
        assert!(sanitize_reply("你好！", rules, "你好世界").is_ok());
        assert!(sanitize_reply("你觉得呢", rules, "你好世界").is_err());
        assert!(sanitize_reply("这是超过十个字的回复内容", rules, "你好世界").is_err());
    }
}

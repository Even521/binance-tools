use anyhow::{Context, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

const DATA_TYPE_TEXT: &str = "text";
const DATA_TYPE_ZSTD: &str = "zstd";
pub const RULE_FORMAT_TEXT: &str = "text";
pub const RULE_FORMAT_MARKDOWN: &str = "markdown";
const COMPRESSION_LEVEL: i32 = 3;

const MARKET_PRODUCTS_DEFAULT_RULE: &str = r#"你是加密货币短线筛选助手。请只基于请求中的 JSON 数据筛选，不要编造外部行情、新闻或实时价格。

输出要求：
- 只选出最值得关注的 1 个币种。
- 严格使用一行格式：$币种 极短理由
- 中文汉字数量建议不少于 60 个且不超过 90 个；$币种代码、空格、标点和英文字母不计入字数。
- 不要标题，不要 Markdown 表格，不要风险提示，不要解释数据来源。
- 不要使用买入、梭哈、暴涨、翻倍、稳赚、必涨、带单、喊单、内幕、财富自由、保证收益、合约、杠杆、冲、无脑买、抄底、逃顶、目标价、止盈止损等容易触发风控或举报的词。
- 不要承诺收益，不要诱导交易，不要写价格预测。
- 不要输出具体价格、涨跌幅数值、目标位、百分比或任何预测性数字。
- 理由必须引用 JSON 里的 2 到 3 个观察点做相对描述，例如成交额在榜单中的位置、流通市值形成的盘面厚度、标签/赛道辨识度、流通供给、24h 高低区间留下的波动感。
- 不要只写热度、成交活跃、板块关注度、流动性这些泛词；要说明为什么这个币比同批标的更值得看。
- 每次措辞要自然变化，禁止反复套用“热度领先、成交活跃、流动性良好、市场情绪积极”这类固定模板。
- 如果数据不足，也必须从已有数据中选 1 个。"#;

const SQUARE_AI_MARKET_DEFAULT_RULE: &str = r#"你是币安广场内容助手。请只基于请求中的 JSON 数据筛选，不要编造外部行情、新闻或实时价格。

输出要求：
- 只选出最值得关注的 1 个币种，必须从 JSON products 中选择。
- 禁止选择今天已经发布过的标题。
- 严格使用一行格式：$币种 极短理由
- 中文汉字数量必须不少于 60 个且不超过 90 个；$币种代码、空格、标点和英文字母不计入字数。
- 不要标题，不要 Markdown 表格，不要风险提示，不要解释数据来源。
- 不要使用买入、梭哈、暴涨、翻倍、稳赚、必涨、带单、喊单、内幕、财富自由、保证收益、合约、杠杆、冲、无脑买、抄底、逃顶、目标价、止盈止损等容易触发币安广场风控或举报的词。
- 不要承诺收益，不要诱导交易，不要写价格预测。
- 不要输出具体价格、涨跌幅数值、目标位、百分比或任何预测性数字。
- 理由必须围绕 JSON 里的 2 到 3 个重点指标写，例如成交额在榜单里的相对位置、流通市值形成的盘面厚度、标签/赛道辨识度、流通供给、流动性或市场热度。
- 表达要像人工观察后的短评，可以带一点主观判断，例如“承接更主动”“辨识度更高”“热度没有散”，但不要夸张。
- 每次措辞要自然变化，禁止反复套用“板块关注度持续提升、成交活跃、流动性良好、市场热度持续走高”这类固定模板。
- 禁止使用“值得跟踪”“值得追踪”“更值得跟踪”“值得留意”“值得关注”作为结尾，结尾要根据指标自然收束。
- 优先写出具体观察角度，例如成交承接、资金容量、赛道扩散、流动性改善、市场讨论焦点，不要只写泛泛热度。"#;

const SQUARE_FALLBACK_REASONS_DEFAULT_RULE: &str = r#"volume|成交额排位比较靠前，资金承接显得更主动，盘面热度不算单薄，配合当前标签辨识度，后续更适合观察换手节奏能否维持
volume|量能表现明显更醒目，短线关注正在聚集，流通体量又给盘面留出一定厚度，若换手节奏保持，讨论焦点可能继续围绕它展开
volume|成交活跃度给人印象较深，资金流动没有明显冷清，结合赛道标签和盘面承接，当前更像是榜单里辨识度较高的标的
volume|交易额和流动性同时靠前，说明市场参与面不窄，流通市值也能提供一定容量，短评角度看更适合放在核心观察池里
volume|盘口热度主要由成交承接支撑，不只是题材情绪推动，叠加流通规模和标签记忆点，后续强弱要看资金是否继续停留
cap|流通体量相对扎实，成交配合度也能跟上，整体不是单纯情绪带动，结合标签辨识度，后面更需要看热度延续性
cap|体量和流动性搭配较均衡，市场承接没有明显断层，赛道信息也比较清楚，短线表现比普通轮动标的更有辨识度
cap|市值基础不算薄弱，交易节奏也没有掉队，流通供给给盘面留出缓冲，整体观感偏稳，适合用更长一点的周期观察
cap|流通规模给了盘面一定厚度，成交活跃度又能配合，标签信息也不模糊，说明当前关注并非只停留在概念层面
cap|资金容量和市场热度匹配度不错，波动里仍有承接，若成交表现没有明显转冷，后续盘面反馈会更容易看清楚
tag|标签覆盖的叙事较清晰，板块辨识度比较高，叠加成交不冷清和流通盘支撑，容易成为当前讨论里的焦点
tag|所属概念有一定记忆点，近期资金参与感增强，成交承接没有明显掉队，若板块情绪延续，热度可能继续停留
tag|赛道标签比较鲜明，市场容易形成共同认知，配合当前交易活跃度和流动性表现，后续反馈可以继续观察
tag|概念标签和资金活跃度形成呼应，不像孤立拉动，流通体量也提供一定承接空间，短线更需要观察板块内扩散效果
tag|叙事辨识度比较直接，成交也能支撑关注度，流动性没有明显拖累，若同赛道继续活跃，它的位置会更突出
default|近期市场关注开始升温，成交和流动性都有一定改善，虽然标签辨识度不算特别强，但盘面已经露出一些可观察的看点
default|短线热度有抬头迹象，资金参与并不算弱，流通体量也没有明显拖累，后续重点看盘面反馈是否能继续确认
default|当前交易活跃度有改善，市场讨论逐步增加，流动性表现没有明显冷清，整体节奏不算沉闷，后面可以多留意变化
default|数据层面没有特别突兀，但成交和热度都有修复迹象，叠加盘面承接没有明显断层，更适合用小周期继续观察强弱
default|盘面表现不算高调，胜在资金参与逐步改善，流动性和成交承接还能配合，若后续承接稳定，关注度可能慢慢打开"#;

const DEFAULT_AI_RULES: &[(&str, &str, &str)] = &[
    ("market_products", "市场榜单", MARKET_PRODUCTS_DEFAULT_RULE),
    (
        "square_ai_market",
        "币安广场 AI 草稿",
        SQUARE_AI_MARKET_DEFAULT_RULE,
    ),
    (
        "square_fallback_reasons",
        "币安广场兜底文案",
        SQUARE_FALLBACK_REASONS_DEFAULT_RULE,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRule {
    pub context_key: String,
    pub label: String,
    pub format: String,
    pub content: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRuleMetadata {
    pub context_key: String,
    pub label: String,
    pub format: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub fn save_ai_rule_blocking(context_key: &str, label: &str, content: &str) -> anyhow::Result<()> {
    save_ai_rule_with_format_blocking(context_key, label, RULE_FORMAT_TEXT, content)
}

pub fn save_ai_rule_with_format_blocking(
    context_key: &str,
    label: &str,
    format: &str,
    content: &str,
) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    save_ai_rule_with_format(&connection, context_key, label, format, content)
}

pub fn seed_default_ai_rules(connection: &Connection) -> anyhow::Result<()> {
    for (context_key, label, content) in DEFAULT_AI_RULES {
        save_ai_rule_if_missing(connection, context_key, label, content)?;
    }
    Ok(())
}

pub fn save_ai_rule(
    connection: &Connection,
    context_key: &str,
    label: &str,
    content: &str,
) -> anyhow::Result<()> {
    save_ai_rule_with_format(connection, context_key, label, RULE_FORMAT_TEXT, content)
}

pub fn save_ai_rule_with_format(
    connection: &Connection,
    context_key: &str,
    label: &str,
    format: &str,
    content: &str,
) -> anyhow::Result<()> {
    let context_key = normalize_context_key(context_key)?;
    let label = normalize_label(label);
    let format = normalize_format(format)?;
    let content = content.trim();
    let compressed = encode_rule_content(content)?;

    connection
        .execute(
            r#"
            INSERT INTO ai_rules (
                context_key,
                label,
                format,
                data_type,
                data,
                enabled,
                created_at,
                updated_at
            )
            VALUES (?1, ?2, ?3, 'zstd', ?4, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(context_key) DO UPDATE SET
                label = excluded.label,
                format = excluded.format,
                data_type = excluded.data_type,
                data = excluded.data,
                enabled = 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![&context_key, label, format, compressed],
        )
        .with_context(|| format!("save AI rule failed: {context_key}"))?;

    Ok(())
}

fn save_ai_rule_if_missing(
    connection: &Connection,
    context_key: &str,
    label: &str,
    content: &str,
) -> anyhow::Result<()> {
    let context_key = normalize_context_key(context_key)?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM ai_rules WHERE context_key = ?1",
            params![&context_key],
            |_| Ok(()),
        )
        .optional()
        .with_context(|| format!("check AI rule exists failed: {context_key}"))?
        .is_some();
    if exists {
        return Ok(());
    }

    save_ai_rule_with_format(connection, &context_key, label, RULE_FORMAT_TEXT, content)
}

pub fn load_ai_rule_blocking(context_key: &str) -> anyhow::Result<Option<AiRule>> {
    let connection = crate::db::open_default_connection()?;
    load_ai_rule(&connection, context_key)
}

pub fn load_ai_rule(connection: &Connection, context_key: &str) -> anyhow::Result<Option<AiRule>> {
    let context_key = normalize_context_key(context_key)?;
    connection
        .query_row(
            r#"
            SELECT context_key, label, format, data_type, data, enabled, created_at, updated_at
            FROM ai_rules
            WHERE context_key = ?1
            "#,
            params![&context_key],
            |row| {
                let data_type: String = row.get(3)?;
                let data: Vec<u8> = row.get(4)?;
                let content = decode_rule_content(&data_type, &data).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Blob,
                        err.into(),
                    )
                })?;

                Ok(AiRule {
                    context_key: row.get(0)?,
                    label: row.get(1)?,
                    format: normalize_format(row.get::<_, String>(2)?.as_str()).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            err.into(),
                        )
                    })?,
                    content,
                    enabled: row.get::<_, i64>(5)? != 0,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()
        .with_context(|| format!("load AI rule failed: {context_key}"))
}

pub fn list_ai_rules_blocking() -> anyhow::Result<Vec<AiRuleMetadata>> {
    let connection = crate::db::open_default_connection()?;
    list_ai_rules(&connection)
}

pub fn list_ai_rules(connection: &Connection) -> anyhow::Result<Vec<AiRuleMetadata>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT context_key, label, format, enabled, created_at, updated_at
            FROM ai_rules
            ORDER BY updated_at DESC, context_key ASC
            "#,
        )
        .context("prepare AI rules list query failed")?;

    statement
        .query_map([], |row| {
            Ok(AiRuleMetadata {
                context_key: row.get(0)?,
                label: row.get(1)?,
                format: normalize_format(row.get::<_, String>(2)?.as_str()).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        err.into(),
                    )
                })?,
                enabled: row.get::<_, i64>(3)? != 0,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .context("query AI rules list failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read AI rules list failed")
}

pub fn set_ai_rule_enabled_blocking(context_key: &str, enabled: bool) -> anyhow::Result<()> {
    let connection = crate::db::open_default_connection()?;
    set_ai_rule_enabled(&connection, context_key, enabled)
}

pub fn set_ai_rule_enabled(
    connection: &Connection,
    context_key: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    let context_key = normalize_context_key(context_key)?;
    connection
        .execute(
            r#"
            UPDATE ai_rules
            SET enabled = ?2, updated_at = CURRENT_TIMESTAMP
            WHERE context_key = ?1
            "#,
            params![&context_key, if enabled { 1 } else { 0 }],
        )
        .with_context(|| format!("update AI rule enabled failed: {context_key}"))?;
    Ok(())
}

fn normalize_context_key(context_key: &str) -> anyhow::Result<String> {
    let context_key = context_key.trim();
    if context_key.is_empty() {
        return Err(anyhow!("AI rule context key cannot be empty"));
    }
    Ok(context_key.to_string())
}

fn normalize_label(label: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        "未命名规则".to_string()
    } else {
        label.to_string()
    }
}

fn normalize_format(format: &str) -> anyhow::Result<String> {
    match format.trim().to_lowercase().as_str() {
        "" | RULE_FORMAT_TEXT => Ok(RULE_FORMAT_TEXT.to_string()),
        RULE_FORMAT_MARKDOWN | "md" => Ok(RULE_FORMAT_MARKDOWN.to_string()),
        value => Err(anyhow!("unknown AI rule format: {value}")),
    }
}

fn encode_rule_content(content: &str) -> anyhow::Result<Vec<u8>> {
    zstd::encode_all(content.as_bytes(), COMPRESSION_LEVEL).context("compress AI rule failed")
}

fn decode_rule_content(data_type: &str, data: &[u8]) -> anyhow::Result<String> {
    let bytes = match data_type {
        DATA_TYPE_ZSTD => zstd::decode_all(data).context("decompress AI rule failed")?,
        DATA_TYPE_TEXT => data.to_vec(),
        value => return Err(anyhow!("unknown AI rule data type: {value}")),
    };

    String::from_utf8(bytes).context("parse AI rule text failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn stores_rule_content_as_zstd() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        save_ai_rule(
            &connection,
            "market_products",
            "市场榜单",
            "优先分析成交额和流动性",
        )
        .unwrap();

        let (data_type, data): (String, Vec<u8>) = connection
            .query_row(
                "SELECT data_type, data FROM ai_rules WHERE context_key = ?1",
                params!["market_products"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(data_type, DATA_TYPE_ZSTD);
        assert_ne!(data, "优先分析成交额和流动性".as_bytes());

        let rule = load_ai_rule(&connection, "market_products")
            .unwrap()
            .unwrap();
        assert_eq!(rule.label, "市场榜单");
        assert_eq!(rule.content, "优先分析成交额和流动性");
        assert!(rule.enabled);
    }

    #[test]
    fn reads_legacy_text_rule_content() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();
        connection
            .execute(
                r#"
                INSERT INTO ai_rules (context_key, label, data_type, data)
                VALUES (?1, ?2, 'text', ?3)
                "#,
                params!["legacy", "Legacy", "plain rule".as_bytes()],
            )
            .unwrap();

        let rule = load_ai_rule(&connection, "legacy").unwrap().unwrap();
        assert_eq!(rule.content, "plain rule");
    }

    #[test]
    fn stores_markdown_rule_format() {
        let connection = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&connection).unwrap();

        save_ai_rule_with_format(
            &connection,
            "markdown_rule",
            "Markdown Rule",
            RULE_FORMAT_MARKDOWN,
            "## Rule\n\n- Use structured bullets",
        )
        .unwrap();

        let rule = load_ai_rule(&connection, "markdown_rule").unwrap().unwrap();
        assert_eq!(rule.format, RULE_FORMAT_MARKDOWN);
        assert_eq!(rule.content, "## Rule\n\n- Use structured bullets");
    }
}

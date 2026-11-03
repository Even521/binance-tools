//! Entertainment RSS News SQLite storage and deduplication.

use anyhow::Context;
use rusqlite::{Connection, params};

#[derive(Debug, Clone)]
pub struct NewsItemEntity {
    pub source: String,
    pub title: String,
    pub url: Option<String>,
    pub summary: Option<String>,
    pub published_at: Option<String>,
    pub collected_at: String,
}

/// 批量插入新闻到 SQLite，利用 UNIQUE 约束与 INSERT OR IGNORE 自动去重。
pub fn insert_news_items(
    connection: &Connection,
    items: &[NewsItemEntity],
) -> anyhow::Result<usize> {
    let mut stmt = connection
        .prepare(
            r#"
        INSERT OR IGNORE INTO entertainment_news (source, title, url, summary, published_at, collected_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        )
        .context("prepare insert_news_items statement")?;

    let mut inserted_count = 0;
    for item in items {
        let affected = stmt.execute(params![
            item.source,
            item.title,
            item.url,
            item.summary,
            item.published_at,
            item.collected_at,
        ])?;
        if affected > 0 {
            inserted_count += 1;
        }
    }
    Ok(inserted_count)
}

/// 保存 X 文案改写结果，只更新中文摘要/翻译字段，不改动中文标题。
pub fn update_news_rewrite(
    connection: &Connection,
    id: i64,
    rewritten_text: &str,
) -> anyhow::Result<()> {
    connection
        .execute(
            "UPDATE entertainment_news SET chinese_summary = ?1 WHERE id = ?2",
            params![rewritten_text, id],
        )
        .context("save news X rewrite failed")?;
    Ok(())
}

/// 更新或保存新闻的中文翻译结果
pub fn update_news_translation(
    connection: &Connection,
    id: i64,
    chinese_title: &str,
    chinese_summary: &str,
) -> anyhow::Result<()> {
    connection
        .execute(
            "UPDATE entertainment_news SET chinese_title = ?1, chinese_summary = ?2 WHERE id = ?3",
            params![chinese_title, chinese_summary, id],
        )
        .context("update news translation failed")?;
    Ok(())
}

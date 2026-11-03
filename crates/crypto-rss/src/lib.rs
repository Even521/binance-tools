use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use feed_rs::parser;
use reqwest::{
    Client,
    header::{ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewsItem {
    pub source: String,
    pub title: String,
    pub url: Option<String>,
    pub summary: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedResult {
    pub source: String,
    pub feed_url: String,
    pub fetched: usize,
    pub unique: usize,
    pub error: Option<String>,
}

pub struct Collector {
    client: Client,
}

impl Collector {
    pub fn new() -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "application/rss+xml, application/atom+xml, application/xml, text/xml, */*",
            ),
        );
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));

        let client = Client::builder()
            .default_headers(headers)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("build HTTP client")?;
        Ok(Self { client })
    }

    pub async fn collect_feed(&self, feed: &FeedConfig) -> Result<Vec<NewsItem>> {
        let candidate_urls = feed_candidate_urls(&feed.url);
        let mut errors = Vec::new();

        for url in candidate_urls {
            match self.collect_feed_from_url(feed, &url).await {
                Ok(items) => return Ok(items),
                Err(err) => errors.push(format!("{url}: {err:#}")),
            }
        }

        anyhow::bail!(
            "all RSS endpoints failed: {}",
            errors.join(" | ")
        )
    }

    async fn collect_feed_from_url(
        &self,
        feed: &FeedConfig,
        url: &str,
    ) -> Result<Vec<NewsItem>> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("request RSS: {url}"))?
            .error_for_status()
            .with_context(|| format!("RSS HTTP status: {url}"))?;
        let bytes = response.bytes().await.context("read RSS body")?;
        let parsed =
            parser::parse(&bytes[..]).with_context(|| format!("parse RSS/Atom: {url}"))?;

        let collected_at = Utc::now();
        let mut items = Vec::new();

        for entry in parsed.entries {
            let title = entry
                .title
                .map(|v| v.content.trim().to_string())
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }

            // Keep Chinese-language headlines; English-only articles are excluded.
            if !contains_chinese(&title) {
                continue;
            }

            let raw_url = entry
                .links
                .iter()
                .find(|link| {
                    link.rel
                        .as_deref()
                        .map(|r| r == "alternate")
                        .unwrap_or(true)
                })
                .map(|link| link.href.clone());
            let item_url = raw_url.as_deref().map(normalize_url);

            let summary = entry
                .summary
                .map(|s| s.content.trim().to_string())
                .filter(|s| !s.is_empty());

            items.push(NewsItem {
                source: feed.name.clone(),
                title,
                url: item_url,
                summary,
                published_at: entry.published.or(entry.updated),
                collected_at,
            });
        }

        Ok(items)
    }

    pub async fn collect_all(&self, feeds: &[FeedConfig]) -> (Vec<NewsItem>, Vec<FeedResult>) {
        let mut all = Vec::new();
        let mut reports = Vec::new();

        for feed in feeds {
            match self.collect_feed(feed).await {
                Ok(items) => {
                    let fetched = items.len();
                    all.extend(items);
                    reports.push(FeedResult {
                        source: feed.name.clone(),
                        feed_url: feed.url.clone(),
                        fetched,
                        unique: 0,
                        error: None,
                    });
                }
                Err(err) => reports.push(FeedResult {
                    source: feed.name.clone(),
                    feed_url: feed.url.clone(),
                    fetched: 0,
                    unique: 0,
                    error: Some(format!("{err:#}")),
                }),
            }
        }

        let mut seen = HashSet::new();
        let mut unique_items = Vec::new();

        for item in all {
            let key = item
                .url
                .clone()
                .unwrap_or_else(|| title_fingerprint(&item.title));
            if seen.insert(key) {
                unique_items.push(item);
            }
        }

        // Report unique counts per source based on the retained items.
        for report in &mut reports {
            report.unique = unique_items
                .iter()
                .filter(|item| item.source == report.source)
                .count();
        }

        (unique_items, reports)
    }
}

// rsshub.app 遇到 403/限流/实例故障时，依次尝试 RSSHub 文档列出的公共实例。
// 只对 rsshub.app 地址启用备用实例，其他 RSS 源保持原地址。
fn feed_candidate_urls(raw: &str) -> Vec<String> {
    const FALLBACK_HOSTS: &[&str] = &[
        "rsshub.rssforever.com",
        "hub.slarker.me",
        "rsshub.pseudoyu.com",
    ];

    let Ok(original) = Url::parse(raw) else {
        return vec![raw.to_string()];
    };

    if original.host_str() != Some("rsshub.app") {
        return vec![raw.to_string()];
    }

    let mut urls = vec![raw.to_string()];
    for host in FALLBACK_HOSTS {
        let mut candidate = original.clone();
        if candidate.set_host(Some(host)).is_ok() {
            urls.push(candidate.to_string());
        }
    }
    urls
}

fn contains_chinese(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

fn normalize_url(raw: &str) -> String {
    let Ok(mut parsed) = Url::parse(raw) else {
        return raw.trim().to_string();
    };
    parsed.set_fragment(None);
    let tracking_keys = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "fbclid",
        "gclid",
    ];
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| !tracking_keys.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    parsed.set_query(None);
    if !kept.is_empty() {
        parsed.query_pairs_mut().extend_pairs(kept);
    }
    parsed.to_string()
}

fn title_fingerprint(title: &str) -> String {
    let normalized: String = title
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}


#[cfg(test)]
mod feed_fallback_tests {
    use super::feed_candidate_urls;

    #[test]
    fn adds_public_instances_for_rsshub_app() {
        let urls = feed_candidate_urls("https://rsshub.app/6park/enter8/gold");
        assert_eq!(urls.len(), 4);
        assert_eq!(urls[0], "https://rsshub.app/6park/enter8/gold");
        assert!(urls.iter().any(|url| url == "https://rsshub.rssforever.com/6park/enter8/gold"));
        assert!(urls.iter().any(|url| url == "https://hub.slarker.me/6park/enter8/gold"));
        assert!(urls.iter().any(|url| url == "https://rsshub.pseudoyu.com/6park/enter8/gold"));
    }

    #[test]
    fn does_not_rewrite_other_feed_hosts() {
        let url = "https://rss.sina.com.cn/ent/star/focus7.xml";
        assert_eq!(feed_candidate_urls(url), vec![url.to_string()]);
    }
}

#[cfg(test)]
mod language_filter_tests {
    use super::contains_chinese;

    #[test]
    fn keeps_chinese_headlines_with_english_tickers() {
        assert!(contains_chinese("比特币 ETF 资金流入 BTC"));
    }

    #[test]
    fn rejects_english_only_headlines() {
        assert!(!contains_chinese("Bitcoin ETF sees record inflows"));
    }
}

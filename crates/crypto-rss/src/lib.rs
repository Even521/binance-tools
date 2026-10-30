use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use feed_rs::parser;
use reqwest::Client;
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
        let client = Client::builder()
            .user_agent("crypto-rss-collector/0.1 (+RSS feed reader)")
            .timeout(Duration::from_secs(20))
            .build()
            .context("build HTTP client")?;
        Ok(Self { client })
    }

    pub async fn collect_feed(&self, feed: &FeedConfig) -> Result<Vec<NewsItem>> {
        let response = self
            .client
            .get(&feed.url)
            .send()
            .await
            .with_context(|| format!("request RSS: {}", feed.url))?
            .error_for_status()
            .with_context(|| format!("RSS HTTP status: {}", feed.url))?;
        let bytes = response.bytes().await.context("read RSS body")?;
        let parsed =
            parser::parse(&bytes[..]).with_context(|| format!("parse RSS/Atom: {}", feed.url))?;

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
            let url = raw_url.as_deref().map(normalize_url);

            let summary = entry
                .summary
                .map(|s| s.content.trim().to_string())
                .filter(|s| !s.is_empty());

            items.push(NewsItem {
                source: feed.name.clone(),
                title,
                url,
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

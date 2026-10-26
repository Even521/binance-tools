use anyhow::{Context, Result};
use crypto_rss::{Collector, FeedConfig};
use std::{env, fs, path::Path};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let config_path = args.get(1).map(String::as_str).unwrap_or("feeds.json");
    let output_path = args.get(2).map(String::as_str).unwrap_or("news.jsonl");

    let config_text = fs::read_to_string(config_path)
        .with_context(|| format!("read config file: {config_path}"))?;
    let feeds: Vec<FeedConfig> = serde_json::from_str(&config_text)
        .with_context(|| format!("parse config file: {config_path}"))?;

    let collector = Collector::new()?;
    let (items, reports) = collector.collect_all(&feeds).await;

    for report in &reports {
        if let Some(error) = &report.error {
            eprintln!("[FAIL] {}: {}", report.source, error);
        } else {
            println!("[OK] {}: fetched={}, unique={}", report.source, report.fetched, report.unique);
        }
    }

    let mut output = String::new();
    for item in &items {
        output.push_str(&serde_json::to_string(item)?);
        output.push('\n');
    }

    let path = Path::new(output_path);
    fs::write(path, output).with_context(|| format!("write output: {}", path.display()))?;
    println!("Collected {} unique items -> {}", items.len(), path.display());

    Ok(())
}

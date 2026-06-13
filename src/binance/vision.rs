use anyhow::{Context, Result, anyhow, bail};
use chrono::{Duration, NaiveDate};
use reqwest::blocking::Client;
use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::time::Duration as StdDuration;
use zip::ZipArchive;

const BINANCE_VISION_BASE: &str = "https://data.binance.vision";

pub const SUPPORTED_SPOT_KLINE_INTERVALS: &[&str] = &[
    "1s", "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w",
    "1mo",
];

#[derive(Clone, Debug)]
pub struct VisionKline {
    pub symbol: String,
    pub interval: String,
    pub open_time: i64,
    pub open_price: f64,
    pub high_price: f64,
    pub low_price: f64,
    pub close_price: f64,
    pub volume: f64,
    pub close_time: i64,
}

#[derive(Clone, Debug)]
pub struct VisionDownloadResult {
    pub klines: Vec<VisionKline>,
    pub cached_files: usize,
    pub downloaded_files: usize,
    pub missing_files: usize,
}

pub fn download_spot_daily_klines_blocking(
    symbol: &str,
    interval: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<VisionDownloadResult> {
    let symbol = normalize_symbol(symbol)?;
    let interval = normalize_interval(interval)?;

    if end < start {
        bail!("End date cannot be earlier than start date");
    }
    let days = (end - start).num_days() + 1;
    if days > 366 {
        bail!("The first backtest version can download at most 366 days at once");
    }

    let client = Client::builder()
        .timeout(StdDuration::from_secs(30))
        .build()
        .context("Failed to create Binance Vision client")?;

    let mut date = start;
    let mut klines = Vec::new();
    let mut cached_files = 0;
    let mut downloaded_files = 0;
    let mut missing_files = 0;
    let mut connection = crate::db::open_default_connection()?;
    let expected_rows_per_day = expected_daily_rows(&interval)?;

    while date <= end {
        let (day_start, day_end) = day_open_time_range_millis(date)?;
        let stored_rows = crate::db::spot::count_spot_klines_in_range(
            &connection,
            &symbol,
            &interval,
            day_start,
            day_end,
        )?;
        if stored_rows >= expected_rows_per_day {
            cached_files += 1;
            date += Duration::days(1);
            continue;
        }

        let cache_path = daily_kline_cache_path(&symbol, &interval, date)?;
        if cache_path.is_file() {
            let csv = fs::read_to_string(&cache_path).with_context(|| {
                format!(
                    "Failed to read cached backtest data: {}",
                    cache_path.display()
                )
            })?;
            let mut cached_klines =
                parse_daily_kline_csv(&symbol, &interval, &csv).with_context(|| {
                    format!(
                        "Failed to parse cached backtest data: {}",
                        cache_path.display()
                    )
                })?;
            cached_files += 1;
            crate::db::spot::upsert_spot_vision_klines(&mut connection, &cached_klines)?;
            klines.append(&mut cached_klines);
            date += Duration::days(1);
            continue;
        }

        let url = daily_kline_url(&symbol, &interval, date);
        let response = client
            .get(&url)
            .send()
            .with_context(|| format!("Failed to download Binance Vision file: {url}"))?;

        if response.status().as_u16() == 404 {
            missing_files += 1;
            date += Duration::days(1);
            continue;
        }
        if !response.status().is_success() {
            bail!(
                "Binance Vision returned status {}: {}",
                response.status(),
                url
            );
        }

        let bytes = response
            .bytes()
            .with_context(|| format!("Failed to read Binance Vision file: {url}"))?;
        let csv = extract_daily_kline_csv(bytes.as_ref())
            .with_context(|| format!("Failed to parse Binance Vision file: {url}"))?;
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create backtest data package directory: {}",
                    parent.display()
                )
            })?;
        }
        fs::write(&cache_path, csv.as_bytes()).with_context(|| {
            format!(
                "Failed to write decompressed backtest data package file: {}",
                cache_path.display()
            )
        })?;
        let mut file_klines = parse_daily_kline_csv(&symbol, &interval, &csv)
            .with_context(|| format!("Failed to parse Binance Vision CSV: {url}"))?;
        downloaded_files += 1;
        crate::db::spot::upsert_spot_vision_klines(&mut connection, &file_klines)?;
        klines.append(&mut file_klines);

        date += Duration::days(1);
    }

    let (start_open_time, _) = day_open_time_range_millis(start)?;
    let (_, end_open_time) = day_open_time_range_millis(end)?;
    klines = crate::db::spot::list_spot_vision_klines_in_range(
        &connection,
        &symbol,
        &interval,
        start_open_time,
        end_open_time,
    )?;

    if klines.is_empty() {
        bail!(
            "No kline data was downloaded. Check symbol, interval, and date range. Binance Vision publishes recent daily files with a delay, so today's file may not exist yet."
        );
    }

    Ok(VisionDownloadResult {
        klines,
        cached_files,
        downloaded_files,
        missing_files,
    })
}

fn daily_kline_url(symbol: &str, interval: &str, date: NaiveDate) -> String {
    format!(
        "{BINANCE_VISION_BASE}/data/spot/daily/klines/{symbol}/{interval}/{symbol}-{interval}-{}.zip",
        date.format("%Y-%m-%d")
    )
}

fn daily_kline_cache_path(symbol: &str, interval: &str, date: NaiveDate) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    Ok(cwd
        .join("data")
        .join("backtest")
        .join("spot")
        .join("daily")
        .join("klines")
        .join(symbol)
        .join(interval)
        .join(format!(
            "{symbol}-{interval}-{}.csv",
            date.format("%Y-%m-%d")
        )))
}

fn day_open_time_range_millis(date: NaiveDate) -> Result<(i64, i64)> {
    let start = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("Invalid date: {date}"))?
        .and_utc()
        .timestamp_millis();
    Ok((start, start + 86_400_000 - 1))
}

fn expected_daily_rows(interval: &str) -> Result<usize> {
    let millis = match interval {
        "1s" => 1_000,
        "1m" => 60_000,
        "3m" => 3 * 60_000,
        "5m" => 5 * 60_000,
        "15m" => 15 * 60_000,
        "30m" => 30 * 60_000,
        "1h" => 60 * 60_000,
        "2h" => 2 * 60 * 60_000,
        "4h" => 4 * 60 * 60_000,
        "6h" => 6 * 60 * 60_000,
        "8h" => 8 * 60 * 60_000,
        "12h" => 12 * 60 * 60_000,
        "1d" => 86_400_000,
        "3d" | "1w" | "1mo" => return Ok(1),
        _ => bail!("Unsupported interval: {interval}"),
    };

    Ok((86_400_000 / millis).max(1) as usize)
}

fn extract_daily_kline_csv(bytes: &[u8]) -> Result<String> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader).context("Failed to open zip file")?;
    if archive.is_empty() {
        bail!("Zip file is empty");
    }

    let mut csv = String::new();
    archive
        .by_index(0)
        .context("Failed to read CSV from zip")?
        .read_to_string(&mut csv)
        .context("Failed to read CSV content")?;

    Ok(csv)
}

fn parse_daily_kline_csv(symbol: &str, interval: &str, csv: &str) -> Result<Vec<VisionKline>> {
    csv.lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| {
            line.chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
        })
        .map(|line| parse_kline_row(symbol, interval, line))
        .collect()
}

fn parse_kline_row(symbol: &str, interval: &str, line: &str) -> Result<VisionKline> {
    let columns: Vec<&str> = line.split(',').collect();
    if columns.len() < 7 {
        return Err(anyhow!("Kline CSV row has too few columns: {line}"));
    }

    Ok(VisionKline {
        symbol: symbol.to_string(),
        interval: interval.to_string(),
        open_time: normalize_timestamp(columns[0].parse()?),
        open_price: columns[1].parse()?,
        high_price: columns[2].parse()?,
        low_price: columns[3].parse()?,
        close_price: columns[4].parse()?,
        volume: columns[5].parse()?,
        close_time: normalize_timestamp(columns[6].parse()?),
    })
}

fn normalize_timestamp(value: i64) -> i64 {
    if value > 10_000_000_000_000 {
        value / 1000
    } else {
        value
    }
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let value = symbol.trim().to_uppercase();
    if value.is_empty() {
        bail!("Symbol cannot be empty");
    }
    Ok(value)
}

fn normalize_interval(interval: &str) -> Result<String> {
    let value = interval.trim().to_lowercase();
    if value.is_empty() {
        bail!("Interval cannot be empty");
    }
    if !SUPPORTED_SPOT_KLINE_INTERVALS.contains(&value.as_str()) {
        bail!(
            "Binance Vision does not support interval `{}`. Supported intervals: {}. For a 120-day backtest, use `1d` and set the date range to 120 days.",
            value,
            SUPPORTED_SPOT_KLINE_INTERVALS.join(", ")
        );
    }
    Ok(value)
}

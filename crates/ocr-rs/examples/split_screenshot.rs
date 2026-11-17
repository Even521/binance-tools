use std::{env, path::PathBuf, process, time::Instant};

use anyhow::{Context, Result, bail};
use image::{ImageReader, RgbImage, Rgba};
use ocr_rs::OcrEngine;

const BENCHMARK_ITERATIONS: usize = 3;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut input = None;
    let mut use_gpu = false;
    let mut compare = false;
    let mut url_only = false;
    for arg in env::args_os().skip(1) {
        let arg_path = PathBuf::from(&arg);
        if arg == "--gpu" {
            use_gpu = true;
        } else if arg == "--compare" {
            compare = true;
        } else if arg == "--url-only" {
            url_only = true;
        } else if input.is_none() {
            input = Some(arg_path);
        } else {
            bail!(
                "Usage: cargo run -p ocr-rs --example split_screenshot -- [screenshot.png] [--gpu|--compare] [--url-only]"
            );
        }
    }
    if compare && use_gpu {
        bail!("Choose either --gpu or --compare");
    }
    let input = input.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("data/x_assistant/screenshots/round-038/20261116-001306-308-01-feed.png")
    });

    let input = input
        .canonicalize()
        .with_context(|| format!("Screenshot does not exist: {}", input.display()))?;
    let image = ImageReader::open(&input)
        .with_context(|| format!("Failed to open {}", input.display()))?
        .decode()
        .context("Failed to decode screenshot")?;
    if compare {
        return compare_full_page_cpu_directml(&image.to_rgb8());
    }
    let (width, height) = (image.width(), image.height());

    // Pixel boundaries follow the visible separators in the supplied screenshot.
    // For other screenshot sizes, use the same normalized positions.
    let scale_x = width as f32 / 1920.0;
    let scale_y = height as f32 / 1032.0;
    let browser_content_top = (120.0 * scale_y).round() as u32;
    let left_sidebar_right = (595.0 * scale_x).round() as u32;
    let timeline_right = (1194.0 * scale_x).round() as u32;
    let regions = [
        ("browser-header", 0, 0, width, browser_content_top),
        (
            "left-sidebar",
            0,
            browser_content_top,
            left_sidebar_right,
            height - browser_content_top,
        ),
        (
            "timeline",
            left_sidebar_right,
            browser_content_top,
            timeline_right - left_sidebar_right,
            height - browser_content_top,
        ),
        (
            "right-sidebar",
            timeline_right,
            browser_content_top,
            width - timeline_right,
            height - browser_content_top,
        ),
    ];

    let mut timeline_crop = None;
    for (name, x, y, region_w, region_h) in regions {
        let output = input.with_file_name(format!(
            "{}-{name}.png",
            input
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("screenshot")
        ));
        let crop = image.crop_imm(x, y, region_w, region_h);
        crop.save(&output)
            .with_context(|| format!("Failed to save {}", output.display()))?;
        if name == "timeline" {
            timeline_crop = Some((crop.to_rgb8(), output.clone()));
        }
        println!(
            "{name}: x={x}..{}, y={y}..{}, size={region_w}x{region_h}, file={}",
            x + region_w,
            y + region_h,
            output.display()
        );
    }

    let mut preview = image.to_rgba8();
    let red = Rgba([255, 0, 0, 255]);
    let cyan = Rgba([0, 220, 255, 255]);
    let yellow = Rgba([255, 220, 0, 255]);
    draw_vertical(&mut preview, left_sidebar_right, red);
    draw_vertical(&mut preview, timeline_right, cyan);
    draw_horizontal(&mut preview, browser_content_top, yellow);
    let preview_path = input.with_file_name(format!(
        "{}-regions-preview.png",
        input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("screenshot")
    ));
    preview.save(&preview_path)?;
    println!("Boundary preview: {}", preview_path.display());

    let (timeline_image, timeline_path) = timeline_crop.context("timeline crop was not created")?;
    println!("\n=== OCR benchmark ===");
    let init_started = Instant::now();
    #[cfg(target_os = "windows")]
    let mut engine = if use_gpu {
        OcrEngine::new_directml().context("Failed to initialize DirectML PP-OCR engine")?
    } else {
        OcrEngine::new().context("Failed to initialize CPU PP-OCR engine")?
    };
    #[cfg(not(target_os = "windows"))]
    let mut engine = {
        if use_gpu {
            bail!("DirectML is only available on Windows");
        }
        OcrEngine::new().context("Failed to initialize CPU PP-OCR engine")?
    };
    let init_elapsed = init_started.elapsed();
    println!(
        "OCR backend: {} (engine init {:?})",
        engine.backend_name(),
        init_elapsed
    );

    if use_gpu && !url_only {
        let started = Instant::now();
        let result = engine.recognize_image_detailed_strict(&image.to_rgb8());
        let elapsed = started.elapsed();
        let result = result.context("DirectML full-page OCR failed without CPU fallback")?;
        println!(
            "DirectML full-page inference: {elapsed:?}, text blocks={}",
            result.text_blocks.len()
        );
        println!("DirectML OCR succeeded. Use the default CPU mode for a same-image comparison.");
        return Ok(());
    }

    // Try the address-bar row first, then use the wider top crop used by the app
    // when Chrome's small unzoomed URL text is not recognized on its own.
    let url_crops = [
        (
            "address-text",
            (width as f32 * 0.05).round() as u32,
            (42.0 * scale_y).round() as u32,
            (width as f32 * 0.45).round() as u32,
            (38.0 * scale_y).round() as u32,
            3,
        ),
        (
            "address-bar",
            0,
            (42.0 * scale_y).round() as u32,
            width,
            (38.0 * scale_y).round() as u32,
            2,
        ),
        (
            "browser-top-fallback",
            0,
            height * 2 / 100,
            width,
            height * 8 / 100,
            2,
        ),
    ];
    let header_started = Instant::now();
    let mut current_url = None;
    for (label, left, top, crop_width, crop_height, upscale) in url_crops {
        let crop = image.crop_imm(left, top, crop_width, crop_height);
        let crop = image::imageops::resize(
            &crop.to_rgb8(),
            crop.width().saturating_mul(upscale),
            crop.height().saturating_mul(upscale),
            image::imageops::FilterType::Lanczos3,
        );
        let result = engine
            .recognize_image_detailed(&crop)
            .with_context(|| format!("Browser URL OCR failed for {label}"))?;
        println!("URL OCR crop={label}, x={left}..{}, y={top}..{}, size={}x{}", left + crop_width, top + crop_height, crop.width(), crop.height());
        for (index, block) in result.text_blocks.iter().enumerate() {
            println!("  {:03}: {}", index + 1, block.text);
        }
        current_url = result
            .text_blocks
            .iter()
            .find_map(|block| extract_x_url(&block.text));
        if current_url.is_some() {
            break;
        }
    }
    let header_elapsed = header_started.elapsed();
    println!("\n=== Browser header URL OCR ===");
    println!("Header OCR: {header_elapsed:?}");
    println!(
        "Recognized URL: {}",
        current_url.as_deref().unwrap_or("<not found>")
    );
    if url_only {
        return Ok(());
    }

    // Warm both paths once, then benchmark each using the same CPU engine.
    engine
        .recognize_image_with_action_coordinates_and_timeline(&image.to_rgb8(), &input)
        .context("Full-page OCR warmup failed")?;
    engine
        .recognize_image_with_action_coordinates_and_timeline(&timeline_image, &timeline_path)
        .context("Timeline-crop OCR warmup failed")?;

    let mut full_page_times = Vec::with_capacity(BENCHMARK_ITERATIONS);
    let mut timeline_crop_times = Vec::with_capacity(BENCHMARK_ITERATIONS);
    let mut timeline_times = Vec::with_capacity(BENCHMARK_ITERATIONS);
    let full_page_rgb = image.to_rgb8();
    let mut full_page_output = None;
    let mut timeline_output = None;
    for iteration in 0..BENCHMARK_ITERATIONS {
        let started = Instant::now();
        let full_result = engine
            .recognize_image_with_action_coordinates_and_timeline(&full_page_rgb, &input)
            .with_context(|| format!("Full-page OCR iteration {} failed", iteration + 1))?;
        full_page_times.push(started.elapsed());
        if iteration + 1 == BENCHMARK_ITERATIONS {
            full_page_output = Some(full_result);
        }

        let crop_started = Instant::now();
        let timeline_image = image
            .crop_imm(
                left_sidebar_right,
                browser_content_top,
                timeline_right - left_sidebar_right,
                height - browser_content_top,
            )
            .to_rgb8();
        timeline_crop_times.push(crop_started.elapsed());
        let started = Instant::now();
        let (result, parsed, json) = engine
            .recognize_image_with_action_coordinates_and_timeline_for_crop(
                &timeline_image,
                &timeline_path,
            )
            .with_context(|| format!("Timeline-crop OCR iteration {} failed", iteration + 1))?;
        timeline_times.push(started.elapsed());
        if iteration + 1 == BENCHMARK_ITERATIONS {
            timeline_output = Some((result, parsed, json));
        }
    }
    print_timings("Full page", &full_page_times);
    print_timings("Timeline crop prep", &timeline_crop_times);
    print_timings("Timeline crop OCR + parse", &timeline_times);
    let crop_total_times = timeline_crop_times
        .iter()
        .zip(&timeline_times)
        .map(|(crop, ocr)| *crop + *ocr)
        .collect::<Vec<_>>();
    print_timings("Timeline crop total", &crop_total_times);

    let (_full_ocr_result, full_timeline_result, mut full_json) =
        full_page_output.context("Full-page OCR did not produce a result")?;
    let (ocr_result, timeline_result, mut json) =
        timeline_output.context("Timeline OCR did not produce a result")?;
    let current_url_value = current_url
        .clone()
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    full_json["current_url"] = current_url_value.clone();
    json["current_url"] = current_url_value;

    println!("\n=== Full-page vs timeline-crop comparison ===");
    println!(
        "URL: {}",
        current_url.as_deref().unwrap_or("<not found>")
    );
    println!(
        "Post count: full={}, crop={}",
        full_timeline_result.timeline.posts.len(),
        timeline_result.timeline.posts.len()
    );
    let full_reply = &full_json["action_coordinates"]["reply"];
    let crop_reply = &json["action_coordinates"]["reply"];
    let crop_reply_absolute = crop_reply
        .get("x")
        .and_then(serde_json::Value::as_i64)
        .zip(crop_reply.get("y").and_then(serde_json::Value::as_i64))
        .map(|(x, y)| {
            serde_json::json!({
                "x": x + i64::from(left_sidebar_right),
                "y": y + i64::from(browser_content_top),
            })
        })
        .unwrap_or(serde_json::Value::Null);
    println!(
        "Reply action point: full={full_reply}, crop-local={crop_reply}, crop-restored={crop_reply_absolute}"
    );
    for index in 0..full_timeline_result
        .timeline
        .posts
        .len()
        .max(timeline_result.timeline.posts.len())
    {
        let full_post = full_timeline_result.timeline.posts.get(index);
        let crop_post = timeline_result.timeline.posts.get(index);
        match (full_post, crop_post) {
            (Some(full), Some(crop)) => println!(
                "post #{:02}: account_equal={}, author_equal={}, timestamp_equal={}, text_equal={}\n  full account={:?}, text={:?}\n  crop account={:?}, text={:?}",
                index + 1,
                full.account == crop.account,
                full.author_name == crop.author_name,
                full.timestamp == crop.timestamp,
                full.text == crop.text,
                full.account,
                full.text,
                crop.account,
                crop.text,
            ),
            (Some(full), None) => println!(
                "post #{:02}: only in full page: account={:?}, text={:?}",
                index + 1,
                full.account,
                full.text
            ),
            (None, Some(crop)) => println!(
                "post #{:02}: only in crop: account={:?}, text={:?}",
                index + 1,
                crop.account,
                crop.text
            ),
            (None, None) => unreachable!(),
        }
    }

    println!("\n=== Timeline OCR raw text (last iteration) ===");
    for (index, block) in ocr_result.text_blocks.iter().enumerate() {
        println!("{:03}: {}", index + 1, block.text);
    }
    println!("\n=== Full-page OCR JSON ===");
    println!("{}", serde_json::to_string_pretty(&full_json)?);
    println!("\n=== Timeline-crop OCR JSON ===");
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

fn compare_full_page_cpu_directml(image: &RgbImage) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    bail!("DirectML is only available on Windows");

    #[cfg(target_os = "windows")]
    {
        let cpu_init_started = Instant::now();
        let mut cpu = OcrEngine::new().context("Failed to initialize CPU PP-OCR engine")?;
        let cpu_init = cpu_init_started.elapsed();

        let gpu_init_started = Instant::now();
        let mut gpu =
            OcrEngine::new_directml().context("Failed to initialize DirectML PP-OCR engine")?;
        let gpu_init = gpu_init_started.elapsed();

        let cpu_warmup_started = Instant::now();
        let cpu_warmup = cpu.recognize_image_detailed(image)?;
        let cpu_warmup_time = cpu_warmup_started.elapsed();

        let gpu_warmup_started = Instant::now();
        let gpu_warmup = gpu.recognize_image_detailed_strict(image)?;
        let gpu_warmup_time = gpu_warmup_started.elapsed();

        let mut cpu_times = Vec::with_capacity(BENCHMARK_ITERATIONS);
        let mut gpu_times = Vec::with_capacity(BENCHMARK_ITERATIONS);
        let mut cpu_result = cpu_warmup;
        let mut gpu_result = gpu_warmup;
        for _ in 0..BENCHMARK_ITERATIONS {
            let started = Instant::now();
            cpu_result = cpu.recognize_image_detailed(image)?;
            cpu_times.push(started.elapsed());

            let started = Instant::now();
            gpu_result = gpu.recognize_image_detailed_strict(image)?;
            gpu_times.push(started.elapsed());
        }

        let average = |times: &[std::time::Duration]| {
            std::time::Duration::from_secs_f64(
                times
                    .iter()
                    .map(std::time::Duration::as_secs_f64)
                    .sum::<f64>()
                    / times.len() as f64,
            )
        };
        let cpu_average = average(&cpu_times);
        let gpu_average = average(&gpu_times);
        let cpu_text = cpu_result
            .text_blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        let gpu_text = gpu_result
            .text_blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();

        println!("Input: full image {}x{}", image.width(), image.height());
        println!(
            "CPU init={cpu_init:?}, warmup={cpu_warmup_time:?}, runs={cpu_times:?}, avg={cpu_average:?}"
        );
        println!(
            "DirectML init={gpu_init:?}, warmup={gpu_warmup_time:?}, runs={gpu_times:?}, avg={gpu_average:?}"
        );
        println!(
            "Text blocks: CPU={}, DirectML={}",
            cpu_text.len(),
            gpu_text.len()
        );
        println!("Text and order exactly equal: {}", cpu_text == gpu_text);
        println!(
            "DirectML speedup: {:.2}x",
            cpu_average.as_secs_f64() / gpu_average.as_secs_f64()
        );

        if cpu_text != gpu_text {
            let max_len = cpu_text.len().max(gpu_text.len());
            for index in 0..max_len {
                if cpu_text.get(index) != gpu_text.get(index) {
                    println!(
                        "Difference #{:03}: CPU={:?}, DirectML={:?}",
                        index + 1,
                        cpu_text.get(index),
                        gpu_text.get(index)
                    );
                }
            }
        }
        Ok(())
    }
}

fn print_timings(label: &str, times: &[std::time::Duration]) {
    let total = times.iter().copied().sum::<std::time::Duration>();
    let average = total / times.len() as u32;
    println!("{label}: runs={times:?}, average={average:?}");
}

fn extract_x_url(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut candidates = Vec::new();

    // Chrome commonly displays the origin without a scheme, e.g. `x.com/home`.
    for (index, _) in lower.match_indices("x.com") {
        let after_host = lower.as_bytes().get(index + "x.com".len()).copied();
        let host_suffix_is_valid = after_host.is_none()
            || matches!(after_host, Some(b'/' | b'?' | b'#'));
        let host_prefix_is_valid = index == 0
            || !lower.as_bytes()[index - 1].is_ascii_alphanumeric()
                && !matches!(lower.as_bytes()[index - 1], b'.' | b'-');
        if host_suffix_is_valid && host_prefix_is_valid {
            let prefix = &lower[..index];
            let start = if prefix.ends_with("https://") {
                index - "https://".len()
            } else if prefix.ends_with("http://") {
                index - "http://".len()
            } else {
                index
            };
            candidates.push(start);
        }
    }

    let start = candidates.into_iter().min()?;
    let candidate = text[start..]
        .split_whitespace()
        .next()?
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '(' | ')' | ',' | ';'))
        .trim_end_matches('.');
    let url_lower = candidate.to_ascii_lowercase();
    let address = url_lower
        .strip_prefix("https://")
        .or_else(|| url_lower.strip_prefix("http://"))
        .unwrap_or(&url_lower);
    let host = address.split(|ch| matches!(ch, '/' | '?' | '#')).next()?;
    (host == "x.com").then(|| candidate.to_string())
}

fn draw_vertical(image: &mut image::RgbaImage, x: u32, color: Rgba<u8>) {
    if x < image.width() {
        for y in 0..image.height() {
            image.put_pixel(x, y, color);
        }
    }
}

fn draw_horizontal(image: &mut image::RgbaImage, y: u32, color: Rgba<u8>) {
    if y < image.height() {
        for x in 0..image.width() {
            image.put_pixel(x, y, color);
        }
    }
}

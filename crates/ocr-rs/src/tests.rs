use std::{env, path::PathBuf};

use image::ImageReader;

use crate::{OcrEngine, extract_x_url, timeline::parse_post_header};

/// Split a browser screenshot into four visual regions and save them beside the source image.
/// Run with `cargo test -p ocr-rs split_timeline_screenshot_into_four_regions -- --nocapture`.
#[test]
fn split_timeline_screenshot_into_four_regions() {
    let screenshot = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/x_assistant/screenshots/round-038/20261116-001306-308-01-feed.png");
    let screenshot = screenshot
        .canonicalize()
        .expect("screenshot does not exist");
    let image = ImageReader::open(&screenshot)
        .expect("failed to open screenshot")
        .decode()
        .expect("failed to decode screenshot");
    let (width, height) = (image.width(), image.height());

    // Approximate X desktop layout for the supplied 1920x1032 browser-client screenshot.
    // Top browser chrome: 0..8% height; content columns: left 0..18%, timeline 18..76%, right 76..100%.
    let header_h = height * 8 / 100;
    let left_w = width * 18 / 100;
    let timeline_right = width * 76 / 100;
    let regions = [
        ("browser-header", 0, 0, width, header_h),
        ("left-sidebar", 0, header_h, left_w, height - header_h),
        (
            "timeline",
            left_w,
            header_h,
            timeline_right - left_w,
            height - header_h,
        ),
        (
            "right-sidebar",
            timeline_right,
            header_h,
            width - timeline_right,
            height - header_h,
        ),
    ];

    for (name, x, y, region_w, region_h) in regions {
        let output = screenshot.with_file_name(format!("20261116-001306-308-01-feed-{name}.png"));
        image
            .crop_imm(x, y, region_w, region_h)
            .save(&output)
            .unwrap_or_else(|error| panic!("failed to save {}: {error}", output.display()));
        println!(
            "{name}: x={x}..{}, y={y}..{}, size={region_w}x{region_h}, file={}",
            x + region_w,
            y + region_h,
            output.display()
        );
    }
}

#[test]
fn extracts_http_urls_without_accepting_host_substrings() {
    assert_eq!(
        extract_x_url("http://x.com/home"),
        Some("http://x.com/home".into())
    );
    assert_eq!(
        extract_x_url("https://x.com/home"),
        Some("https://x.com/home".into())
    );
    assert_eq!(extract_x_url("x.com/home"), Some("x.com/home".into()));
    assert_eq!(
        extract_x_url("6x.com/compose/post"),
        Some("x.com/compose/post".into())
    );
    assert_eq!(extract_x_url("evilx.com/home"), None);
}

#[test]
fn parses_promoted_post_header_and_strips_ad_label() {
    assert_eq!(
        parse_post_header("广告 Gary Du @GaryDu18 · 31分钟"),
        Some((
            "Gary Du".to_string(),
            "@GaryDu18".to_string(),
            Some("31分钟".to_string()),
            true,
        ))
    );
}

#[test]
fn parses_regular_post_header_as_not_ad() {
    assert_eq!(
        parse_post_header("Freesky @Freeskyerp · 1分钟"),
        Some((
            "Freesky".to_string(),
            "@Freeskyerp".to_string(),
            Some("1分钟".to_string()),
            false,
        ))
    );
}

#[test]
fn recognize_configured_compose_reply_page() {
    let Some(image_path) = env::var_os("OCR_COMPOSE_TEST_IMAGE") else {
        eprintln!("skipped: set OCR_COMPOSE_TEST_IMAGE to a compose-page screenshot");
        return;
    };
    let image_path = PathBuf::from(image_path)
        .canonicalize()
        .expect("compose-page screenshot path does not exist");
    let mut engine = OcrEngine::new().expect("failed to initialize PP-OCRv6 Tiny");

    let output = engine
        .recognize_compose_reply_page(&image_path)
        .expect("failed to recognize compose page");
    assert!(output.get("current_url").is_some());
    assert!(output.get("dialog_text_blocks").is_some());
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

#[cfg(target_os = "windows")]
#[test]
fn benchmark_cpu_vs_directml_on_configured_image() {
    use std::time::{Duration, Instant};

    let Some(image_path) = env::var_os("OCR_TEST_IMAGE") else {
        eprintln!("skipped: set OCR_TEST_IMAGE to a screenshot for the CPU/GPU benchmark");
        return;
    };
    let image_path = PathBuf::from(image_path)
        .canonicalize()
        .expect("benchmark image path does not exist");
    let image = ImageReader::open(&image_path)
        .expect("failed to open benchmark image")
        .decode()
        .expect("failed to decode benchmark image")
        .to_rgb8();
    const ITERATIONS: usize = 5;

    let cpu_init_started = Instant::now();
    let mut cpu = OcrEngine::new().expect("failed to initialize CPU PP-OCRv6 Tiny");
    let cpu_init = cpu_init_started.elapsed();
    let cpu_warmup = cpu
        .recognize_image_detailed(&image)
        .expect("CPU OCR warmup failed");
    assert!(
        !cpu_warmup.text_blocks.is_empty(),
        "CPU OCR returned no text"
    );
    let cpu_text = cpu_warmup
        .text_blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>();
    let mut cpu_times = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let started = Instant::now();
        let result = cpu
            .recognize_image_detailed(&image)
            .expect("CPU OCR iteration failed");
        cpu_times.push(started.elapsed());
        assert!(!result.text_blocks.is_empty(), "CPU OCR returned no text");
    }
    let average = |times: &[Duration]| {
        Duration::from_secs_f64(
            times.iter().map(Duration::as_secs_f64).sum::<f64>() / times.len() as f64,
        )
    };
    let cpu_average = average(&cpu_times);
    eprintln!(
        "CPU OCR benchmark image={} warmup=1 iterations={} init={:?} times={:?} avg={:?}",
        image_path.display(),
        ITERATIONS,
        cpu_init,
        cpu_times,
        cpu_average
    );

    let gpu_init_started = Instant::now();
    let mut directml = match OcrEngine::new_directml() {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("DirectML initialization failed: {error:#}");
            return;
        }
    };
    let gpu_init = gpu_init_started.elapsed();
    let gpu_warmup = match directml.recognize_image_detailed_strict(&image) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("DirectML OCR warmup failed after {gpu_init:?} initialization: {error:#}");
            return;
        }
    };
    assert!(
        !gpu_warmup.text_blocks.is_empty(),
        "DirectML OCR returned no text"
    );
    let gpu_text = gpu_warmup
        .text_blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>();
    let mut gpu_times = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let started = Instant::now();
        let result = directml
            .recognize_image_detailed_strict(&image)
            .expect("DirectML OCR iteration failed");
        gpu_times.push(started.elapsed());
        assert!(
            !result.text_blocks.is_empty(),
            "DirectML OCR returned no text"
        );
    }

    let gpu_average = average(&gpu_times);
    eprintln!(
        "OCR benchmark image={} iterations={} cpu_init={:?} directml_init={:?} cpu_times={:?} directml_times={:?} cpu_avg={:?} directml_avg={:?} speedup={:.2}x text_equal={}",
        image_path.display(),
        ITERATIONS,
        cpu_init,
        gpu_init,
        cpu_times,
        gpu_times,
        cpu_average,
        gpu_average,
        cpu_average.as_secs_f64() / gpu_average.as_secs_f64(),
        cpu_text == gpu_text,
    );
}

#[test]
fn recognize_configured_image_and_print_result() {
    let image_path = if let Some(image_path) = env::var_os("OCR_TEST_IMAGE") {
        PathBuf::from(image_path)
    } else {
        eprintln!("skipped: set OCR_TEST_IMAGE to a Chinese or multilingual image file path");
        return;
    };

    let image_path = if image_path.is_absolute() || image_path.is_file() {
        image_path
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(image_path)
    };
    let image_path = image_path
        .canonicalize()
        .expect("test image path does not exist");

    let mut engine = OcrEngine::new().expect("failed to initialize PP-OCRv6 Tiny");

    let (result, output) = engine
        .recognize_path_with_action_coordinates(&image_path)
        .expect("failed to recognize test image and locate action coordinates");
    assert!(
        !result.text_blocks.is_empty(),
        "OCR returned no text; check the image and downloaded models"
    );

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

#[test]
fn recognize_configured_image_gpu_and_print_result() {
    let image_path = if let Some(image_path) = env::var_os("OCR_TEST_IMAGE") {
        PathBuf::from(image_path)
    } else {
        eprintln!("skipped: set OCR_TEST_IMAGE to a Chinese or multilingual image file path");
        return;
    };

    let image_path = if image_path.is_absolute() || image_path.is_file() {
        image_path
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(image_path)
    };
    let image_path = image_path
        .canonicalize()
        .expect("test image path does not exist");

    let mut engine = OcrEngine::new_directml().expect("failed to initialize PP-OCRv6 Tiny");

    let (result, output) = engine
        .recognize_path_with_action_coordinates(&image_path)
        .expect("failed to recognize test image and locate action coordinates");
    assert!(
        !result.text_blocks.is_empty(),
        "OCR returned no text; check the image and downloaded models"
    );

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

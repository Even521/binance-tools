use std::{env, path::PathBuf};

use crate::{OcrEngine, extract_x_url, timeline::parse_post_header};

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

use std::{cell::RefCell, path::Path};

use ocr_rs::{OcrEngine, OcrResult, TextBlock, parse_timeline};

#[derive(Debug)]
struct OcrLine {
    text: String,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

struct OcrDocument {
    text: String,
    lines: Vec<OcrLine>,
    scale_x: f32,
    scale_y: f32,
}

pub(super) struct LocalPostIdentification {
    pub author_name: String,
    pub account_handle: String,
    pub posted_at: String,
    pub post_text: String,
    pub reply_point: (i32, i32),
    pub repost_point: (i32, i32),
    pub like_point: (i32, i32),
    pub post_text_bottom_y: i32,
    pub post_bottom_y: i32,
}

#[derive(Clone)]
struct MetricCandidate {
    center_x: u32,
    center_y: u32,
    left: u32,
    relative_x: f32,
}

pub(super) fn recognize_compose_reply_page(path: &Path) -> anyhow::Result<serde_json::Value> {
    with_cached_ocr_engine(|engine| engine.recognize_compose_reply_page(path))
}

pub(super) fn recognize_current_url(path: &Path) -> anyhow::Result<Option<String>> {
    use image::imageops::crop_imm;

    let image = image::open(path)?.to_rgb8();
    let crop_top = image.height() * 2 / 100;
    let crop_bottom = image.height() * 9 / 100;
    let crop_width = image.width() * 80 / 100;
    anyhow::ensure!(
        crop_bottom > crop_top && crop_width > 0,
        "浏览器地址栏裁剪区域无效"
    );
    let mut address_bar =
        crop_imm(&image, 0, crop_top, crop_width, crop_bottom - crop_top).to_image();
    address_bar = image::imageops::resize(
        &address_bar,
        address_bar.width().saturating_mul(2),
        address_bar.height().saturating_mul(2),
        image::imageops::FilterType::Lanczos3,
    );
    with_cached_ocr_engine(|engine| {
        let result = engine.recognize_image_detailed(&address_bar)?;
        Ok(result
            .text_blocks
            .iter()
            .find_map(|block| extract_x_url(&block.text)))
    })
}

pub(super) fn is_x_home_address(address: &str) -> bool {
    is_x_address_for_path(address, "home")
}

pub(super) fn is_x_compose_post_address(address: &str) -> bool {
    is_x_address_for_path(address, "compose/post")
}

fn is_x_address_for_path(address: &str, expected_path: &str) -> bool {
    let address = address.trim().to_ascii_lowercase();
    let address = address.strip_prefix("https://").unwrap_or(&address);
    let address = address.strip_prefix("www.").unwrap_or(address);
    let path = address.split(['?', '#']).next().unwrap_or_default();
    path == format!("x.com/{expected_path}") || path == format!("x.com/{expected_path}/")
}

fn extract_x_url(text: &str) -> Option<String> {
    let normalized = text.to_ascii_lowercase();
    let mut candidates = [normalized.find("https://"), normalized.find("http://")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for (index, _) in normalized.match_indices("x.com/") {
        let valid_boundary = index == 0
            || index == 1
                && normalized.as_bytes()[0].is_ascii_digit()
                && normalized.as_bytes().get(1) == Some(&b'x')
            || !normalized.as_bytes()[index - 1].is_ascii_alphanumeric()
                && !matches!(normalized.as_bytes()[index - 1], b'.' | b'-');
        if valid_boundary {
            candidates.push(index);
        }
    }
    let start = candidates.into_iter().min()?;
    Some(
        text[start..]
            .split_whitespace()
            .next()?
            .trim_matches(|ch: char| matches!(ch, '\"' | '\'' | '(' | ')' | ',' | ';'))
            .trim_end_matches('.')
            .to_string(),
    )
}

struct ComposeDialogCrop {
    image: image::RgbaImage,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

fn crop_compose_dialog(path: &Path) -> anyhow::Result<ComposeDialogCrop> {
    use image::imageops::crop_imm;

    let image = image::open(path)?.to_rgba8();
    let left = image.width() * 28 / 100;
    let right = image.width() * 72 / 100;
    let top = image.height() * 10 / 100;
    let bottom = image.height() * 82 / 100;
    anyhow::ensure!(right > left && bottom > top, "回复弹窗 OCR 裁剪区域无效");
    let crop_image = crop_imm(&image, left, top, right - left, bottom - top).to_image();
    Ok(ComposeDialogCrop {
        image: crop_image,
        left,
        top,
        width: right - left,
        height: bottom - top,
    })
}

pub(super) fn identify_post_and_action_bar(path: &Path) -> anyhow::Result<LocalPostIdentification> {
    identify_post_and_action_bar_at(path, 0)
}

pub(super) fn identify_post_and_action_bar_at(
    path: &Path,
    post_index: usize,
) -> anyhow::Result<LocalPostIdentification> {
    let source = image::open(path)?.to_rgb8();
    with_cached_ocr_engine(|engine| {
        let (result, page_json) = engine.recognize_image_with_action_coordinates(&source, path)?;
        anyhow::ensure!(
            page_json["current_url"]
                .as_str()
                .is_some_and(is_x_home_address),
            "当前地址不是 x.com/home，可能已有回复/发帖弹窗遮挡时间线；停止帖子解析"
        );
        super::automation::log_event(&format!(
            "阶段 1/3：截图后当前地址与时间线 PP-OCR JSON：{}",
            serde_json::to_string(&serde_json::json!({
                "current_url": page_json["current_url"],
                "timeline": page_json["timeline"],
            }))?
        ));
        let parsed = parse_timeline(&result.text_blocks, &source);
        super::automation::log_event(&format!(
            "OCR诊断：文本框数量={}，解析帖子数量={}",
            result.text_blocks.len(),
            parsed.timeline.posts.len()
        ));

        for (index, candidate) in parsed.timeline.posts.iter().enumerate() {
            super::automation::log_event(&format!(
                "OCR诊断：候选#{} 作者={:?} 账号={:?} 时间={:?} 正文长度={} 评论={:?} 浏览量={:?}",
                index + 1,
                candidate.author_name,
                candidate.account,
                candidate.timestamp,
                candidate.text.trim().chars().count(),
                candidate.metrics.comments,
                candidate.metrics.views,
            ));
        }
        let complete_post_indices = parsed
            .timeline
            .posts
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let has_identity = candidate
                    .author_name
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    && candidate
                        .account
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    && candidate
                        .timestamp
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty());
                let has_text_and_action_row = !candidate.text.trim().is_empty()
                    && (candidate.metrics.comments.is_some() || candidate.metrics.views.is_some());
                (has_identity && has_text_and_action_row).then_some(index)
            })
            .collect::<Vec<_>>();
        let selected_index = complete_post_indices
            .get(post_index)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "PP-OCR 未识别到第 {} 条完整时间线帖子（当前完整帖子数：{}）",
                    post_index + 1,
                    complete_post_indices.len()
                )
            })?;
        let post = &parsed.timeline.posts[selected_index];
        let author_name = post
            .author_name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("PP-OCR 未识别到帖子作者"))?;
        let account_handle = post
            .account
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("PP-OCR 未识别到帖子账号"))?;
        let posted_at = post
            .timestamp
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("PP-OCR 未识别到帖子发布时间"))?;
        anyhow::ensure!(!post.text.trim().is_empty(), "PP-OCR 识别到的帖子正文为空");

        let account_token = account_handle.trim_start_matches('@');
        let header_y = result
            .text_blocks
            .iter()
            .filter(|block| block.text.contains(account_token) && block.text.contains(&posted_at))
            .filter_map(text_block_bounds)
            .map(|bounds| bounds.1)
            .min()
            .ok_or_else(|| anyhow::anyhow!("未能定位 PP-OCR 识别到的帖子头部"))?;

        let next_header_y = parsed
            .timeline
            .posts
            .get(selected_index + 1)
            .and_then(|next_post| next_post.account.as_deref())
            .map(|account| account.trim_start_matches('@'))
            .and_then(|next_account| {
                result
                    .text_blocks
                    .iter()
                    .filter(|block| block.text.contains(next_account))
                    .filter_map(text_block_bounds)
                    .map(|bounds| bounds.1)
                    .filter(|&y| y > header_y)
                    .min()
            })
            .unwrap_or(source.height());

        let mut counts = result
            .text_blocks
            .iter()
            .filter_map(|block| {
                let text = block.text.trim();
                if !is_numeric_count(text) {
                    return None;
                }
                let (left, top, right, bottom) = text_block_bounds(block)?;
                let center_x = (left + right) / 2;
                let center_y = (top + bottom) / 2;
                let relative_x = center_x as f32 / source.width().max(1) as f32;
                (center_y > header_y
                    && center_y < next_header_y
                    && relative_x >= 0.30
                    && relative_x <= 0.68)
                    .then_some(MetricCandidate {
                        center_x,
                        center_y,
                        left,
                        relative_x,
                    })
            })
            .collect::<Vec<_>>();
        counts.sort_by_key(|candidate| (candidate.center_y, candidate.center_x));
        let mut rows: Vec<Vec<MetricCandidate>> = Vec::new();
        for candidate in counts {
            if let Some(row) = rows.last_mut()
                && candidate.center_y.saturating_sub(row[0].center_y) <= 12
            {
                row.push(candidate);
            } else {
                rows.push(vec![candidate]);
            }
        }
        let mut action_row = rows
            .into_iter()
            .filter(|row| {
                row.len() >= 3
                    && row.iter().any(|item| item.relative_x < 0.40)
                    && row
                        .iter()
                        .any(|item| (0.45..0.53).contains(&item.relative_x))
                    && row
                        .iter()
                        .any(|item| (0.52..0.63).contains(&item.relative_x))
            })
            .min_by_key(|row| row[0].center_y);

        let mut action_row = action_row
            .take()
            .ok_or_else(|| anyhow::anyhow!("未识别到完整互动栏，禁止猜测回复坐标"))?;

        if action_row.len() < 3 {
            anyhow::bail!("互动栏识别结果不足 3 个，禁止猜测回复坐标");
        }

        action_row.sort_by_key(|item| item.center_x);
        anyhow::ensure!(
            action_row.iter().all(|item| {
                let y = item.center_y as i32;
                y > header_y as i32 && y < next_header_y as i32
            }),
            "互动栏包含其他帖子的坐标，禁止点击回复"
        );

        let comment = &action_row[0];
        let like = &action_row[action_row.len() - 2];
        let repost = (action_row.len() >= 4).then(|| &action_row[1]);

        let icon_offset = (source.width() as f32 * 0.009).round() as u32;
        let reply_x = comment.left.saturating_add(icon_offset) as i32;
        let like_x = like.left.saturating_add(icon_offset) as i32;
        let repost_x = repost
            .map(|item| item.left.saturating_add(icon_offset) as i32)
            .unwrap_or((reply_x + like_x) / 2);
        let action_y = ((comment.center_y + like.center_y) / 2) as i32;
        anyhow::ensure!(
            action_y > header_y as i32 && action_y < next_header_y as i32,
            "互动栏不属于目标帖子 @{}：Y={}，帖子范围={}..{}",
            account_token,
            action_y,
            header_y,
            next_header_y
        );

        let post_bottom_y = action_y;
        let reply_point = (reply_x, action_y);
        let repost_point = (repost_x, action_y);
        let like_point = (like_x, action_y);

        let annotated_path = path.with_file_name(format!(
            "{}-ocr-coordinates.png",
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("screenshot")
        ));
        save_action_bar_annotation(
            &source,
            &annotated_path,
            reply_point,
            repost_point,
            like_point,
        )?;
        super::automation::log_event(&format!(
            "阶段 1/3：全页 PP-OCR 互动栏候选坐标：回复={reply_point:?}，转发={repost_point:?}，点赞={like_point:?}；已生成标注截图 {}。",
            annotated_path.display()
        ));
        Ok(LocalPostIdentification {
            author_name,
            account_handle,
            posted_at,
            post_text: post.text.clone(),
            reply_point,
            repost_point,
            like_point,
            post_text_bottom_y: action_y.saturating_sub(24).max(header_y as i32),
            post_bottom_y,
        })
    })
}

fn save_action_bar_annotation(
    source: &image::RgbImage,
    path: &Path,
    reply: (i32, i32),
    repost: (i32, i32),
    like: (i32, i32),
) -> anyhow::Result<()> {
    let mut annotated = source.clone();
    for (index, (point, color)) in [
        (reply, image::Rgb([255, 40, 40])),
        (repost, image::Rgb([255, 190, 0])),
        (like, image::Rgb([40, 220, 80])),
    ]
    .into_iter()
    .enumerate()
    {
        let (center_x, center_y) = (point.0, point.1);
        for offset in -10..=10 {
            for (x, y) in [(center_x + offset, center_y), (center_x, center_y + offset)] {
                if x >= 0
                    && y >= 0
                    && (x as u32) < annotated.width()
                    && (y as u32) < annotated.height()
                {
                    annotated.put_pixel(x as u32, y as u32, color);
                }
            }
        }
        let radius_squared = 9 * 9;
        for y in (center_y - 11).max(0)..=(center_y + 11).min(annotated.height() as i32 - 1) {
            for x in (center_x - 11).max(0)..=(center_x + 11).min(annotated.width() as i32 - 1) {
                let dx = x - center_x;
                let dy = y - center_y;
                if (dx * dx + dy * dy - radius_squared).abs() <= 18 {
                    annotated.put_pixel(x as u32, y as u32, color);
                }
            }
        }
        let badge_x = center_x + 13;
        let badge_y = center_y - 13;
        for y in 0..9 {
            for x in 0..9 {
                let px = badge_x + x;
                let py = badge_y + y;
                if px >= 0
                    && py >= 0
                    && (px as u32) < annotated.width()
                    && (py as u32) < annotated.height()
                {
                    annotated.put_pixel(px as u32, py as u32, color);
                }
            }
        }
        let digit = (index + 1) as u8;
        for (row, bits) in DIGIT_GLYPHS[digit as usize].iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    let px = badge_x + 2 + col;
                    let py = badge_y + 1 + row as i32;
                    if px >= 0
                        && py >= 0
                        && (px as u32) < annotated.width()
                        && (py as u32) < annotated.height()
                    {
                        annotated.put_pixel(px as u32, py as u32, image::Rgb([255, 255, 255]));
                    }
                }
            }
        }
    }
    annotated.save(path)?;
    Ok(())
}

const DIGIT_GLYPHS: [[u8; 7]; 4] = [
    [0, 0, 0, 0, 0, 0, 0],
    [4, 12, 4, 4, 4, 4, 14],
    [14, 17, 1, 2, 4, 8, 31],
    [30, 1, 1, 14, 1, 1, 30],
];

pub(super) fn locate_new_posts_notice(path: &Path) -> anyhow::Result<Option<(i32, i32)>> {
    let doc = recognize_document(path)?;
    let (width, height) = image::image_dimensions(path)?;
    let expected = normalize_text("显示帖子");
    Ok(doc
        .lines
        .iter()
        .filter_map(|line| {
            let x = (line.left + line.right) * 0.5 / doc.scale_x;
            let y = (line.top + line.bottom) * 0.5 / doc.scale_y;
            let normalized = normalize_text(&line.text);
            (text_match_ratio(&normalized, &expected) >= 0.75
                && x >= width as f32 * 0.20
                && x <= width as f32 * 0.80
                && y <= height as f32 * 0.40)
                .then_some((x.round() as i32, y.round() as i32))
        })
        .next())
}

pub(super) fn reply_composer_is_open(path: &Path) -> anyhow::Result<bool> {
    let crop = crop_compose_dialog(path)?;
    let doc = recognize_document_image(
        &image::DynamicImage::ImageRgba8(crop.image.clone()),
        4,
        3,
        Some(200),
    )?;
    let expected = normalize_text("发布你的回复");
    Ok(doc
        .lines
        .iter()
        .any(|line| text_match_ratio(&normalize_text(&line.text), &expected) >= 0.75))
}

pub(super) fn recognize_sent_toast(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_sent_confirmation(&doc.text))
}

pub(super) fn recognize_already_sent_toast(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_already_sent_confirmation(&doc.text))
}

pub(super) fn recognize_save_draft_confirmation(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(doc.lines.iter().any(|line| {
        text_match_ratio(&normalize_text(&line.text), &normalize_text("保存帖子")) >= 0.75
    }))
}

pub(super) fn locate_discard_draft_button(path: &Path) -> anyhow::Result<Option<(i32, i32)>> {
    use image::imageops::crop_imm;

    let screenshot = image::open(path)?.to_rgb8();
    let crop_left = screenshot.width() * 30 / 100;
    let crop_right = screenshot.width() * 70 / 100;
    let crop_top = screenshot.height() * 32 / 100;
    let crop_bottom = screenshot.height() * 76 / 100;
    anyhow::ensure!(
        crop_right > crop_left && crop_bottom > crop_top,
        "草稿确认框 OCR 裁剪区域无效"
    );
    let crop = crop_imm(
        &screenshot,
        crop_left,
        crop_top,
        crop_right - crop_left,
        crop_bottom - crop_top,
    )
    .to_image();
    let crop = image::DynamicImage::ImageRgb8(crop);

    for (scale_num, threshold) in [(2, Some(200)), (3, None)] {
        let doc = recognize_document_image(&crop, scale_num, 1, threshold)?;
        let save_prompt = doc.lines.iter().any(|line| {
            text_match_ratio(&normalize_text(&line.text), &normalize_text("保存帖子")) >= 0.75
        });
        if !save_prompt {
            continue;
        }

        let discard_button = doc.lines.iter().find_map(|line| {
            let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
            let center_y = (line.top + line.bottom) * 0.5 / doc.scale_y;
            (text_match_ratio(&normalize_text(&line.text), &normalize_text("放弃")) >= 0.75
                && center_x >= (crop_right - crop_left) as f32 * 0.25
                && center_x <= (crop_right - crop_left) as f32 * 0.75
                && center_y >= (crop_bottom - crop_top) as f32 * 0.50
                && center_y <= (crop_bottom - crop_top) as f32 * 0.95)
                .then_some((
                    (center_x + crop_left as f32).round() as i32,
                    (center_y + crop_top as f32).round() as i32,
                ))
        });
        if discard_button.is_some() {
            return Ok(discard_button);
        }
    }
    Ok(None)
}

pub(super) fn recognize_reload_confirmation(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_reload_confirmation(&doc.text))
}

pub(super) fn recognize_rate_limit(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_rate_limit(&doc.text))
}

pub(super) fn recognize_retryable_send_error(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_retryable_send_error(&doc.text)
        || doc
            .lines
            .iter()
            .any(|line| contains_retryable_send_error(&line.text)))
}

pub(super) fn verify_reply_tooltip(
    path: &Path,
    target_x: i32,
    target_y: i32,
) -> anyhow::Result<bool> {
    use image::imageops::crop_imm;

    let screenshot = image::open(path)?.to_rgba8();
    let crop_left = (target_x - 150).max(0) as u32;
    let crop_top = (target_y - 120).max(0) as u32;
    let crop_right = (target_x + 150).max(0).min(screenshot.width() as i32) as u32;
    let crop_bottom = (target_y + 120).max(0).min(screenshot.height() as i32) as u32;
    anyhow::ensure!(
        crop_right > crop_left && crop_bottom > crop_top,
        "悬停 OCR 裁剪区域无效"
    );

    let crop_path = path.with_file_name(format!(
        "{}-reply-tooltip-crop.png",
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("screenshot")
    ));
    crop_imm(
        &screenshot,
        crop_left,
        crop_top,
        crop_right - crop_left,
        crop_bottom - crop_top,
    )
    .to_image()
    .save(&crop_path)?;

    let modes = [
        (3, 1, None, "局部 3 倍灰度"),
        (4, 1, Some(200), "局部 4 倍阈值 200"),
    ];
    for (scale_num, scale_den, threshold, mode) in modes {
        let doc = recognize_document_with_options(&crop_path, scale_num, scale_den, threshold)?;
        let nearby = doc
            .lines
            .iter()
            .filter_map(|line| {
                let x = (line.left + line.right) * 0.5 / doc.scale_x + crop_left as f32;
                let y = (line.top + line.bottom) * 0.5 / doc.scale_y + crop_top as f32;
                ((x - target_x as f32).abs() <= 120.0
                    && y >= target_y as f32 - 90.0
                    && y <= target_y as f32 + 90.0)
                    .then_some((line.text.as_str(), x, y))
            })
            .collect::<Vec<_>>();
        let verified = nearby
            .iter()
            .any(|(text, x, y)| reply_tooltip_matches_target(text, *x, *y, target_x, target_y));
        let nearby_text = nearby
            .iter()
            .map(|(text, x, y)| format!("{text}@({x:.0},{y:.0})"))
            .collect::<Vec<_>>()
            .join(" | ");
        super::automation::log_event(&format!(
            "悬停回复图标 PP-OCR 校验（{mode}）：目标=({target_x},{target_y})，识别到‘回复’提示={verified}；附近文字：{nearby_text}；裁剪图={}。",
            crop_path.display()
        ));
        if verified {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reply_tooltip_matches_target(
    text: &str,
    center_x: f32,
    center_y: f32,
    target_x: i32,
    target_y: i32,
) -> bool {
    normalize_text(text) == normalize_text("回复")
        && (center_x - target_x as f32).abs() <= 120.0
        && center_y >= target_y as f32 - 90.0
        && center_y <= target_y as f32 + 90.0
}

pub(super) fn locate_reply_input(
    path: &Path,
    _screenshot_width: u32,
    _screenshot_height: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    let crop = crop_compose_dialog(path)?;
    let expected = normalize_text("发布你的回复");
    let modes = [
        (4, 3, Some(200), "原始阈值 200"),
        (2, 1, None, "2 倍灰度"),
        (2, 1, Some(230), "2 倍阈值 230"),
        (2, 1, Some(160), "2 倍阈值 160"),
    ];

    for (attempt, (scale_num, scale_den, threshold, mode_name)) in modes.iter().enumerate() {
        let doc = recognize_document_image(
            &image::DynamicImage::ImageRgba8(crop.image.clone()),
            *scale_num,
            *scale_den,
            *threshold,
        )?;
        let all_lines = doc
            .lines
            .iter()
            .map(|line| {
                format!(
                    "{}@({:.0},{:.0})",
                    line.text,
                    (line.left + line.right) * 0.5 / doc.scale_x + crop.left as f32,
                    (line.top + line.bottom) * 0.5 / doc.scale_y + crop.top as f32
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        super::automation::log_event(&format!(
            "阶段 2/3：回复弹窗局部 OCR（{mode_name}）共 {} 行。全文：\n{}\n弹窗识别行及屏幕坐标：{}",
            doc.lines.len(),
            doc.text,
            all_lines
        ));

        let best = doc
            .lines
            .iter()
            .filter_map(|line| {
                let normalized = normalize_text(&line.text);
                let line_len = normalized.chars().count();
                let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
                let center_y = (line.top + line.bottom) * 0.5 / doc.scale_y;
                if line_len < 4
                    || line_len > 28
                    || center_x < crop.width as f32 * 0.10
                    || center_x > crop.width as f32 * 0.90
                    || center_y < crop.height as f32 * 0.10
                    || center_y > crop.height as f32 * 0.90
                {
                    return None;
                }
                let score = text_match_ratio(&normalized, &expected);
                (score >= 0.72).then_some((line, center_x, center_y, score))
            })
            .max_by(|left, right| left.3.total_cmp(&right.3));

        if let Some((line, center_x, center_y, score)) = best {
            if attempt > 0 {
                super::automation::log_event(&format!(
                    "阶段 2/3：通过备用 PP-OCR 模式‘{mode_name}’识别到回复输入占位文字；识别行‘{}’，匹配度 {:.0}%。",
                    line.text,
                    score * 100.0
                ));
            }
            return Ok(Some((
                center_x.round() as i32 + crop.left as i32,
                center_y.round() as i32 + crop.top as i32,
            )));
        }
    }

    super::automation::log_event("阶段 2/3：所有 PP-OCR 预处理方案均未匹配回复输入占位文字。");
    Ok(None)
}

pub(super) fn locate_reply_submit_button(
    path: &Path,
    reply_text: &str,
    _screenshot_width: u32,
    _screenshot_height: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    let crop = crop_compose_dialog(path)?;
    let doc = recognize_document_image(
        &image::DynamicImage::ImageRgba8(crop.image.clone()),
        4,
        3,
        Some(200),
    )?;
    let reply_text = normalize_text(reply_text);
    let matching_lines = doc
        .lines
        .iter()
        .filter(|line| {
            let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
            let center_y = (line.top + line.bottom) * 0.5 / doc.scale_y;
            let recognized = normalize_text(&line.text);
            text_match_ratio(&recognized, &reply_text) >= 0.80
                && center_x >= crop.width as f32 * 0.10
                && center_x <= crop.width as f32 * 0.90
                && center_y >= crop.height as f32 * 0.10
                && center_y <= crop.height as f32 * 0.90
        })
        .collect::<Vec<_>>();
    let normalized_document = normalize_text(&doc.text);
    let reply_text_verified =
        !matching_lines.is_empty() || text_match_ratio(&normalized_document, &reply_text) >= 0.80;
    if !reply_text_verified {
        let candidates = doc
            .lines
            .iter()
            .filter(|line| {
                let x = (line.left + line.right) * 0.5 / doc.scale_x;
                let y = (line.top + line.bottom) * 0.5 / doc.scale_y;
                x >= crop.width as f32 * 0.10
                    && x <= crop.width as f32 * 0.90
                    && y >= crop.height as f32 * 0.10
                    && y <= crop.height as f32 * 0.90
            })
            .map(|line| {
                format!(
                    "{}({:.0},{:.0})",
                    line.text,
                    (line.left + line.right) * 0.5 / doc.scale_x + crop.left as f32,
                    (line.top + line.bottom) * 0.5 / doc.scale_y + crop.top as f32
                )
            })
            .take(30)
            .collect::<Vec<_>>()
            .join(" | ");
        super::automation::log_event(&format!(
            "阶段 2/3：提交前 PP-OCR 未匹配回复文本‘{reply_text}’。输入区域识别行：{candidates}"
        ));
        return Ok(None);
    }

    let ocr_button = doc
        .lines
        .iter()
        .filter_map(|line| {
            let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
            let center_y = ((line.top + line.bottom) * 0.5 - 3.0) / doc.scale_y;
            (normalize_text(&line.text) == normalize_text("回复")
                && center_x >= crop.width as f32 * 0.60
                && center_x <= crop.width as f32 * 0.98
                && center_y >= crop.height as f32 * 0.55
                && center_y <= crop.height as f32 * 0.98)
                .then_some((center_x, center_y))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1));
    if let Some((x, y)) = ocr_button {
        let point = (
            x.round() as i32 + crop.left as i32,
            y.round() as i32 + crop.top as i32,
        );
        super::automation::log_event(&format!(
            "阶段 2/3：弹窗 OCR 直接识别到右下方‘回复’提交按钮，坐标=({}, {})。",
            point.0, point.1
        ));
        return Ok(Some(point));
    }

    let crop_rgb = image::DynamicImage::ImageRgba8(crop.image.clone()).to_rgb8();
    match find_dark_submit_button(&crop_rgb, crop.width, crop.height)? {
        Some((x, y)) => Ok(Some((x + crop.left as i32, y + crop.top as i32))),
        None => {
            super::automation::log_event(
                "阶段 2/3：PP-OCR 已匹配回复文本，但本地图像检测未找到符合尺寸的深色提交按钮。",
            );
            Ok(None)
        }
    }
}

fn recognize_document(path: &Path) -> anyhow::Result<OcrDocument> {
    recognize_document_with_options(path, 4, 3, Some(200))
}

fn recognize_document_with_options(
    path: &Path,
    scale_num: u32,
    scale_den: u32,
    threshold: Option<u8>,
) -> anyhow::Result<OcrDocument> {
    recognize_document_image(&image::open(path)?, scale_num, scale_den, threshold)
}

fn recognize_document_image(
    source: &image::DynamicImage,
    scale_num: u32,
    scale_den: u32,
    threshold: Option<u8>,
) -> anyhow::Result<OcrDocument> {
    anyhow::ensure!(scale_den > 0, "OCR 缩放比例的分母不能为零");
    let grayscale = source.to_luma8();
    let mut enlarged = image::imageops::resize(
        &grayscale,
        source.width().saturating_mul(scale_num) / scale_den,
        source.height().saturating_mul(scale_num) / scale_den,
        image::imageops::FilterType::Lanczos3,
    );
    if let Some(threshold) = threshold {
        for pixel in enlarged.pixels_mut() {
            pixel.0[0] = if pixel.0[0] < threshold { 0 } else { 255 };
        }
    }
    let scale_x = enlarged.width() as f32 / source.width() as f32;
    let scale_y = enlarged.height() as f32 / source.height() as f32;
    let rgb = image::DynamicImage::ImageLuma8(enlarged).to_rgb8();
    with_cached_ocr_engine(|engine| {
        let result = engine.recognize_image_detailed(&rgb)?;
        document_from_result(&result, scale_x, scale_y)
    })
}

fn document_from_result(
    result: &OcrResult,
    scale_x: f32,
    scale_y: f32,
) -> anyhow::Result<OcrDocument> {
    let lines = result
        .text_blocks
        .iter()
        .filter_map(|block| {
            let left = block.box_points.iter().map(|point| point.x).min()?;
            let right = block.box_points.iter().map(|point| point.x).max()?;
            let top = block.box_points.iter().map(|point| point.y).min()?;
            let bottom = block.box_points.iter().map(|point| point.y).max()?;
            Some(OcrLine {
                text: block.text.clone(),
                left: left as f32,
                top: top as f32,
                right: right as f32,
                bottom: bottom as f32,
            })
        })
        .collect::<Vec<_>>();
    let text = result
        .text_blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(OcrDocument {
        text,
        lines,
        scale_x,
        scale_y,
    })
}

fn with_cached_ocr_engine<T>(
    operation: impl FnOnce(&mut OcrEngine) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    thread_local! {
        static OCR_ENGINE: RefCell<Option<OcrEngine>> = const { RefCell::new(None) };
    }

    OCR_ENGINE.with(|cached| {
        let mut cached = cached.borrow_mut();
        if cached.is_none() {
            *cached = Some(OcrEngine::new()?);
        }
        operation(cached.as_mut().expect("OCR engine initialized"))
    })
}

fn normalize_text(text: &str) -> String {
    text.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

fn contains_rate_limit(text: &str) -> bool {
    let normalized = normalize_text(text).to_lowercase();
    [
        "请求过于频繁",
        "操作过于频繁",
        "请稍后再试",
        "暂时限制",
        "rate limit",
        "rate limited",
        "too many requests",
        "temporarily blocked",
    ]
    .iter()
    .any(|phrase| normalized.contains(&normalize_text(phrase).to_lowercase()))
}

fn contains_reload_confirmation(text: &str) -> bool {
    let normalized = normalize_text(text);
    normalized.contains(&normalize_text("重新加载此网站"))
        && normalized.contains(&normalize_text("系统可能不会保存你所做的更改"))
}

fn contains_retryable_send_error(text: &str) -> bool {
    let normalized = normalize_text(text);
    [
        "出错了别担心让我们再试一次",
        "出错了请稍后再尝试发送你的帖子",
    ]
    .iter()
    .any(|message| normalized.contains(&normalize_text(message)))
}

fn contains_post_unavailable_error(text: &str) -> bool {
    let normalized = normalize_text(text);
    ["已被删除", "不可见", "不可用", "此帖子", "找不到"]
        .iter()
        .any(|message| normalized.contains(&normalize_text(message)))
}

pub(super) fn recognize_post_unavailable_error(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_post_unavailable_error(&doc.text)
        || doc
            .lines
            .iter()
            .any(|line| contains_post_unavailable_error(&line.text)))
}

fn contains_already_sent_confirmation(text: &str) -> bool {
    let normalized = normalize_text(text).to_lowercase();
    [
        "你已经发送过了",
        "你已经发送过这条",
        "youvealreadysent",
        "youalreadysent",
    ]
    .iter()
    .any(|phrase| normalized.contains(&normalize_text(phrase).to_lowercase()))
}

fn contains_sent_confirmation(text: &str) -> bool {
    let normalized = normalize_text(text).to_lowercase();
    if ["yourpostwassent", "yourreplywassent"]
        .iter()
        .any(|message| normalized.contains(message))
    {
        return true;
    }

    let Some(sent_index) = normalized.find("已发送") else {
        return false;
    };
    normalized[..sent_index]
        .chars()
        .rev()
        .take(4)
        .any(|ch| matches!(ch, '帖' | '贴' | '回'))
}

fn find_dark_submit_button(
    image: &image::RgbImage,
    screenshot_width: u32,
    screenshot_height: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    let width = image.width() as usize;
    let x_start = screenshot_width as usize * 45 / 100;
    let x_end = (screenshot_width as usize * 82 / 100).min(width);
    let y_start = screenshot_height as usize * 42 / 100;
    let y_end = (screenshot_height as usize * 82 / 100).min(image.height() as usize);
    let mut visited = vec![false; width * image.height() as usize];
    let mut queue = Vec::new();
    let mut candidates = Vec::new();

    for y in y_start..y_end {
        for x in x_start..x_end {
            let index = y * width + x;
            if visited[index] || !is_dark_pixel(&image, x, y) {
                continue;
            }
            visited[index] = true;
            queue.clear();
            queue.push(index);
            let (mut left, mut right, mut top, mut bottom) = (x, x, y, y);
            let mut area = 0usize;

            while let Some(current) = queue.pop() {
                let current_x = current % width;
                let current_y = current / width;
                left = left.min(current_x);
                right = right.max(current_x);
                top = top.min(current_y);
                bottom = bottom.max(current_y);
                area += 1;

                for (next_x, next_y) in [
                    (current_x as isize - 1, current_y as isize),
                    (current_x as isize + 1, current_y as isize),
                    (current_x as isize, current_y as isize - 1),
                    (current_x as isize, current_y as isize + 1),
                ] {
                    if next_x < x_start as isize
                        || next_x >= x_end as isize
                        || next_y < y_start as isize
                        || next_y >= y_end as isize
                    {
                        continue;
                    }
                    let next_index = next_y as usize * width + next_x as usize;
                    if !visited[next_index]
                        && is_dark_pixel(&image, next_x as usize, next_y as usize)
                    {
                        visited[next_index] = true;
                        queue.push(next_index);
                    }
                }
            }

            let component_width = right - left + 1;
            let component_height = bottom - top + 1;
            let aspect = component_width as f32 / component_height.max(1) as f32;
            if (35..=140).contains(&component_width)
                && (18..=72).contains(&component_height)
                && (1.2..=5.0).contains(&aspect)
                && area >= 400
            {
                candidates.push(((left + right) as i32 / 2, (top + bottom) as i32 / 2));
            }
        }
    }

    candidates.sort_by_key(|(x, y)| {
        let dx = (*x - screenshot_width as i32 * 63 / 100).abs();
        let dy = (*y - screenshot_height as i32 * 58 / 100).abs();
        dx + dy
    });
    Ok(candidates.into_iter().next())
}

fn text_block_bounds(block: &TextBlock) -> Option<(u32, u32, u32, u32)> {
    Some((
        block.box_points.iter().map(|point| point.x).min()?,
        block.box_points.iter().map(|point| point.y).min()?,
        block.box_points.iter().map(|point| point.x).max()?,
        block.box_points.iter().map(|point| point.y).max()?,
    ))
}

fn is_numeric_count(text: &str) -> bool {
    text.split_whitespace().last().is_some_and(|token| {
        !token.is_empty()
            && token.chars().any(|ch| ch.is_ascii_digit())
            && token.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
    })
}

fn text_match_ratio(recognized: &str, expected: &str) -> f32 {
    if recognized.contains(expected) {
        return 1.0;
    }
    let expected_chars = expected.chars().collect::<Vec<_>>();
    if expected_chars.is_empty() {
        return 0.0;
    }
    let recognized_chars = recognized.chars().collect::<Vec<_>>();
    let mut matched = 0usize;
    let mut cursor = 0usize;
    for expected_char in &expected_chars {
        if let Some(offset) = recognized_chars[cursor..]
            .iter()
            .position(|recognized_char| recognized_char == expected_char)
        {
            matched += 1;
            cursor += offset + 1;
        }
    }
    matched as f32 / expected_chars.len() as f32
}

fn is_dark_pixel(image: &image::RgbImage, x: usize, y: usize) -> bool {
    let [red, green, blue] = image.get_pixel(x as u32, y as u32).0;
    red < 55 && green < 60 && blue < 65
}

#[cfg(test)]
mod tests {
    use super::{
        contains_already_sent_confirmation, contains_rate_limit, contains_reload_confirmation,
        contains_retryable_send_error, contains_sent_confirmation, extract_x_url,
        identify_post_and_action_bar, is_x_compose_post_address, is_x_home_address,
        locate_discard_draft_button, reply_tooltip_matches_target, text_match_ratio,
    };
    use std::{env, path::Path};

    #[test]
    fn locates_discard_button_in_configured_draft_confirmation() {
        let Some(path) = env::var_os("OCR_DRAFT_TEST_IMAGE") else {
            return;
        };
        let point = locate_discard_draft_button(Path::new(&path))
            .expect("local OCR failed to inspect the draft confirmation");
        assert!(point.is_some(), "OCR did not locate the discard button");
        eprintln!("discard button point: {point:?}");
    }

    #[test]
    fn identifies_configured_timeline_post_with_local_ocr() {
        let Some(path) = env::var_os("OCR_TEST_IMAGE") else {
            return;
        };
        let post = identify_post_and_action_bar(Path::new(&path))
            .expect("local OCR failed to identify the configured timeline post");
        assert!(!post.author_name.trim().is_empty());
        assert!(!post.account_handle.trim().is_empty());
        assert!(!post.posted_at.trim().is_empty());
        assert!(!post.post_text.trim().is_empty());
        assert!(post.reply_point.0 < post.repost_point.0);
        assert!(post.repost_point.0 < post.like_point.0);
        assert!((post.reply_point.1 - post.like_point.1).abs() <= 8);
    }

    #[test]
    fn accepts_only_x_home_as_safe_reply_target() {
        assert!(is_x_home_address("https://x.com/home"));
        assert!(is_x_home_address("https://x.com/home?tab=following"));
        assert!(is_x_home_address("x.com/home"));
        assert!(!is_x_home_address("http://x.com/home"));
        assert!(!is_x_home_address("https://x.com/home/other"));
        assert!(!is_x_home_address("https://example.com/home"));
        assert!(is_x_compose_post_address("https://x.com/compose/post"));
        assert!(is_x_compose_post_address("x.com/compose/post"));
        assert!(!is_x_compose_post_address("https://x.com/compose/tweet"));
        assert!(!is_x_compose_post_address("http://x.com/compose/post"));
        assert!(!is_x_home_address("https://evilx.com/home"));
        assert!(!is_x_home_address("https://x.com.evil.test/home"));
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
    fn rejects_chat_navigation_label_as_reply_tooltip() {
        assert!(!reply_tooltip_matches_target(
            "聊天", 351.0, 607.0, 351, 607
        ));
    }

    #[test]
    fn accepts_reply_text_with_one_ocr_character_error() {
        assert!(text_match_ratio("慢即是快沉淀方能致远", "慢即是快沉淀方能致远") >= 0.8);
        assert!(text_match_ratio("慢即是快沉淀方能致", "慢即是快沉淀方能致远") >= 0.8);
        assert!(text_match_ratio("不相关的回复文本", "慢即是快沉淀方能致远") < 0.8);
    }

    #[test]
    fn manual_reply_coordinate_accepts_nearby_tooltip() {
        assert!(reply_tooltip_matches_target(
            "回复", 668.0, 978.0, 668, 1005
        ));
    }

    #[test]
    fn scrolled_auto_mislocated_coordinate_rejects_distant_tooltip() {
        assert!(!reply_tooltip_matches_target(
            "回复", 668.0, 978.0, 672, 691
        ));
        assert!(!reply_tooltip_matches_target("0 5", 676.0, 749.0, 672, 691));
    }

    #[test]
    fn recognizes_browser_reload_confirmation_without_matching_unrelated_text() {
        assert!(contains_reload_confirmation(
            "重新加载此网站？系统可能不会保存你所做的更改。"
        ));
        assert!(!contains_reload_confirmation("重新加载页面"));
    }

    #[test]
    fn recognizes_explicit_rate_limit_messages_only() {
        assert!(contains_rate_limit("请求过于频繁，请稍后再试"));
        assert!(contains_rate_limit("Too many requests"));
        assert!(!contains_rate_limit("出错了，别担心，让我们再试一次"));
    }

    #[test]
    fn recognizes_x_network_send_error_messages() {
        assert!(contains_retryable_send_error(
            "出错了，别担心，让我们再试一次。"
        ));
        assert!(contains_retryable_send_error(
            "出错了！请稍后再尝试发送你的帖子。"
        ));
        assert!(!contains_retryable_send_error("你的回复已发送。"));
    }

    #[test]
    fn accepts_common_chinese_ocr_confusions_in_sent_toast() {
        assert!(contains_sent_confirmation("你的帖孑已发送。查看"));
        assert!(contains_sent_confirmation("你的回复已发送。查看"));
        assert!(!contains_sent_confirmation("草稿内容仍在编辑框中"));
        assert!(!contains_sent_confirmation("帖子发送失败"));
    }

    #[test]
    fn detects_platform_duplicate_send_warning() {
        assert!(contains_already_sent_confirmation("哇哦！你已经发送过了。"));
        assert!(contains_already_sent_confirmation(
            "You've already sent that."
        ));
        assert!(!contains_already_sent_confirmation("出错了，请再试一次"));
    }
}

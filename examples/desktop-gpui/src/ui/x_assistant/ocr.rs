use std::{cell::RefCell, path::Path};

use ocr_rs::{ActionPoint, OcrEngine, OcrResult, TextBlock};
use std::time::Instant;

#[derive(Debug)]
pub(super) struct OcrLine {
    pub(super) text: String,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) right: f32,
    pub(super) bottom: f32,
}

pub(super) struct OcrDocument {
    pub(super) text: String,
    pub(super) lines: Vec<OcrLine>,
    pub(super) scale_x: f32,
    pub(super) scale_y: f32,
}

pub(super) struct LocalPostIdentification {
    pub author_name: String,
    pub account_handle: String,
    pub posted_at: String,
    pub post_text: String,
    pub reply_point: (i32, i32),
    pub repost_point: Option<(i32, i32)>,
    pub like_point: Option<(i32, i32)>,
    pub post_text_bottom_y: i32,
    pub post_bottom_y: i32,
}

pub(super) fn recognize_compose_reply_page(path: &Path) -> anyhow::Result<serde_json::Value> {
    with_cached_ocr_engine(|engine| engine.recognize_compose_reply_page(path))
}

/// 使用自动回复相同的弹窗局部 OCR 策略定位 X 发帖输入区域。
/// 先裁剪中央 compose 弹窗，再用多种 PP-OCR 预处理识别占位提示，
/// 并把局部坐标映射回原始截图；不再在整页坐标范围内搜索相似文字。
pub(super) fn locate_post_composer_input(path: &Path) -> anyhow::Result<Option<(i32, i32)>> {
    let crop = crop_compose_dialog(path)?;
    let expected = [
        normalize_text("What's happening?").to_lowercase(),
        normalize_text("What is happening?").to_lowercase(),
        normalize_text("有什么新鲜事"),
        normalize_text("有什么新鲜事？"),
        normalize_text("发布你的帖子"),
    ];
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
        let best = doc.lines.iter().filter_map(|line| {
            let normalized = normalize_text(&line.text).to_lowercase();
            let line_len = normalized.chars().count();
            let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
            let center_y = (line.top + line.bottom) * 0.5 / doc.scale_y;
            if line_len < 4
                || line_len > 36
                || center_x < crop.width as f32 * 0.10
                || center_x > crop.width as f32 * 0.90
                || center_y < crop.height as f32 * 0.10
                || center_y > crop.height as f32 * 0.90
            {
                return None;
            }
            let score = expected.iter()
                .map(|phrase| text_match_ratio(&normalized, phrase))
                .fold(0.0_f32, f32::max);
            (score >= 0.72).then_some((center_x, center_y, score, line.text.as_str()))
        }).max_by(|left, right| left.2.total_cmp(&right.2));

        if let Some((center_x, center_y, score, recognized)) = best {
            let point = (
                (center_x + crop.left as f32).round() as i32,
                (center_y + crop.top as f32).round() as i32,
            );
            super::automation::log_event(&format!(
                "[自动发帖 OCR] 弹窗局部输入框识别成功：模式={mode_name}，文本={recognized:?}，匹配度={:.0}%，客户区坐标=({}, {})",
                score * 100.0, point.0, point.1
            ));
            if attempt > 0 {
                super::automation::log_event(&format!(
                    "[自动发帖 OCR] 首选模式未命中，备用模式‘{mode_name}’识别成功"
                ));
            }
            return Ok(Some(point));
        }
    }

    super::automation::log_event(
        "[自动发帖 OCR] compose 弹窗局部 OCR 未匹配到明确的发帖占位文字，停止操作",
    );
    Ok(None)
}
/// 使用自动回复的局部弹窗 OCR 方式定位发帖按钮。
/// 必须同时满足发帖文字匹配和弹窗右下区域约束，不使用猜测坐标。
pub(super) fn locate_post_submit_button(path: &Path) -> anyhow::Result<Option<(i32, i32)>> {
    let crop = crop_compose_dialog(path)?;
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
        let candidate = doc.lines.iter().filter_map(|line| {
            let text = normalize_text(&line.text).to_lowercase();
            let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
            let center_y = (line.top + line.bottom) * 0.5 / doc.scale_y;
            let label_match = text.contains(&normalize_text("发帖"))
                || text.contains(&normalize_text("发布"))
                || text.contains("post");
            // 发帖弹窗的按钮通常位于弹窗右下方，但不同窗口高度下
            // 其相对位置可能高于回复弹窗按钮；按已采集的发帖截图覆盖 35%-90%。
            let in_button_zone = center_x >= crop.width as f32 * 0.60
                && center_x <= crop.width as f32 * 0.98
                && center_y >= crop.height as f32 * 0.35
                && center_y <= crop.height as f32 * 0.90;
            super::automation::log_event(&format!(
                "[自动发帖 OCR] 弹窗提交按钮候选：模式={mode_name}，文本={:?}，局部坐标=({center_x:.0},{center_y:.0})，文字匹配={label_match}，按钮区域匹配={in_button_zone}",
                line.text
            ));
            (label_match && in_button_zone).then_some((center_x, center_y, line.text.as_str()))
        }).max_by(|left, right| left.1.total_cmp(&right.1));

        if let Some((center_x, center_y, recognized)) = candidate {
            let point = (
                (center_x + crop.left as f32).round() as i32,
                (center_y + crop.top as f32).round() as i32,
            );
            super::automation::log_event(&format!(
                "[自动发帖 OCR] 弹窗局部 OCR 识别到提交按钮：模式={mode_name}，文本={recognized:?}，客户区坐标=({}, {})",
                point.0, point.1
            ));
            if attempt > 0 {
                super::automation::log_event(&format!(
                    "[自动发帖 OCR] 首选模式未命中，备用模式‘{mode_name}’识别成功"
                ));
            }
            return Ok(Some(point));
        }
    }

    super::automation::log_event(
        "[自动发帖 OCR] compose 弹窗右下区域未识别到可信的发帖按钮；未点击，禁止猜测坐标",
    );
    Ok(None)
}
/// 只在 compose 弹窗内部核验正文，采用自动回复相同的局部裁剪与多模式 PP-OCR。
/// 不扫描时间线，避免发布后正文出现在主页时被误认为编辑器仍然打开。
pub(super) fn verify_post_composer_content(path: &Path, content: &str) -> anyhow::Result<bool> {
    let crop = crop_compose_dialog(path)?;
    let expected = normalize_text(content.split(" · ").next().unwrap_or(content)).to_lowercase();
    let expected = expected.chars().take(48).collect::<String>();
    anyhow::ensure!(
        expected.chars().count() >= 8,
        "待核验的发帖正文过短，不能可靠验证输入结果"
    );

    let modes = [
        (4, 3, Some(200), "原始阈值 200"),
        (2, 1, None, "2 倍灰度"),
        (2, 1, Some(230), "2 倍阈值 230"),
        (2, 1, Some(160), "2 倍阈值 160"),
    ];
    let mut best_score = 0.0_f32;
    let mut best_text = String::new();

    for (scale_num, scale_den, threshold, mode_name) in modes {
        let doc = recognize_document_image(
            &image::DynamicImage::ImageRgba8(crop.image.clone()),
            scale_num,
            scale_den,
            threshold,
        )?;
        let recognized = normalize_text(&doc.text).to_lowercase();
        let score = text_match_ratio(&recognized, &expected);
        super::automation::log_event(&format!(
            "[自动发帖 OCR] 弹窗正文核验（{mode_name}）：匹配度={:.0}%，要求至少 80%；识别文本={:?}",
            score * 100.0,
            recognized
        ));
        if score > best_score {
            best_score = score;
            best_text = recognized;
        }
        if score >= 0.80 {
            return Ok(true);
        }
    }

    super::automation::log_event(&format!(
        "[自动发帖 OCR] 弹窗正文核验未通过：最高匹配度={:.0}%，最佳识别文本={:?}",
        best_score * 100.0,
        best_text
    ));
    Ok(false)
}
/// 识别 X 发布成功 toast。
/// X 的提示可能出现在页面底部左侧，不能只扫描中央区域；
/// 因此覆盖截图底部全宽，并兼容中文、英文提示文本。
pub(super) fn recognize_post_sent_toast(path: &Path) -> anyhow::Result<bool> {
    use image::imageops::crop_imm;

    let source = image::open(path)?;
    let width = source.width();
    let height = source.height();
    anyhow::ensure!(width >= 800 && height >= 600, "发布结果截图尺寸异常（{width}x{height}）");

    // 旧实现只扫描 x=25%..75%、y=70%..96%，会漏掉位于左下方的 X toast，
    // 也会漏掉贴近窗口底边的提示。新实现扫描整个底部 42% 区域。
    let left = 0;
    let top = height * 58 / 100;
    let crop = crop_imm(&source, left, top, width, height - top).to_image();
    let crop = image::DynamicImage::ImageRgba8(crop);

    // 蓝色提示在深色/浅色背景下对比度不同，保留灰度与多阈值识别。
    let modes = [
        (3, 2, None, "1.5x grayscale"),
        (2, 1, Some(160), "2x threshold-160"),
        (2, 1, Some(210), "2x threshold-210"),
    ];
    let mut last_text = String::new();
    for (scale_num, scale_den, threshold, mode) in modes {
        let doc = recognize_document_image(&crop, scale_num, scale_den, threshold)?;
        let recognized = doc.text.trim();
        let matched = contains_sent_confirmation(recognized);
        super::automation::log_event(&format!(
            "[自动发帖 OCR] 底部全宽成功提示识别（{mode}）：匹配={}，识别文本={:?}",
            matched, recognized
        ));
        last_text = recognized.to_string();
        if matched {
            return Ok(true);
        }
    }

    super::automation::log_event(&format!(
        "[自动发帖 OCR] 底部全宽区域未识别到成功提示，最后识别文本={:?}",
        last_text
    ));
    Ok(false)
}

pub(super) fn recognize_current_url(path: &Path) -> anyhow::Result<Option<String>> {
    use image::imageops::crop_imm;

    let image = image::open(path)?.to_rgb8();
    let width = image.width();
    let height = image.height();

    // 通用策略：覆盖浏览器顶部，不依赖屏幕分辨率或设备类型。
    // 从较窄区域逐步扩大，减少不必要的 OCR 运算。
    let crop_configs = [
        (0, 2, 100, 10),
        (0, 0, 100, 12),
        (0, 0, 100, 16),
        (0, 0, 100, 20),
    ];

    with_cached_ocr_engine(|engine| {
        for (index, &(left_pct, top_pct, right_pct, bottom_pct)) in crop_configs.iter().enumerate()
        {
            let left = width * left_pct / 100;
            let top = height * top_pct / 100;
            let right = width * right_pct / 100;
            let bottom = height * bottom_pct / 100;

            if right <= left || bottom <= top {
                continue;
            }

            let mut crop = crop_imm(&image, left, top, right - left, bottom - top).to_image();

            crop = image::imageops::resize(
                &crop,
                crop.width().saturating_mul(2),
                crop.height().saturating_mul(2),
                image::imageops::FilterType::Lanczos3,
            );

            let result = engine.recognize_image_detailed(&crop)?;

            let recognized = result
                .text_blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" | ");

            super::automation::log_event(&format!(
                "[OCR-地址栏] 第{}次识别，区域={}%-{}%，结果={}",
                index + 1,
                top_pct,
                bottom_pct,
                recognized
            ));

            if let Some(url) = result
                .text_blocks
                .iter()
                .find_map(|block| extract_x_url(&block.text))
            {
                let normalized = url.to_ascii_lowercase();

                // 只接受 x.com 的 HTTP/HTTPS 地址或裸域名地址。
                let valid = normalized.starts_with("https://x.com/")
                    || normalized.starts_with("http://x.com/")
                    || normalized.starts_with("x.com/");

                if valid {
                    super::automation::log_event(&format!("[OCR-地址栏] 通用识别成功：{}", url));
                    return Ok(Some(url));
                }
            }
        }

        super::automation::log_event("[OCR-地址栏] 所有通用裁剪区域均未识别到 X 地址");

        Ok(None)
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
        // OCR 和时间线解析一次完成，不再重复 parse_timeline
        let (result, parsed, page_json) =
            engine.recognize_image_with_action_coordinates_and_timeline(&source, path)?;

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

                // 评论数和浏览量容易因 OCR 漏识别，不应作为帖子完整性的硬性条件。
                // 互动栏坐标会在后续步骤独立识别并进行结构校验。
                let has_text = !candidate.text.trim().is_empty();

                (has_identity && has_text).then_some(index)
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

        let (header_left, header_y, _, _) = result
            .text_blocks
            .iter()
            .filter(|block| block.text.contains(account_token) && block.text.contains(&posted_at))
            .filter_map(text_block_bounds)
            .min_by_key(|bounds| bounds.1)
            .ok_or_else(|| anyhow::anyhow!("未能定位 PP-OCR 识别到的帖子头部"))?;

        // 账号中出现 i/l、o/0 等易混淆字符，或首轮置信度偏低时，
        // 对作者栏单独放大复核。两次结果不一致就停止，不放宽账号匹配。
        let header_confidence = result
            .text_blocks
            .iter()
            .filter(|block| block.text.contains(account_token) && block.text.contains(&posted_at))
            .map(|block| block.text_score)
            .max_by(|left, right| left.total_cmp(right))
            .unwrap_or(0.0);
        let has_ambiguous_handle_char = account_token
            .chars()
            .any(|ch| matches!(ch, 'i' | 'I' | 'l' | 'L' | '1' | 'o' | 'O' | '0'));
        if header_confidence < 0.92 || has_ambiguous_handle_char {
            let crop_top = header_y.saturating_sub(6);
            let crop_bottom = (header_y + 76).min(source.height());
            let crop_left = header_left.saturating_sub(8);
            let crop_right = (header_left + source.width() * 70 / 100).min(source.width());
            anyhow::ensure!(
                crop_right > crop_left && crop_bottom > crop_top,
                "账号二次 OCR 作者栏裁剪区域无效"
            );
            let header_crop = image::imageops::crop_imm(
                &source,
                crop_left,
                crop_top,
                crop_right - crop_left,
                crop_bottom - crop_top,
            )
            .to_image();
            let enlarged_header = image::imageops::resize(
                &header_crop,
                header_crop.width().saturating_mul(2),
                header_crop.height().saturating_mul(2),
                image::imageops::FilterType::Lanczos3,
            );
            let second_pass = engine.recognize_image_detailed(&enlarged_header)?;
            let second_handle = second_pass
                .text_blocks
                .iter()
                .filter_map(|block| extract_account_token(&block.text).map(|value| {
                    (value, block.text_score, block.text.clone())
                }))
                .filter(|(value, _, _)| value.eq_ignore_ascii_case(account_token))
                .max_by(|left, right| left.1.total_cmp(&right.1));
            let Some((verified_handle, confidence, recognized_line)) = second_handle else {
                anyhow::bail!(
                    "账号二次 OCR 未能确认 @{}，首轮置信度 {:.2}；停止以避免回复错账号",
                    account_token,
                    header_confidence
                );
            };
            anyhow::ensure!(
                verified_handle.eq_ignore_ascii_case(account_token),
                "账号二次 OCR 不一致：首轮=@{}，复核=@{}，复核行={:?}，置信度={:.2}；停止以避免回复错账号",
                account_token,
                verified_handle,
                recognized_line,
                confidence
            );
            super::automation::log_event(&format!(
                "[账号二次核验] 首轮=@{}，复核=@{}，首轮置信度={:.2}，复核置信度={:.2}，通过。",
                account_token, verified_handle, header_confidence, confidence
            ));
        }

        // 下一条真正的顶层帖子，应该与当前帖子作者栏基本左对齐。
        // 图片中的引用作者通常会向右缩进，不能作为帖子边界。
        let header_alignment_tolerance = (source.width() * 4 / 100).max(40);

        let next_header_y = parsed
            .timeline
            .posts
            .iter()
            .skip(selected_index + 1)
            .find_map(|next_post| {
                let next_account = next_post.account.as_deref()?.trim_start_matches('@');

                let next_timestamp = next_post.timestamp.as_deref()?;

                result
                    .text_blocks
                    .iter()
                    .filter(|block| {
                        block.text.contains(next_account) && block.text.contains(next_timestamp)
                    })
                    .filter_map(text_block_bounds)
                    .filter(|bounds| {
                        let next_left = bounds.0;
                        let next_y = bounds.1;

                        next_y > header_y
                            && next_left.abs_diff(header_left) <= header_alignment_tolerance
                    })
                    .map(|bounds| bounds.1)
                    .min()
            })
            .unwrap_or(source.height());

        super::automation::log_event(&format!(
            "[帖子边界] 当前作者栏 x={} y={}，下一条顶层帖子 y={}，截图高度={}，横向容差={}",
            header_left,
            header_y,
            next_header_y,
            source.height(),
            header_alignment_tolerance
        ));

        // 直接复用 timeline.rs 已经识别的

        // 保存目标帖子正文，避免后续局部 OCR 时继续依赖原始借用。
        let post_text = post.text.clone();

        // 优先使用整页 OCR 的回复坐标。
        // 只有回复坐标缺失或不属于目标帖子时，才启动局部 OCR。
        let action_coordinates = &post.action_coordinates;

        let is_valid_reply = |point: &ActionPoint| {
            point.x < source.width()
                && point.y < source.height()
                && point.y > header_y
                && point.y < next_header_y
        };

        let (reply_point, repost_point, like_point, action_y) = if let Some(reply) =
            action_coordinates
                .reply
                .as_ref()
                .filter(|point| is_valid_reply(point))
        {
            // 正常情况：继续使用原来的整页 OCR 坐标。
            let is_valid_optional_point = |point: &ActionPoint| {
                point.x > reply.x
                    && point.x < source.width()
                    && point.y < source.height()
                    && point.y > header_y
                    && point.y < next_header_y
                    && point.y.abs_diff(reply.y) <= 8
            };

            let repost = action_coordinates
                .repost
                .as_ref()
                .filter(|point| is_valid_optional_point(point));

            let repost_point = repost.map(|point| (point.x as i32, point.y as i32));

            let like = action_coordinates.like.as_ref().filter(|point| {
                is_valid_optional_point(point)
                    && repost_point.is_none_or(|repost| point.x as i32 > repost.0)
            });

            let reply_point = (reply.x as i32, reply.y as i32);
            let like_point = like.map(|point| (point.x as i32, point.y as i32));

            super::automation::log_event("[互动栏恢复] 整页 OCR 坐标有效，跳过局部 OCR");

            (reply_point, repost_point, like_point, reply.y as i32)
        } else {
            // 异常情况：整页 OCR 没有得到有效回复坐标。
            super::automation::log_event("[互动栏恢复] 整页回复坐标缺失或越界，启动局部 OCR");

            // 1. 裁剪目标帖子区域。
            let crop = super::crop::crop_post_action_region(&source, header_y, next_header_y)?;

            // 保存局部裁剪图片，方便检查互动栏是否被裁掉。
            let crop_debug_path = path.with_file_name(format!(
                "{}-post-action-crop.png",
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("screenshot")
            ));

            crop.image.save(&crop_debug_path)?;

            super::automation::log_event(&format!(
                "[互动栏恢复] 目标账号={} 发布时间={} 原图={}x{} header_y={} next_header_y={} crop_top={} crop_height={} crop_width={} 裁剪图片={}",
                account_handle,
                posted_at,
                source.width(),
                source.height(),
                header_y,
                next_header_y,
                crop.top,
                crop.image.height(),
                crop.image.width(),
                crop_debug_path.display(),
            ));

            // 2. 使用当前缓存的 OCR 引擎识别裁剪图。
            // 不重新初始化引擎，也不重新识别整页。
            let crop_start = Instant::now();

            let (_crop_result, mut crop_parsed, _crop_json) = engine
                .recognize_image_with_action_coordinates_and_timeline_for_crop(&crop.image, path)?;
            let mut coordinate_scale = 1u32;

            // 首轮局部 OCR 未找到目标回复坐标时，仅在失败路径放大 2 倍复核。
            let first_pass_has_reply = crop_parsed.timeline.posts.iter().any(|candidate| {
                let same_account = candidate.account.as_deref().is_some_and(|account| {
                    account.trim_start_matches('@').eq_ignore_ascii_case(account_token)
                });
                same_account
                    && candidate.timestamp.as_deref() == Some(posted_at.as_str())
                    && candidate.action_coordinates.reply.is_some()
            });
            if !first_pass_has_reply {
                super::automation::log_event(
                    "[互动栏恢复] 首轮局部 OCR 未取得目标回复坐标，启动 2 倍放大复核。",
                );
                let enlarged = image::imageops::resize(
                    &crop.image,
                    crop.image.width().saturating_mul(2),
                    crop.image.height().saturating_mul(2),
                    image::imageops::FilterType::Lanczos3,
                );
                let (_, retry_parsed, _) = engine
                    .recognize_image_with_action_coordinates_and_timeline_for_crop(&enlarged, path)?;
                let retry_has_reply = retry_parsed.timeline.posts.iter().any(|candidate| {
                    let same_account = candidate.account.as_deref().is_some_and(|account| {
                        account.trim_start_matches('@').eq_ignore_ascii_case(account_token)
                    });
                    same_account
                        && candidate.timestamp.as_deref() == Some(posted_at.as_str())
                        && candidate.action_coordinates.reply.is_some()
                });
                if retry_has_reply {
                    crop_parsed = retry_parsed;
                    coordinate_scale = 2;
                    super::automation::log_event(
                        "[互动栏恢复] 2 倍放大复核成功，使用复核坐标。",
                    );
                } else {
                    super::automation::log_event(
                        "[互动栏恢复] 2 倍放大复核仍未找到目标回复坐标。",
                    );
                }
            }

            super::automation::log_event(&format!(
                "[互动栏恢复] 局部 OCR（含必要时放大复核）耗时：{} ms",
                crop_start.elapsed().as_millis()
            ));

            // 必须同时匹配同一个账号和发布时间，禁止借用其他帖子的互动栏。
            let recovered_post = crop_parsed
                .timeline
                .posts
                .iter()
                .find(|candidate| {
                    let same_account = candidate.account.as_deref().is_some_and(|account| {
                        account
                            .trim_start_matches('@')
                            .eq_ignore_ascii_case(account_token)
                    });
                    let same_time = candidate.timestamp.as_deref() == Some(posted_at.as_str());
                    same_account && same_time
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("局部 OCR 未能匹配目标帖子 @{}，禁止恢复坐标", account_token)
                })?;

            super::automation::log_event("[互动栏恢复] 目标账号和发布时间核验通过");

            let local_reply = recovered_post
                .action_coordinates
                .reply
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("局部 OCR 两轮均未识别到回复坐标，禁止猜测"))?;
            let local_reply_x = local_reply.x / coordinate_scale;
            let local_reply_y = local_reply.y / coordinate_scale;

            anyhow::ensure!(
                local_reply_x < crop.image.width() && local_reply_y < crop.image.height(),
                "局部回复坐标超出裁剪区域"
            );

            let global_reply_y = super::crop::restore_y(local_reply_y, crop.top)?;

            anyhow::ensure!(
                global_reply_y > header_y
                    && global_reply_y < next_header_y
                    && global_reply_y < source.height(),
                "局部 OCR 回复坐标不属于目标帖子，禁止点击"
            );

            let reply_point = (local_reply_x as i32, global_reply_y as i32);

            // 6. 转发、点赞属于辅助坐标。
            // 不符合位置关系时直接忽略，不影响回复坐标。
            let restore_optional_point = |point: Option<&ActionPoint>| -> Option<(i32, i32)> {
                let point = point?;

                let point_x = point.x / coordinate_scale;
                let point_y = point.y / coordinate_scale;
                if point_x <= local_reply_x
                    || point_x >= crop.image.width()
                    || point_y >= crop.image.height()
                    || point_y.abs_diff(local_reply_y) > 8
                {
                    return None;
                }

                let global_y = super::crop::restore_y(point_y, crop.top).ok()?;

                if global_y <= header_y || global_y >= next_header_y {
                    return None;
                }

                Some((point_x as i32, global_y as i32))
            };

            let repost_point =
                restore_optional_point(recovered_post.action_coordinates.repost.as_ref());

            let mut like_point =
                restore_optional_point(recovered_post.action_coordinates.like.as_ref());

            // 点赞必须在转发右侧。
            if let (Some(repost), Some(like)) = (repost_point, like_point) {
                if like.0 <= repost.0 {
                    like_point = None;
                }
            }

            super::automation::log_event(&format!(
                "[互动栏恢复] 局部坐标还原成功：回复={:?} 转发={:?} 点赞={:?}",
                reply_point, repost_point, like_point,
            ));

            (reply_point, repost_point, like_point, global_reply_y as i32)
        };

        // 无论是整页识别还是局部恢复，都再次验证回复坐标。
        anyhow::ensure!(
            reply_point.0 >= 0
                && reply_point.1 > header_y as i32
                && reply_point.1 < next_header_y as i32
                && (reply_point.0 as u32) < source.width()
                && (reply_point.1 as u32) < source.height(),
            "最终回复坐标未通过目标帖子范围校验，禁止点击"
        );

        let post_bottom_y = action_y;

        super::automation::log_event(&format!(
            "[互动栏坐标复用] 作者={}，账号={}，回复={:?}，转发={:?}，点赞={:?}",
            author_name, account_handle, reply_point, repost_point, like_point,
        ));

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
            post_text,
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
    repost: Option<(i32, i32)>,
    like: Option<(i32, i32)>,
) -> anyhow::Result<()> {
    let mut annotated = source.clone();
    for (index, (point, color)) in [
        (Some(reply), image::Rgb([255, 40, 40])),
        (repost, image::Rgb([255, 190, 0])),
        (like, image::Rgb([40, 220, 80])),
    ]
    .into_iter()
    .enumerate()
    {
        let Some((center_x, center_y)) = point else {
            continue;
        };
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

pub(super) fn recognize_document(path: &Path) -> anyhow::Result<OcrDocument> {
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
        static OCR_ENGINE: RefCell<Option<OcrEngine>> = const {
            RefCell::new(None)
        };
    }

    OCR_ENGINE.with(|cached| {
        let mut cached = cached.borrow_mut();

        // 第一次调用时初始化 OCR 引擎
        if cached.is_none() {
            let init_start = Instant::now();

            let engine = OcrEngine::new_auto()?;
            super::automation::log_event(&format!(
                "[OCR] {} OCR 初始化成功",
                engine.backend_name()
            ));

            let init_ms = init_start.elapsed().as_millis();

            super::automation::log_event(&format!("[OCR] 引擎初始化耗时：{} ms", init_ms));

            *cached = Some(engine);
        }

        // 统计本次 OCR 实际执行耗时
        let ocr_start = Instant::now();

        let result = operation(cached.as_mut().expect("OCR engine initialized"));

        let ocr_ms = ocr_start.elapsed().as_millis();

        match &result {
            Ok(_) => {
                super::automation::log_event(&format!("[OCR] 本次识别完成，耗时：{} ms", ocr_ms));
            }
            Err(err) => {
                super::automation::log_event(&format!(
                    "[OCR] 本次识别失败，耗时：{} ms，错误：{:#}",
                    ocr_ms, err
                ));
            }
        }

        result
    })
}

fn normalize_text(text: &str) -> String {
    text.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

pub(super) fn contains_rate_limit(text: &str) -> bool {
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

pub(super) fn contains_retryable_send_error(text: &str) -> bool {
    let normalized = normalize_text(text).to_lowercase();

    // 优先匹配完整提示；同时兼容 OCR 漏字、标点、空格和换行。
    let exact_phrases = [
        "出错了别担心让我们再试一次",
        "出错了请稍后再尝试发送你的帖子",
        "somethingwentwrongpleasetryagain",
        "somethingwentwrongtryagain",
        "please try again",
    ];
    if exact_phrases
        .iter()
        .any(|phrase| normalized.contains(&normalize_text(phrase).to_lowercase()))
    {
        return true;
    }

    // 中文提示允许少量 OCR 漏字，但必须同时识别“出错”与“再试/再尝试”语义。
    let has_error = normalized.contains("出错") || normalized.contains("错误");
    let has_retry = normalized.contains("再试") || normalized.contains("再尝试");
    let has_reassurance = normalized.contains("别担心") || normalized.contains("稍后");
    has_error && has_retry && has_reassurance
}

pub(super) fn contains_post_unavailable_error(text: &str) -> bool {
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

pub(super) fn contains_already_sent_confirmation(text: &str) -> bool {
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

pub(super) fn contains_sent_confirmation(text: &str) -> bool {
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

fn extract_account_token(text: &str) -> Option<String> {
    let at = text.find('@')?;
    let token = text[at + 1..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .take(16)
        .collect::<String>();
    (!token.is_empty() && token.len() <= 15).then_some(token)
}

fn text_block_bounds(block: &TextBlock) -> Option<(u32, u32, u32, u32)> {
    Some((
        block.box_points.iter().map(|point| point.x).min()?,
        block.box_points.iter().map(|point| point.y).min()?,
        block.box_points.iter().map(|point| point.x).max()?,
        block.box_points.iter().map(|point| point.y).max()?,
    ))
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
        //assert!(post.repost_point.0 < post.like_point.0);
        //assert!((post.reply_point.1 - post.like_point.1).abs() <= 8);
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
        assert!(contains_retryable_send_error("出错了，别担心，让我们再试"));
        assert!(contains_retryable_send_error("出错了！稍后再试一次"));
        assert!(contains_retryable_send_error("Something went wrong, please try again"));
        assert!(!contains_retryable_send_error("出错了，请检查网络连接"));
        assert!(!contains_retryable_send_error("你的回复已发送。"));
    }

    #[test]
    fn accepts_common_chinese_ocr_confusions_in_sent_toast() {
        assert!(contains_sent_confirmation("你的贴已发送。查看"));
        assert!(contains_sent_confirmation("你的帖子已发送。查看"));
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

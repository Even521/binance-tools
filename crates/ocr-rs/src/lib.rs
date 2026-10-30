mod timeline;

#[cfg(test)]
mod tests;

pub use ppocr_rs::{OcrResult, TextBlock};
pub use timeline::{
    ActionPoint, BrowserRegion, LocatedText, PostActionCoordinates, PostMetrics, RightSidebar,
    SidebarSection, TimelineOcrResult, TimelinePost, TimelineRegion, parse_timeline,
};

use std::path::Path;

use anyhow::{Context, Result};
use image::{DynamicImage, ImageReader};
use ppocr_rs::{ModelHub, OcrLite, OcrOptions, PpOcrVersion};

/// Local PP-OCRv6 engine with Chinese and multilingual text recognition.
pub struct OcrEngine {
    engine: OcrLite,
}

impl OcrEngine {
    /// Loads PP-OCRv6 Tiny from the local model cache, downloading it if needed.
    pub fn new() -> Result<Self> {
        let model_hub = ModelHub::with_default_cache()?;
        let models = model_hub.ensure(PpOcrVersion::V6Tiny)?;
        let detection_model = models.det_onnx.to_string_lossy();
        let recognition_model = models.rec_onnx.to_string_lossy();
        let dictionary = models.dict_txt.to_string_lossy();

        let mut engine = OcrLite::new();
        engine.init_models_no_angle(&detection_model, &recognition_model, &dictionary, 4)?;
        Ok(Self { engine })
    }

    /// Loads PP-OCRv6 Tiny and uses Windows DirectML for GPU inference.
    #[cfg(target_os = "windows")]
    pub fn new_directml() -> Result<Self> {
        let model_hub = ModelHub::with_default_cache()?;
        let models = model_hub.ensure(PpOcrVersion::V6Tiny)?;
        let detection_model = models.det_onnx.to_string_lossy();
        let recognition_model = models.rec_onnx.to_string_lossy();
        let dictionary = models.dict_txt.to_string_lossy();

        let mut engine = OcrLite::new();
        engine.init_models_no_angle_with_builder(
            &detection_model,
            &recognition_model,
            &dictionary,
            4,
            directml_session_builder,
        )?;
        Ok(Self { engine })
    }

    /// Recognizes a full page and returns its OCR details plus timeline/action coordinates as JSON.

    /// 从图片路径读取图片，再执行 OCR 和坐标识别。
    /// 保留此方法，兼容原有调用。
    pub fn recognize_path_with_action_coordinates(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(OcrResult, serde_json::Value)> {
        let path = path.as_ref();

        let image = ImageReader::open(path)
            .with_context(|| format!("failed to open image {}", path.display()))?
            .decode()
            .with_context(|| format!("failed to decode image {}", path.display()))?
            .to_rgb8();

        self.recognize_image_with_action_coordinates(&image, path)
    }

    /// 使用已经解码的图片执行 OCR 和坐标识别。
    /// 避免调用方已经读取图片后，再次从磁盘解码。
    pub fn recognize_image_with_action_coordinates(
        &mut self,
        image: &image::RgbImage,
        path: impl AsRef<Path>,
    ) -> Result<(OcrResult, serde_json::Value)> {
        let path = path.as_ref();

        let result = self.recognize_image_detailed(image)?;
        let timeline = parse_timeline(&result.text_blocks, image);

        let text_blocks = result
            .text_blocks
            .iter()
            .filter_map(|block| {
                let left = block.box_points.iter().map(|point| point.x).min()?;
                let top = block.box_points.iter().map(|point| point.y).min()?;
                let right = block.box_points.iter().map(|point| point.x).max()?;
                let bottom = block.box_points.iter().map(|point| point.y).max()?;

                Some(serde_json::json!({
                    "text": block.text,
                    "box": {
                        "left": left,
                        "top": top,
                        "right": right,
                        "bottom": bottom
                    },
                    "center": {
                        "x": (left + right) / 2,
                        "y": (top + bottom) / 2
                    },
                    "confidence": block.text_score,
                }))
            })
            .collect::<Vec<_>>();

        let coordinates = timeline
            .timeline
            .posts
            .first()
            .map(|post| {
                let point = |value: &Option<ActionPoint>| {
                    value.as_ref().map(|p| {
                        serde_json::json!({
                            "x": p.x,
                            "y": p.y
                        })
                    })
                };

                serde_json::json!({
                    "reply": point(&post.action_coordinates.reply),
                    "repost": point(&post.action_coordinates.repost),
                    "like": point(&post.action_coordinates.like),
                    "views": point(&post.action_coordinates.views),
                    "coordinate_source": "timeline_first_post"
                })
            })
            .unwrap_or_else(|| {
                serde_json::json!({
                    "reply": null,
                    "repost": null,
                    "like": null,
                    "views": null,
                    "coordinate_source": "unavailable"
                })
            });

        let current_url = result.text_blocks.iter().find_map(|block| {
            let text = block.text.trim();
            let normalized = text.to_ascii_lowercase();
            let marker = find_x_url_start(&normalized)?;

            let url = text[marker..]
                .split_whitespace()
                .next()?
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '(' | ')' | ',' | ';'));

            Some(url.trim_end_matches('.').to_string())
        });

        let json = serde_json::json!({
            "image": path.display().to_string(),
            "current_url": current_url,
            "image_size": {
                "width": image.width(),
                "height": image.height()
            },
            "text_block_count": text_blocks.len(),
            "text_blocks": text_blocks,
            "browser": timeline.browser,
            "left_navigation": timeline.left_navigation,
            "timeline": timeline.timeline,
            "right_sidebar": timeline.right_sidebar,
            "action_coordinates": coordinates,
        });

        Ok((result, json))
    }

    /// Recognizes only the browser address bar and centered reply composer in one API call.
    /// Returns their recognized text, global screenshot coordinates, current URL, and reply input point.
    pub fn recognize_compose_reply_page(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<serde_json::Value> {
        let path = path.as_ref();
        let image = ImageReader::open(path)
            .with_context(|| format!("failed to open image {}", path.display()))?
            .decode()
            .with_context(|| format!("failed to decode image {}", path.display()))?
            .to_rgb8();
        let width = image.width();
        let height = image.height();

        let address_top = height * 2 / 100;
        let address_bottom = height * 9 / 100;
        let address_width = width * 80 / 100;
        anyhow::ensure!(
            address_bottom > address_top && address_width > 0,
            "browser address bar crop is invalid"
        );
        let address_crop = image::imageops::crop_imm(
            &image,
            0,
            address_top,
            address_width,
            address_bottom - address_top,
        )
        .to_image();
        let address_crop = image::imageops::resize(
            &address_crop,
            address_crop.width().saturating_mul(2),
            address_crop.height().saturating_mul(2),
            image::imageops::FilterType::Lanczos3,
        );
        let address_result = self.recognize_image_detailed(&address_crop)?;
        let current_url = address_result
            .text_blocks
            .iter()
            .find_map(|block| extract_x_url(&block.text));

        let dialog_left = width * 28 / 100;
        let dialog_right = width * 72 / 100;
        let dialog_top = height * 10 / 100;
        let dialog_bottom = height * 82 / 100;
        anyhow::ensure!(
            dialog_right > dialog_left && dialog_bottom > dialog_top,
            "reply composer crop is invalid"
        );
        let dialog_crop = image::imageops::crop_imm(
            &image,
            dialog_left,
            dialog_top,
            dialog_right - dialog_left,
            dialog_bottom - dialog_top,
        )
        .to_image();
        let dialog_result = self.recognize_image_detailed(&dialog_crop)?;
        let mut reply_input = None;
        let dialog_text_blocks = dialog_result
            .text_blocks
            .iter()
            .filter_map(|block| {
                let left = block.box_points.iter().map(|point| point.x).min()?;
                let top = block.box_points.iter().map(|point| point.y).min()?;
                let right = block.box_points.iter().map(|point| point.x).max()?;
                let bottom = block.box_points.iter().map(|point| point.y).max()?;
                let global_left = dialog_left + left;
                let global_top = dialog_top + top;
                let global_right = dialog_left + right;
                let global_bottom = dialog_top + bottom;
                let normalized = normalize_ocr_text(&block.text);
                if normalized.contains(&normalize_ocr_text("发布你的回复")) {
                    reply_input = Some(serde_json::json!({
                        "x": (global_left + global_right) / 2,
                        "y": (global_top + global_bottom) / 2,
                    }));
                }
                Some(serde_json::json!({
                    "text": block.text,
                    "box": {
                        "left": global_left,
                        "top": global_top,
                        "right": global_right,
                        "bottom": global_bottom,
                    },
                    "confidence": block.text_score,
                }))
            })
            .collect::<Vec<_>>();

        let dialog_text = dialog_result
            .text_blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let reply_target_account = dialog_result.text_blocks.iter().find_map(|block| {
            let text = block.text.trim();

            let marker = text
                .find("回复给")
                .or_else(|| text.find("回复@"))
                .or_else(|| text.find("Replying to"))?;

            let suffix = &text[marker..];
            let at = suffix.find('@')?;

            let account: String = suffix[at + 1..]
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || c.is_whitespace())
                .collect::<String>()
                .replace(' ', "_");

            (!account.is_empty() && account.len() <= 15).then_some(account)
        });

        Ok(serde_json::json!({
            "image": path.display().to_string(),
            "image_size": { "width": width, "height": height },
            "current_url": current_url,
            "address_text": address_result.text_blocks.iter().map(|block| block.text.as_str()).collect::<Vec<_>>().join(" "),
            "dialog_crop": {
                "left": dialog_left,
                "top": dialog_top,
                "right": dialog_right,
                "bottom": dialog_bottom,
            },
            "dialog_text": dialog_text,
            "dialog_text_block_count": dialog_text_blocks.len(),
            "dialog_text_blocks": dialog_text_blocks,
            "reply_input": reply_input,
            "reply_target_account": reply_target_account,
        }))
    }

    /// Recognizes text and returns the detailed result, including text boxes and scores.
    pub fn recognize_path_detailed(&mut self, path: impl AsRef<Path>) -> Result<OcrResult> {
        let path = path.as_ref();
        let image = ImageReader::open(path)
            .with_context(|| format!("failed to open image {}", path.display()))?
            .decode()
            .with_context(|| format!("failed to decode image {}", path.display()))?
            .to_rgb8();

        self.recognize_image_detailed(&image)
    }

    /// Recognizes text in an image and returns the detailed result.
    pub fn recognize_image_detailed(&mut self, image: &image::RgbImage) -> Result<OcrResult> {
        self.engine
            .detect_with_options(
                image,
                10,
                1280,
                0.6,
                0.3,
                1.6,
                false,
                false,
                OcrOptions::default(),
            )
            .map_err(Into::into)
    }

    /// Recognizes text in an image file and joins the detected lines with newlines.
    pub fn recognize_path(&mut self, path: impl AsRef<Path>) -> Result<String> {
        let result = self.recognize_path_detailed(path)?;
        Ok(join_text_blocks(&result))
    }

    /// Recognizes text in an encoded image buffer (PNG, JPEG, BMP, or WebP).
    pub fn recognize_bytes(&mut self, bytes: &[u8]) -> Result<String> {
        let image = image::load_from_memory(bytes).context("failed to decode image buffer")?;
        self.recognize_image(&image)
    }

    /// Recognizes text in a decoded image.
    pub fn recognize_image(&mut self, image: &DynamicImage) -> Result<String> {
        let image = image.to_rgb8();
        let result = self.recognize_image_detailed(&image)?;
        Ok(join_text_blocks(&result))
    }
}

#[cfg(target_os = "windows")]
fn directml_session_builder(
    builder: ort::session::builder::SessionBuilder,
) -> std::result::Result<ort::session::builder::SessionBuilder, ort::Error> {
    let builder = builder
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Disable)?
        .with_memory_pattern(false)?;
    Ok(builder.with_execution_providers([ort::ep::DirectML::default().build()])?)
}

fn find_x_url_start(normalized: &str) -> Option<usize> {
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
    candidates.into_iter().min()
}

fn extract_x_url(text: &str) -> Option<String> {
    let normalized = text.to_ascii_lowercase();
    let start = find_x_url_start(&normalized)?;
    Some(
        text[start..]
            .split_whitespace()
            .next()?
            .trim_matches(|ch: char| matches!(ch, '\"' | '\'' | '(' | ')' | ',' | ';'))
            .trim_end_matches('.')
            .to_string(),
    )
}

fn normalize_ocr_text(text: &str) -> String {
    text.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

fn action_coordinates(
    result: &OcrResult,
    image_width: u32,
    image_height: u32,
) -> serde_json::Value {
    let empty_coordinates = || {
        serde_json::json!({
            "reply": null,
            "repost": null,
            "like": null,
        })
    };
    let mut counts = result
        .text_blocks
        .iter()
        .filter_map(|block| {
            let text = block.text.trim();
            let count = extract_count_token(text)?;
            let left = block.box_points.iter().map(|point| point.x).min()?;
            let top = block.box_points.iter().map(|point| point.y).min()?;
            let right = block.box_points.iter().map(|point| point.x).max()?;
            let bottom = block.box_points.iter().map(|point| point.y).max()?;
            let x = (left + right) / 2;
            let y = (top + bottom) / 2;
            let relative_x = x as f32 / image_width.max(1) as f32;
            (relative_x >= 0.30 && relative_x <= 0.68 && y > image_height / 10)
                .then_some((x, y, left, relative_x, count))
        })
        .collect::<Vec<_>>();
    counts.sort_by_key(|candidate| (candidate.1, candidate.0));
    let reply_fallback = counts
        .iter()
        .filter(|candidate| candidate.3 >= 0.30 && candidate.3 < 0.40)
        .max_by_key(|row| row.1)
        .cloned();
    let view_fallback = counts
        .iter()
        .filter(|candidate| candidate.3 >= 0.48 && candidate.3 <= 0.68)
        .max_by_key(|candidate| candidate.1)
        .cloned();
    let mut rows: Vec<Vec<(u32, u32, u32, f32, String)>> = Vec::new();
    for count in counts {
        if let Some(row) = rows.last_mut()
            && count.1.saturating_sub(row[0].1) <= 12
        {
            row.push(count);
        } else {
            rows.push(vec![count]);
        }
    }
    let mut row = rows
        .into_iter()
        .filter(|row| {
            row.len() >= 3
                && row.iter().any(|item| item.3 < 0.40)
                && row.iter().any(|item| (0.45..0.53).contains(&item.3))
                && row.iter().any(|item| (0.52..0.63).contains(&item.3))
        })
        .max_by_key(|row| row[0].1);
    let Some(mut row) = row.take() else {
        if let Some(reply) = reply_fallback {
            let icon_offset = (image_width as f32 * 0.009).round() as u32;
            let reply_x = reply.2.saturating_add(icon_offset);
            let slot = (image_width as f32 * 0.061).round() as u32;
            let y = reply.1 as i32;
            return serde_json::json!({
                "reply": { "x": reply_x, "y": y },
                "repost": { "x": reply_x.saturating_add(slot), "y": y },
                "like": { "x": reply_x.saturating_add(slot.saturating_mul(2)), "y": y },
                "coordinate_source": "recognized_reply_count_position",
            });
        }
        let Some(view) = view_fallback else {
            return empty_coordinates();
        };
        let icon_offset = (image_width as f32 * 0.009).round() as u32;
        let view_x = view.2.saturating_add(icon_offset);
        let slot = (image_width as f32 * 0.061).round() as u32;
        let reply_x = view_x.saturating_sub(slot.saturating_mul(3));
        let repost_x = reply_x.saturating_add(slot);
        let like_x = reply_x.saturating_add(slot.saturating_mul(2));
        let y = view.1 as i32;
        return serde_json::json!({
            "reply": { "x": reply_x, "y": y },
            "repost": { "x": repost_x, "y": y },
            "like": { "x": like_x, "y": y },
            "views": { "x": view_x, "y": y },
            "coordinate_source": "estimated_from_views_position",
        });
    };
    row.sort_by_key(|item| item.0);
    let comment = &row[0];
    let like = &row[row.len() - 2];
    let repost = (row.len() >= 4).then(|| &row[1]);
    let y = ((comment.1 + like.1) / 2) as i32;
    let icon_offset = (image_width as f32 * 0.009).round() as u32;
    let reply_x = comment.2.saturating_add(icon_offset) as i32;
    let like_x = like.2.saturating_add(icon_offset) as i32;
    let repost_x = repost
        .map(|item| item.2.saturating_add(icon_offset) as i32)
        .unwrap_or((reply_x + like_x) / 2);
    if !(reply_x < repost_x && repost_x < like_x) {
        return empty_coordinates();
    }
    serde_json::json!({
        "reply": { "x": reply_x, "y": y },
        "repost": { "x": repost_x, "y": y },
        "like": { "x": like_x, "y": y },
    })
}

fn extract_count_token(text: &str) -> Option<String> {
    let token = text.split_whitespace().last()?;
    (!token.is_empty()
        && token.chars().any(|ch| ch.is_ascii_digit())
        && token.chars().all(|ch| ch.is_ascii_digit() || ch == ','))
    .then(|| token.to_string())
}

fn join_text_blocks(result: &OcrResult) -> String {
    result
        .text_blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

use std::path::Path;

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

#[cfg(windows)]
pub(super) fn recognize_sent_toast(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_sent_confirmation(&doc.text))
}

#[cfg(windows)]
pub(super) fn recognize_reload_confirmation(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_reload_confirmation(&doc.text))
}

#[cfg(windows)]
pub(super) fn recognize_rate_limit(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    Ok(contains_rate_limit(&doc.text))
}

#[cfg(windows)]
pub(super) fn recognize_retryable_send_error(path: &Path) -> anyhow::Result<bool> {
    let doc = recognize_document(path)?;
    let expected = normalize_text("出错了别担心让我们再试一次");
    let normalized = normalize_text(&doc.text);
    Ok(normalized.contains(&expected)
        || doc
            .lines
            .iter()
            .any(|line| text_match_ratio(&normalize_text(&line.text), &expected) >= 0.86))
}

#[cfg(windows)]
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
                let normalized = normalize_text(&line.text);
                ((x - target_x as f32).abs() <= 120.0
                    && y >= target_y as f32 - 90.0
                    && y <= target_y as f32 + 90.0)
                    .then_some((line.text.as_str(), normalized, x, y))
            })
            .collect::<Vec<_>>();
        let verified = nearby
            .iter()
            .any(|(text, _, x, y)| reply_tooltip_matches_target(text, *x, *y, target_x, target_y));
        let nearby_text = nearby
            .iter()
            .map(|(text, _, x, y)| format!("{text}@({x:.0},{y:.0})"))
            .collect::<Vec<_>>()
            .join(" | ");
        super::automation::log_event(&format!(
            "悬停回复图标 OCR 校验（{mode}）：目标=({target_x},{target_y})，识别到‘回复’提示={verified}；附近文字：{nearby_text}；裁剪图={}。",
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
    let normalized = normalize_text(text);
    normalized == normalize_text("回复")
        && (center_x - target_x as f32).abs() <= 120.0
        && center_y >= target_y as f32 - 90.0
        && center_y <= target_y as f32 + 90.0
}

#[cfg(windows)]
pub(super) fn locate_reply_input(
    path: &Path,
    screenshot_width: u32,
    screenshot_height: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    let expected = normalize_text("发布你的回复");
    // 先跑当前方案，只有未匹配时才尝试其他预处理，避免每次识别都增加耗时。
    let modes = [
        (4, 3, Some(200), "原始阈值 200"),
        (2, 1, None, "2 倍灰度"),
        (2, 1, Some(230), "2 倍阈值 230"),
        (2, 1, Some(160), "2 倍阈值 160"),
    ];

    for (attempt, (scale_num, scale_den, threshold, mode_name)) in modes.iter().enumerate() {
        let doc = recognize_document_with_options(path, *scale_num, *scale_den, *threshold)?;
        let all_lines = doc
            .lines
            .iter()
            .map(|line| {
                format!(
                    "{}@({:.0},{:.0})",
                    line.text,
                    (line.left + line.right) * 0.5 / doc.scale_x,
                    (line.top + line.bottom) * 0.5 / doc.scale_y
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        super::automation::log_event(&format!(
            "阶段 2/3：全屏 OCR（{mode_name}）共 {} 行。全文：\n{}\n全部识别行及坐标：{}",
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
                    || center_x < screenshot_width as f32 * 0.25
                    || center_x > screenshot_width as f32 * 0.75
                    || center_y < screenshot_height as f32 * 0.20
                    || center_y > screenshot_height as f32 * 0.75
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
                    "阶段 2/3：通过备用 OCR 模式‘{mode_name}’识别到回复输入占位文字；识别行‘{}’，匹配度 {:.0}%。",
                    line.text,
                    score * 100.0
                ));
            }
            return Ok(Some((center_x.round() as i32, center_y.round() as i32)));
        }
    }

    super::automation::log_event("阶段 2/3：所有整屏 OCR 预处理方案均未匹配回复输入占位文字。");
    Ok(None)
}

#[cfg(windows)]
pub(super) fn locate_reply_submit_button(
    path: &Path,
    reply_text: &str,
    screenshot_width: u32,
    screenshot_height: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    let doc = recognize_document(path)?;
    let reply_text = normalize_text(reply_text);
    let matching_lines = doc
        .lines
        .iter()
        .filter(|line| {
            let center_x = (line.left + line.right) * 0.5 / doc.scale_x;
            let center_y = (line.top + line.bottom) * 0.5 / doc.scale_y;
            let recognized = normalize_text(&line.text);
            text_match_ratio(&recognized, &reply_text) >= 0.80
                && center_x >= screenshot_width as f32 * 0.28
                && center_x <= screenshot_width as f32 * 0.72
                && center_y >= screenshot_height as f32 * 0.25
                && center_y <= screenshot_height as f32 * 0.65
        })
        .collect::<Vec<_>>();
    if matching_lines.is_empty() {
        let candidates = doc
            .lines
            .iter()
            .filter(|line| {
                let x = (line.left + line.right) * 0.5 / doc.scale_x;
                let y = (line.top + line.bottom) * 0.5 / doc.scale_y;
                x >= screenshot_width as f32 * 0.25
                    && x <= screenshot_width as f32 * 0.75
                    && y >= screenshot_height as f32 * 0.25
                    && y <= screenshot_height as f32 * 0.65
            })
            .map(|line| {
                format!(
                    "{}({:.0},{:.0})",
                    line.text,
                    (line.left + line.right) * 0.5 / doc.scale_x,
                    (line.top + line.bottom) * 0.5 / doc.scale_y
                )
            })
            .take(30)
            .collect::<Vec<_>>()
            .join(" | ");
        super::automation::log_event(&format!(
            "阶段 2/3：提交前 OCR 未匹配回复文本‘{reply_text}’。输入区域识别行：{candidates}"
        ));
        return Ok(None);
    }

    match find_dark_submit_button(path, screenshot_width, screenshot_height)? {
        Some(point) => Ok(Some(point)),
        None => {
            super::automation::log_event(
                "阶段 2/3：OCR 已匹配回复文本，但本地图像检测未找到符合尺寸的深色提交按钮。",
            );
            Ok(None)
        }
    }
}

#[cfg(windows)]
fn recognize_document(path: &Path) -> anyhow::Result<OcrDocument> {
    recognize_document_with_options(path, 4, 3, Some(200))
}

#[cfg(windows)]
fn recognize_document_with_options(
    path: &Path,
    scale_num: u32,
    scale_den: u32,
    threshold: Option<u8>,
) -> anyhow::Result<OcrDocument> {
    use std::io::Cursor;
    use windows::{
        Graphics::Imaging::BitmapDecoder,
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
    };

    let source = image::open(path)?;
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
    let mut encoded = Vec::new();
    image::DynamicImage::ImageLuma8(enlarged.clone())
        .write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Png)?;

    with_cached_ocr_engine(|engine| {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(&encoded)?;
        writer.StoreAsync()?.get()?;
        writer.FlushAsync()?.get()?;
        writer.DetachStream()?;
        stream.Seek(0)?;

        let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
        let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
        let result = engine.RecognizeAsync(&bitmap)?.get()?;
        document_from_result(
            &result,
            enlarged.width() as f32 / source.width() as f32,
            enlarged.height() as f32 / source.height() as f32,
        )
    })
}

#[cfg(windows)]
fn document_from_result(
    result: &windows::Media::Ocr::OcrResult,
    scale_x: f32,
    scale_y: f32,
) -> anyhow::Result<OcrDocument> {
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let mut text = String::new();
        let mut left = f32::INFINITY;
        let mut top = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        for word in line.Words()? {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&word.Text()?.to_string());
            let rect = word.BoundingRect()?;
            left = left.min(rect.X);
            top = top.min(rect.Y);
            right = right.max(rect.X + rect.Width);
            bottom = bottom.max(rect.Y + rect.Height);
        }
        if !text.is_empty() {
            lines.push(OcrLine {
                text,
                left,
                top,
                right,
                bottom,
            });
        }
    }
    let text = result.Text()?.to_string();
    let text = if text.trim().is_empty() {
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text
    };
    Ok(OcrDocument {
        text,
        lines,
        scale_x,
        scale_y,
    })
}

#[cfg(windows)]
fn with_cached_ocr_engine<T>(
    operation: impl FnOnce(&windows::Media::Ocr::OcrEngine) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use std::cell::RefCell;
    use windows::{
        Media::Ocr::OcrEngine,
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
    };

    thread_local! {
        static OCR_ENGINE: RefCell<Option<CachedOcrEngine>> = const { RefCell::new(None) };
    }

    OCR_ENGINE.with(|cached| {
        let mut cached = cached.borrow_mut();
        if cached.is_none() {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
            match OcrEngine::TryCreateFromUserProfileLanguages() {
                Ok(engine) => *cached = Some(CachedOcrEngine(Some(engine))),
                Err(error) => {
                    unsafe { windows::Win32::System::WinRT::RoUninitialize() };
                    return Err(error.into());
                }
            }
        }
        operation(cached.as_ref().unwrap().0.as_ref().unwrap())
    })
}

#[cfg(windows)]
struct CachedOcrEngine(Option<windows::Media::Ocr::OcrEngine>);

#[cfg(windows)]
impl Drop for CachedOcrEngine {
    fn drop(&mut self) {
        self.0.take();
        unsafe { windows::Win32::System::WinRT::RoUninitialize() };
    }
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
    path: &Path,
    screenshot_width: u32,
    screenshot_height: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    let image = image::open(path)?.to_rgb8();
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
                let center_x = (left + right) as i32 / 2;
                let center_y = (top + bottom) as i32 / 2;
                candidates.push((center_x, center_y));
            }
        }
    }

    candidates.sort_by_key(|(x, y)| {
        let dx = (*x - (screenshot_width as i32 * 63 / 100)).abs();
        let dy = (*y - (screenshot_height as i32 * 58 / 100)).abs();
        dx + dy
    });
    Ok(candidates.into_iter().next())
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

#[cfg(not(windows))]
pub(super) fn recognize_sent_toast(_: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}

#[cfg(not(windows))]
pub(super) fn recognize_reload_confirmation(_: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}

#[cfg(not(windows))]
pub(super) fn recognize_rate_limit(_: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}

#[cfg(not(windows))]
pub(super) fn recognize_retryable_send_error(_: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}

#[cfg(not(windows))]
pub(super) fn locate_reply_input(_: &Path, _: u32, _: u32) -> anyhow::Result<Option<(i32, i32)>> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}

#[cfg(not(windows))]
pub(super) fn locate_reply_submit_button(
    _: &Path,
    _: &str,
    _: u32,
    _: u32,
) -> anyhow::Result<Option<(i32, i32)>> {
    anyhow::bail!("Windows OCR 仅支持 Windows")
}

#[cfg(test)]
mod tests {
    use super::{
        contains_rate_limit, contains_reload_confirmation, contains_sent_confirmation,
        reply_tooltip_matches_target, text_match_ratio,
    };

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
    fn accepts_common_chinese_ocr_confusions_in_sent_toast() {
        assert!(contains_sent_confirmation("你的帖孑已发送。查看"));
        assert!(contains_sent_confirmation("你的回复已发送。查看"));
        assert!(!contains_sent_confirmation("草稿内容仍在编辑框中"));
        assert!(!contains_sent_confirmation("帖子发送失败"));
    }
}

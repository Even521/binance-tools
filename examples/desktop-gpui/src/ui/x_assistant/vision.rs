use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use serde::{Deserialize, Deserializer};

use super::automation::{self, native};

#[path = "ocr.rs"]
mod ocr;

pub const DEFAULT_OLLAMA_URL: &str = "http://192.168.31.60:11434/v1";
pub const DEFAULT_VISION_MODEL: &str = "qwen3.5:4b";

const SYSTEM_PROMPT: &str = "你是屏幕视觉操作助手。禁止长篇思考，最快速度返回 JSON 对象。所有坐标和边界值必须使用 0 到 1000 的归一化整数：x=0 表示图片最左，x=1000 表示最右；y=0 表示最上，y=1000 表示最下；x、y 分别按整张图片宽度和高度独立归一化，不要输出像素坐标。无法确认时把 ok 设为 false。";

#[derive(Clone)]
pub struct VisionReplyOptions {
    pub api_url: String,
    pub model: String,
    pub auto_send: bool,
    pub hover_only: bool,
    pub reply_rules: String,
}

pub struct VisionReplyResult {
    pub screenshots: Vec<PathBuf>,
    pub post_text: String,
    pub reply_text: String,
    pub completed: bool,
    pub summary: String,
}

#[derive(Debug, Deserialize)]
struct VisionDecision {
    ok: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    post_text: String,
    #[serde(default)]
    reply_text: String,
    #[serde(default)]
    point: Option<Point>,
    #[serde(default)]
    reply_point: Option<Point>,
    #[serde(default)]
    repost_point: Option<Point>,
    #[serde(default)]
    like_point: Option<Point>,

    #[serde(default)]
    refresh_posts: bool,
    #[serde(default)]
    composer_open: bool,
    #[serde(default)]
    text_matches: bool,
    #[serde(default)]
    reply_field_matches: bool,
    #[serde(default)]
    poll_mode: bool,
    #[serde(default)]
    completed: bool,
}

#[derive(Debug)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PointRepr {
    Pair([i32; 2]),
    Object { x: i32, y: i32 },
}

impl<'de> Deserialize<'de> for Point {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match PointRepr::deserialize(deserializer)? {
            PointRepr::Pair([x, y]) | PointRepr::Object { x, y } => Self { x, y },
        })
    }
}

struct Shot {
    png: PathBuf,
    width: u32,
    height: u32,
    stage: String,
}

pub fn run_visual_reply(
    options: VisionReplyOptions,
    cancel: &AtomicBool,
) -> anyhow::Result<VisionReplyResult> {
    automation::log_event("========== X 辅助回复流程开始 ==========");
    wait_cancel(cancel, Duration::from_secs(5))?;
    native::activate_browser()?;
    wait_cancel(cancel, Duration::from_millis(800))?;

    let dir = PathBuf::from("data")
        .join("x_assistant")
        .join("screenshots");
    fs::create_dir_all(&dir)?;
    let id = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let mut screenshots = Vec::new();

    let model = resolve_model(&options)?;
    let options = VisionReplyOptions { model, ..options };
    automation::log_event(&format!(
        "视觉模型：{}，接口：{}",
        options.model, options.api_url
    ));

    automation::log_event("========== 阶段 1/3：定位并点击帖子回复图标 ==========");
    let mut feed = capture(&dir, &id, "01-feed", &mut screenshots)?;
    if options.hover_only {
        automation::log_event("阶段 1/3：仅移动鼠标，不点击。");
        let prompt = action_bar_prompt(feed.width, feed.height);
        let (post, vision_ms) = ask(&options, &feed, &prompt, cancel)?;
        require_ok(&post, "未能识别帖子和操作栏")?;
        let reply_button = validate_action_targets(&post, feed.width, feed.height)?;
        let post_text = post.post_text.trim().to_string();
        check_cancel(cancel)?;
        native::move_cursor_to_reply(reply_button.x, reply_button.y)?;
        return Ok(VisionReplyResult {
            screenshots,
            post_text,
            reply_text: String::new(),
            completed: false,
            summary: format!(
                "鼠标已停在回复按钮 ({}, {})，未点击。识图耗时 {} ms。",
                reply_button.x,
                reply_button.y,
                vision_ms.as_millis()
            ),
        });
    }
    let prompt = action_bar_prompt(feed.width, feed.height);
    let (post, _vision_ms) = ask(&options, &feed, &prompt, cancel)?;
    automation::log_event(&format!("AI 识别到帖子正文长度：{}", post.post_text.len()));
    require_ok(&post, "未能识别帖子正文")?;

    let mut reply_button = validate_action_targets(&post, feed.width, feed.height)?;
    let mut post_text = post.post_text.trim().to_string();
    if post_text.is_empty() {
        automation::log_event("警告：AI 返回的 post_text 为空，尝试要求其重新识别...");
        anyhow::bail!("AI 识别到的帖子内容为空，请确保浏览器窗口中有清晰的帖子文字。");
    }
    automation::log_event(&format!("成功获取帖子内容，长度：{}", post_text.len()));

    check_cancel(cancel)?;
    let pre_click = capture(&dir, &id, "01-preclick", &mut screenshots)?;
    if !target_region_unchanged(&feed, &pre_click, &reply_button, "帖子回复图标")? {
        automation::log_event(
            "阶段 1/3：识别期间帖子区域变化，检查顶部是否出现‘显示帖子’更新按钮。",
        );
        anyhow::ensure!(
            click_show_new_posts(&options, &pre_click, cancel)?,
            "识别期间页面发生变化，且没有可确认的‘显示帖子’按钮；请重新开始"
        );
        let wait_ms = random_wait_ms();
        automation::log_event(&format!(
            "阶段 1/3：已点击‘显示帖子’，等待 {wait_ms} ms 后重新截图识别。"
        ));
        wait_cancel(cancel, Duration::from_millis(wait_ms))?;
        feed = capture(&dir, &id, "01-refreshed-feed", &mut screenshots)?;
        let refreshed_prompt = action_bar_prompt(feed.width, feed.height);
        let (refreshed_post, _) = ask(&options, &feed, &refreshed_prompt, cancel)?;
        require_ok(&refreshed_post, "刷新后未能识别目标帖子")?;
        reply_button = validate_action_targets(&refreshed_post, feed.width, feed.height)?;
        post_text = refreshed_post.post_text.trim().to_string();
        let refreshed_pre_click = capture(&dir, &id, "01-refreshed-preclick", &mut screenshots)?;
        anyhow::ensure!(
            target_region_unchanged(
                &feed,
                &refreshed_pre_click,
                &reply_button,
                "刷新后的回复图标"
            )?,
            "刷新后帖子区域仍在变化，已停止以避免使用过期坐标"
        );
    }
    native::move_cursor_to_reply(reply_button.x, reply_button.y)?;
    thread::sleep(Duration::from_millis(500));
    native::click_at_client_position(reply_button.x, reply_button.y)?;
    automation::log_event("========== 阶段 2/3：确认回复弹窗、生成并提交回复 ==========");
    let wait_ms = 2_000
        + (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            % 3_001) as u64;
    automation::log_event(&format!("点击回复后等待 {wait_ms} ms，再截图确认回复弹窗"));
    wait_cancel(cancel, Duration::from_millis(wait_ms))?;

    let composer = capture(&dir, &id, "02-composer", &mut screenshots)?;
    let (composer_decision, _) = ask(
        &options,
        &composer,
        &format!(
            "这是一项单次识图任务：在同一次响应中同时确认回复弹窗/原帖并生成回复，不要要求后续再调用模型。目标原帖正文应与以下内容相符：{post_text}。必须同时看见匹配的原帖上下文和占位文字“发布你的回复”；若页面处于发帖/投票编辑模式、上下文不匹配或状态不清楚，返回 ok=false、composer_open=false、reply_text 为空。确认后，在本次响应里同时提供：1) point：占位文字“发布你的回复”所在行的安全输入位置，不能在下方空白区、底部图标工具栏、投票图标或灰色“回复”提交按钮；2) reply_text：遵守以下规则、结合截图中的原帖内容生成不超过80字且不换行的回复。reply_text 是必填字段，不得省略。\n---回复规则开始---\n{}\n---回复规则结束---\n严格返回单个 JSON：{{\"ok\":true,\"composer_open\":true,\"point\":[400,420],\"reply_text\":\"回复正文\",\"reason\":\"\"}}。point 数组顺序为 [x,y]。",
            options.reply_rules.trim()
        ),
        cancel,
    )?;
    require_ok(&composer_decision, "回复框未打开")?;
    anyhow::ensure!(composer_decision.composer_open, "AI 未确认回复框已打开");
    let input_point = require_point(
        &composer_decision,
        composer.width,
        composer.height,
        "回复文本编辑区",
    )?;
    validate_composer_input_point(&input_point, composer.width, composer.height)?;

    let reply_text = sanitize_reply(&composer_decision.reply_text)?;

    let pre_input = capture(&dir, &id, "02-preinput", &mut screenshots)?;
    anyhow::ensure!(
        target_region_unchanged(&composer, &pre_input, &input_point, "回复输入区")?,
        "回复弹窗内容发生变化，输入坐标已过期；已停止"
    );
    native::move_cursor_to_reply(input_point.x, input_point.y)?;
    native::click_at_client_position(input_point.x, input_point.y)?;
    thread::sleep(Duration::from_millis(250));
    native::type_text(&reply_text)?;
    thread::sleep(Duration::from_millis(400));

    automation::log_event("阶段 2/3：回复已输入，截图核对后识别带‘回复’文字的提交按钮。");
    let typed = capture(&dir, &id, "03-typed", &mut screenshots)?;
    let (typed_decision, _) = ask(
        &options,
        &typed,
        &format!(
            "检查输入内容：{reply_text}。只有当这段文字确实显示在原帖下方的回复文本编辑区时，才设置 text_matches=true、reply_field_matches=true、poll_mode=false。若文字出现在投票选项、发帖正文或其他控件中，设置 ok=false、reply_field_matches=false；若是投票编辑界面，poll_mode=true，绝不返回可点击坐标。确认文本正确后，直接根据这张完整截图定位当前弹窗底部黑色“回复”文字按钮的可见边界，返回按钮矩形的几何中心；不要猜测、沿用示例坐标或依据其他运行的按钮位置，不要把回复气泡图标、工具栏或其他按钮当作目标。返回合法 JSON，字段为 ok、text_matches、reply_field_matches、poll_mode、point、reason；point 必须是本张截图中按钮中心的 0..=1000 归一化整数坐标 [x,y]。不要点击任何按钮。",
        ),
        cancel,
    )?;
    require_ok(&typed_decision, "输入内容未通过回复框核对")?;
    anyhow::ensure!(
        typed_decision.text_matches && typed_decision.reply_field_matches,
        "AI 未确认文本位于回复编辑区，已停止，未点击提交"
    );
    anyhow::ensure!(
        !typed_decision.poll_mode,
        "检测到投票编辑模式，已停止，未点击提交"
    );

    if !options.auto_send {
        return Ok(VisionReplyResult {
            screenshots,
            post_text,
            reply_text,
            completed: false,
            summary: "回复已输入并通过截图核对。未点击发送，请人工确认后再发送。".to_string(),
        });
    }

    check_cancel(cancel)?;
    let send_button = require_point(
        &typed_decision,
        typed.width,
        typed.height,
        "弹窗中的“回复”提交按钮",
    )?;
    let pre_submit = capture(&dir, &id, "03-presubmit", &mut screenshots)?;
    anyhow::ensure!(
        target_region_unchanged(&typed, &pre_submit, &send_button, "弹窗中的‘回复’按钮")?,
        "回复弹窗内容发生变化，提交按钮坐标已过期；已停止"
    );
    native::move_cursor_to_reply(send_button.x, send_button.y)?;
    automation::log_event("========== 阶段 3/3：点击‘回复’并判断回复是否完成 ==========");
    native::click_at_client_position(send_button.x, send_button.y)?;
    automation::log_event("阶段 3/3：已点击‘回复’，等待 2 秒后采集结果截图。");
    thread::sleep(Duration::from_secs(2));

    let done = capture(&dir, &id, "04-result", &mut screenshots)?;
    check_cancel(cancel)?;
    let ocr_started = std::time::Instant::now();
    match ocr::recognize_sent_toast(&done.png) {
        Ok(confirmed) => {
            automation::log_event(&format!(
                "阶段 3/3：Windows OCR 耗时 {} ms，{}。",
                ocr_started.elapsed().as_millis(),
                if confirmed {
                    "检测到发送成功提示"
                } else {
                    "未检测到发送成功提示，将使用 AI 复核"
                }
            ));
            if confirmed {
                return Ok(VisionReplyResult {
                    screenshots,
                    post_text,
                    reply_text,
                    completed: true,
                    summary: "Windows OCR 检测到发送成功提示。".to_string(),
                });
            }
        }
        Err(error) => automation::log_event(&format!(
            "阶段 3/3：Windows OCR 耗时 {} ms，无法使用（{}）；将使用 AI 复核",
            ocr_started.elapsed().as_millis(),
            error
        )),
    }
    let (done_decision, _) = ask(
        &options,
        &done,
        "判断刚才的回复是否已经成功发送。请优先检查页面底部或其他位置是否出现 X 的明确成功提示，例如‘你的帖子已发送’或‘你的回复已发送’；只要出现该提示，就必须返回 ok=true、completed=true，即使回复弹窗或草稿框仍然可见（页面可能尚未关闭弹窗）。如果没有成功提示，再检查回复框是否关闭或刚发送的回复是否已出现在原帖下方。只有看不到任何成功证据、且输入内容仍留在未发送框中时，才返回 ok=false、completed=false。严格返回 JSON：{\"ok\":true,\"completed\":true,\"reason\":\"\"}。",
        cancel,
    )?;
    if !done_decision.ok || !done_decision.completed {
        anyhow::bail!(
            "无法确认回复已发送：{}",
            fallback_reason(&done_decision, "完成状态不明确")
        );
    }

    Ok(VisionReplyResult {
        screenshots,
        post_text,
        reply_text,
        completed: true,
        summary: "截图确认回复已发送。".to_string(),
    })
}

fn capture(
    dir: &Path,
    id: &impl std::fmt::Display,
    stage: &str,
    screenshots: &mut Vec<PathBuf>,
) -> anyhow::Result<Shot> {
    let bmp = dir.join(format!("{id}-{stage}.bmp"));
    automation::log_event(&format!("开始采集阶段截图：{stage}"));
    let capture_started = std::time::Instant::now();
    native::capture_window(&bmp)?;
    let image = image::open(&bmp)?;
    let png = bmp.with_extension("png");
    image.save(&png)?;
    automation::log_event(&format!(
        "阶段截图采集完成：{}，尺寸 {}x{}，耗时 {} ms",
        png.display(),
        image.width(),
        image.height(),
        capture_started.elapsed().as_millis()
    ));
    screenshots.push(png.clone());
    Ok(Shot {
        png,
        width: image.width(),
        height: image.height(),
        stage: stage.to_string(),
    })
}

fn target_region_unchanged(
    before: &Shot,
    after: &Shot,
    target: &Point,
    label: &str,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        before.width == after.width && before.height == after.height,
        "识别前后截图尺寸发生变化，{label}坐标已失效；请重新开始"
    );
    let before_image = image::open(&before.png)?.to_rgb8();
    let after_image = image::open(&after.png)?.to_rgb8();
    let radius_x = 120i32;
    let radius_y = 72i32;
    let left = (target.x - radius_x).max(0) as u32;
    let right = (target.x + radius_x).min(before.width as i32) as u32;
    let top = (target.y - radius_y).max(0) as u32;
    let bottom = (target.y + radius_y).min(before.height as i32) as u32;
    anyhow::ensure!(right > left && bottom > top, "无法校验{label}附近截图区域");

    let mut sampled = 0usize;
    let mut changed = 0usize;
    for y in (top..bottom).step_by(3) {
        for x in (left..right).step_by(3) {
            let a = before_image.get_pixel(x, y).0;
            let b = after_image.get_pixel(x, y).0;
            let delta = a
                .iter()
                .zip(b.iter())
                .map(|(a, b)| a.abs_diff(*b) as u16)
                .sum::<u16>()
                / 3;
            sampled += 1;
            if delta >= 28 {
                changed += 1;
            }
        }
    }
    let changed_ratio = changed as f32 / sampled.max(1) as f32;
    automation::log_event(&format!(
        "目标区域复核：{label}，坐标 ({}, {})，画面变化 {:.1}%",
        target.x,
        target.y,
        changed_ratio * 100.0
    ));
    Ok(changed_ratio <= 0.025)
}

fn click_show_new_posts(
    options: &VisionReplyOptions,
    shot: &Shot,
    cancel: &AtomicBool,
) -> anyhow::Result<bool> {
    let prompt = format!(
        "检查截图顶部时间线区域。如果明确看见 X 的新帖子提示按钮，按钮文字类似‘显示 35 帖子’或‘显示帖子’，返回 ok=true、refresh_posts=true 和该按钮中心点；不要把‘显示更多’、导航、广告按钮识别成它。找不到这个确切的新帖子提示时返回 ok=true、refresh_posts=false、point=null。坐标为 0..=1000 的归一化 [x,y]。JSON：{{\"ok\":true,\"refresh_posts\":true,\"point\":[500,150],\"reason\":\"\"}}。截图尺寸 {}x{}。",
        shot.width, shot.height
    );
    let (decision, _) = ask(options, shot, &prompt, cancel)?;
    if !decision.ok || !decision.refresh_posts {
        automation::log_event("阶段 1/3：未确认‘显示帖子’更新按钮，不执行刷新点击。");
        return Ok(false);
    }
    let point = require_point(&decision, shot.width, shot.height, "显示帖子按钮")?;
    anyhow::ensure!(
        point.y <= (shot.height as f32 * 0.28) as i32
            && point.x >= (shot.width as f32 * 0.20) as i32
            && point.x <= (shot.width as f32 * 0.80) as i32,
        "AI 返回的‘显示帖子’按钮位置不在顶部时间线区域，已停止"
    );
    automation::log_event(&format!(
        "阶段 1/3：确认‘显示帖子’按钮，坐标 ({}, {})。",
        point.x, point.y
    ));
    native::move_cursor_to_reply(point.x, point.y)?;
    thread::sleep(Duration::from_millis(250));
    native::click_at_client_position(point.x, point.y)?;
    Ok(true)
}

fn random_wait_ms() -> u64 {
    2_000
        + (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            % 3_001) as u64
}

fn ask(
    options: &VisionReplyOptions,
    shot: &Shot,
    prompt: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<(VisionDecision, Duration)> {
    check_cancel(cancel)?;
    let encoded = model_png(shot)?;

    // 调试：保存发送给 AI 的原尺寸图片
    let debug_path = shot.png.parent().unwrap().join(format!(
        "debug-to-ai-{}.png",
        chrono::Local::now().format("%H%M%S")
    ));
    fs::write(&debug_path, &encoded)?;
    automation::log_event(&format!(
        "[{}] 已保存发送给 AI 的截图：{}",
        phase_label(&shot.stage),
        debug_path.display()
    ));

    let started = std::time::Instant::now();
    let content = binance_tools::ai::send_openai_vision_cancellable(
        &options.api_url,
        &options.model,
        SYSTEM_PROMPT,
        prompt,
        &encoded,
        Duration::from_secs(120), // 恢复为 120 秒超时
        cancel,
    );
    let elapsed = started.elapsed();

    match &content {
        Ok(text) => automation::log_event(&format!(
            "[{}] AI 响应成功 ({}ms)，内容前 1024 字：{}",
            phase_label(&shot.stage),
            elapsed.as_millis(),
            truncate(text, 1024)
        )),
        Err(e) => automation::log_event(&format!(
            "[{}] AI 请求失败 ({}ms)：{}",
            phase_label(&shot.stage),
            elapsed.as_millis(),
            e
        )),
    }

    log_vision_time(shot, elapsed, content.is_ok());
    let content = content?;
    let mut decision = match parse_decision(&content) {
        Ok(d) => d,
        Err(e) => {
            automation::log_event(&format!(
                "[{}] JSON 解析失败，原始返回：{}",
                phase_label(&shot.stage),
                content
            ));
            return Err(e);
        }
    };
    for point in [
        decision.point.as_mut(),
        decision.reply_point.as_mut(),
        decision.repost_point.as_mut(),
        decision.like_point.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        point.x = normalized_coord(point.x, shot.width);
        point.y = normalized_coord(point.y, shot.height);
    }

    Ok((decision, elapsed))
}

fn phase_label(stage: &str) -> &'static str {
    match stage {
        "01-feed" | "01-preclick" | "01-refreshed-feed" | "01-refreshed-preclick" => "阶段 1/3",
        "02-composer" | "02-preinput" | "03-typed" => "阶段 2/3",
        "03-presubmit" | "04-result" => "阶段 3/3",
        _ => "未分类阶段",
    }
}

fn log_vision_time(shot: &Shot, elapsed: Duration, ok: bool) {
    let path = PathBuf::from("data")
        .join("x_assistant")
        .join("vision-timing.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!(
        "[{timestamp}] [{}] 识图 {} ms {} {}",
        phase_label(&shot.stage),
        elapsed.as_millis(),
        if ok { "完成" } else { "已停止或失败" },
        shot.png.display()
    );
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
    automation::log_event(&line);
}

fn wait_cancel(cancel: &AtomicBool, total: Duration) -> anyhow::Result<()> {
    let mut left = total;
    while !left.is_zero() {
        check_cancel(cancel)?;
        let slice = left.min(Duration::from_millis(100));
        thread::sleep(slice);
        left = left.saturating_sub(slice);
    }
    check_cancel(cancel)
}

fn model_png(shot: &Shot) -> anyhow::Result<Vec<u8>> {
    let image = image::open(&shot.png)?;
    let mut encoded = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut encoded),
        image::ImageFormat::Png,
    )?;
    Ok(encoded)
}

fn normalized_coord(value: i32, extent: u32) -> i32 {
    ((value as f32) * extent.saturating_sub(1) as f32 / 1000.0).round() as i32
}

fn parse_decision(content: &str) -> anyhow::Result<VisionDecision> {
    let without_thinking = strip_thinking(content);
    let trimmed = without_thinking.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim()
        .strip_suffix("```")
        .unwrap_or(trimmed)
        .trim();
    let start = json.find('{').unwrap_or(0);
    let end = json.rfind('}').map(|index| index + 1).unwrap_or(json.len());
    serde_json::from_str(&json[start..end])
        .map_err(|err| anyhow::anyhow!("AI 返回的不是可执行 JSON：{err}"))
}

fn strip_thinking(content: &str) -> String {
    let mut out = String::new();
    let mut rest = content;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        rest = &rest[start + "<think>".len()..];
        if let Some(end) = rest.find("</think>") {
            rest = &rest[end + "</think>".len()..];
        } else {
            // 如果只有 <think> 没有 </think>，说明被截断了，尝试保留 <think> 之后的内容
            automation::log_event("警告：AI 输出的 <think> 未闭合，可能遭到了截断");
            out.push_str(rest);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn action_bar_prompt(width: u32, height: u32) -> String {
    format!(
        "截图尺寸为 {width}x{height} 像素。坐标必须使用 0..=1000 的归一化整数，x 和 y 按整张截图宽度、高度分别归一化；不要输出像素坐标。只选择最上方完整可见且正文清晰的一条 X 帖子，准确识别该帖正文下方同一操作栏中的回复、转发、点赞图标。严格返回以下 JSON，不要代码围栏、不要额外字段，坐标用 [x,y] 数组，顺序固定为 x 后 y：{{\"ok\":true,\"post_text\":\"帖子正文\",\"reply_point\":[350,500],\"repost_point\":[410,500],\"like_point\":[470,500]}}。看不清或无法确定属于同一帖子时设置 ok=false。"
    )
}

fn validate_action_targets(
    decision: &VisionDecision,
    width: u32,
    height: u32,
) -> anyhow::Result<Point> {
    let reply = require_named_point(decision.reply_point.as_ref(), width, height, "回复按钮")?;
    let repost = require_named_point(decision.repost_point.as_ref(), width, height, "转发按钮")?;
    let like = require_named_point(decision.like_point.as_ref(), width, height, "点赞按钮")?;
    anyhow::ensure!(
        reply.x < repost.x && repost.x < like.x,
        "回复、转发、点赞坐标的水平顺序异常，已停止"
    );
    let max_y_delta = 24;
    anyhow::ensure!(
        (reply.y - repost.y).abs() <= max_y_delta && (reply.y - like.y).abs() <= max_y_delta,
        "回复、转发、点赞坐标不在同一水平操作栏，已停止"
    );
    for (label, point) in [("回复", &reply), ("转发", &repost), ("点赞", &like)] {
        automation::log_event(&format!(
            "AI 识别到{label}图标坐标：({}, {})",
            point.x, point.y
        ));
    }

    Ok(reply)
}

fn require_ok(decision: &VisionDecision, fallback: &str) -> anyhow::Result<()> {
    anyhow::ensure!(decision.ok, "{}", fallback_reason(decision, fallback));
    Ok(())
}

fn validate_composer_input_point(point: &Point, width: u32, height: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        point.x >= (width as f32 * 0.20) as i32
            && point.x <= (width as f32 * 0.80) as i32
            && point.y >= (height as f32 * 0.20) as i32
            && point.y <= (height as f32 * 0.52) as i32,
        "AI 给出的输入点 ({}, {}) 未落在回复文字区域，可能点到工具栏或提交按钮，已停止",
        point.x,
        point.y
    );
    Ok(())
}

fn require_point(
    decision: &VisionDecision,
    width: u32,
    height: u32,
    label: &str,
) -> anyhow::Result<Point> {
    require_named_point(decision.point.as_ref(), width, height, label)
}

fn require_named_point(
    point: Option<&Point>,
    width: u32,
    height: u32,
    label: &str,
) -> anyhow::Result<Point> {
    let point = point.ok_or_else(|| anyhow::anyhow!("AI 没有返回{label}坐标"))?;
    anyhow::ensure!(
        point.x >= 0 && point.y >= 0 && (point.x as u32) < width && (point.y as u32) < height,
        "{label}坐标 ({}, {}) 超出截图范围 {}x{}",
        point.x,
        point.y,
        width,
        height
    );
    Ok(Point {
        x: point.x,
        y: point.y,
    })
}

fn sanitize_reply(text: &str) -> anyhow::Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    anyhow::ensure!(!text.is_empty(), "AI 生成的回复为空");
    anyhow::ensure!(
        text.chars().count() <= 280,
        "AI 生成的回复超过 280 字，已停止"
    );
    Ok(text)
}

fn fallback_reason(decision: &VisionDecision, fallback: &str) -> String {
    if decision.reason.trim().is_empty() {
        fallback.to_string()
    } else {
        decision.reason.trim().to_string()
    }
}

fn resolve_model(options: &VisionReplyOptions) -> anyhow::Result<String> {
    let preferred = options.model.trim();
    anyhow::ensure!(!preferred.is_empty(), "没有配置视觉模型");
    Ok(preferred.to_string())
}

fn check_cancel(cancel: &AtomicBool) -> anyhow::Result<()> {
    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "视觉回复流程已停止");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Point, normalized_coord, parse_decision, sanitize_reply, validate_composer_input_point,
    };

    #[test]
    fn converts_normalized_coordinates_to_screenshot_pixels() {
        assert_eq!(normalized_coord(350, 1920), 672);
        assert_eq!(normalized_coord(910, 1032), 938);
    }

    #[test]
    fn rejects_composer_points_on_the_bottom_toolbar() {
        let placeholder = Point { x: 768, y: 433 };
        let toolbar = Point { x: 768, y: 567 };
        assert!(validate_composer_input_point(&placeholder, 1920, 1032).is_ok());
        assert!(validate_composer_input_point(&toolbar, 1920, 1032).is_err());
    }

    #[test]
    fn parses_fenced_json_and_rejects_empty_reply() {
        let decision =
            parse_decision("```json\n{\"ok\":true,\"point\":{\"x\":12,\"y\":34}}\n```").unwrap();
        assert!(decision.ok);
        assert_eq!(decision.point.unwrap().x, 12);
        let array_point = parse_decision(
            r#"{"ok":true,"reply_point":[350,500],"repost_point":[410,500],"like_point":[470,500]}"#,
        )
        .unwrap();
        assert_eq!(array_point.reply_point.unwrap().y, 500);
        assert!(sanitize_reply("  你好\n世界  ").unwrap() == "你好 世界");
        assert!(sanitize_reply("   ").is_err());
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

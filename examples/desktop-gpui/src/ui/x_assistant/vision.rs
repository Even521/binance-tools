use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::Duration,
};

use serde::{Deserialize, Deserializer};

use super::automation::{self, native};
use super::ocr;
use super::ocr_status::recognize_send_status;

#[path = "text_ai.rs"]
mod text_ai;

pub const DEFAULT_OLLAMA_URL: &str = "http://192.168.31.60:11434/v1";
pub const DEFAULT_VISION_MODEL: &str = "qwen3.5:4b";
pub const AUTO_SCROLL_ENABLED: bool = false;

const SYSTEM_PROMPT: &str = "你是屏幕视觉操作助手。禁止长篇思考，最快速度返回 JSON 对象。所有坐标和边界值必须使用 0 到 1000 的归一化整数：x=0 表示图片最左，x=1000 表示最右；y=0 表示最上，y=1000 表示最下；有多个回复气泡时候不能在正文的上方必须在正文下方相差高度32像素；x、y 分别按整张图片宽度和高度独立归一化，不要输出像素坐标。无法确认时把 ok 设为 false。";

#[derive(Clone)]
pub struct VisionReplyOptions {
    pub api_url: String,
    pub model: String,
    pub auto_send: bool,
    pub hover_only: bool,
    pub stage2_ocr: bool,
    pub reply_rules: String,
}

#[derive(Debug)]
pub(super) struct RateLimitDetected;

impl std::fmt::Display for RateLimitDetected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("检测到平台限流，已停止自动发送")
    }
}

impl std::error::Error for RateLimitDetected {}

pub struct VisionReplyResult {
    pub screenshots: Vec<PathBuf>,
    pub author_name: String,
    pub account_handle: String,
    pub posted_at: String,
    pub post_text: String,
    pub reply_text: String,
    pub completed: bool,
    pub skipped_duplicate: bool,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize)]
struct VisionDecision {
    ok: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    post_text: String,
    #[serde(default, alias = "username", alias = "name")]
    author_name: String,
    #[serde(default, alias = "handle", alias = "account")]
    account_handle: String,
    #[serde(default, alias = "time", alias = "timestamp")]
    posted_at: String,
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
    post_text_bottom_y: Option<i32>,
    #[serde(default)]
    post_bottom_y: Option<i32>,

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

#[derive(Clone, Debug)]
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

fn convert_normalized_coordinates(
    decision: &mut VisionDecision,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    anyhow::ensure!(width > 0 && height > 0, "截图尺寸无效，无法换算归一化坐标");
    let to_pixel = |value: i32, dimension: u32, axis: &str| -> anyhow::Result<i32> {
        anyhow::ensure!(
            (0..=1000).contains(&value),
            "AI 返回的归一化 {axis} 坐标 {value} 超出 0..1000"
        );
        Ok((value as f64 * (dimension.saturating_sub(1)) as f64 / 1000.0).round() as i32)
    };
    let convert_point = |point: &mut Option<Point>, label: &str| -> anyhow::Result<()> {
        if let Some(point) = point {
            point.x = to_pixel(point.x, width, &format!("{label} x"))?;
            point.y = to_pixel(point.y, height, &format!("{label} y"))?;
        }
        Ok(())
    };

    convert_point(&mut decision.point, "point")?;
    convert_point(&mut decision.reply_point, "reply_point")?;
    convert_point(&mut decision.repost_point, "repost_point")?;
    convert_point(&mut decision.like_point, "like_point")?;
    if let Some(y) = decision.post_text_bottom_y.as_mut() {
        *y = to_pixel(*y, height, "post_text_bottom_y")?;
    }
    if let Some(y) = decision.post_bottom_y.as_mut() {
        *y = to_pixel(*y, height, "post_bottom_y")?;
    }
    Ok(())
}

struct Shot {
    png: PathBuf,
    width: u32,
    height: u32,
    stage: String,
}

pub(super) fn scroll_to_next_post(cancel: &AtomicBool, reason: &str) -> anyhow::Result<()> {
    scroll_down_in_small_steps(cancel, reason)
}

pub(super) fn randomized_recovery(cancel: &AtomicBool, reason: &str) -> anyhow::Result<()> {
    check_cancel(cancel)?;
    if reason.contains("报错") {
        automation::native::activate_browser()?;
        let dir = PathBuf::from("data")
            .join("x_assistant")
            .join("screenshots");
        fs::create_dir_all(&dir)?;
        let id = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
        let mut screenshots = Vec::new();
        let check_shot =
            capture_desktop(&dir, &id, "full-auto-error-state-check", &mut screenshots)?;
        let mut current_url = ocr::recognize_current_url(&check_shot.png)?;
        if current_url
            .as_deref()
            .is_some_and(ocr::is_x_compose_post_address)
        {
            automation::log_event(
                "全自动恢复：发现回复弹窗仍打开；不重新发送，尝试按 Escape 关闭并核验页面。",
            );
            native::press_escape()?;
            wait_cancel(cancel, Duration::from_millis(900))?;
            let after_escape = capture_desktop(
                &dir,
                &id,
                "full-auto-error-state-after-escape",
                &mut screenshots,
            )?;
            if let Some((discard_x, discard_y)) =
                ocr::locate_discard_draft_button(&after_escape.png)?
            {
                automation::log_event("全自动恢复：发现草稿确认框，点击‘放弃’以关闭未发送回复。");
                native::click_at_client_position(discard_x, discard_y)?;
                wait_cancel(cancel, Duration::from_millis(700))?;
                let after_discard = capture_desktop(
                    &dir,
                    &id,
                    "full-auto-error-state-after-discard",
                    &mut screenshots,
                )?;
                current_url = ocr::recognize_current_url(&after_discard.png)?;
            } else if ocr::recognize_save_draft_confirmation(&after_escape.png)? {
                automation::log_event(
                    "全自动恢复：发现草稿确认框（未精准定位放弃按钮），点击备用位置关闭。",
                );
                let fallback_x = after_escape.width as i32 * 50 / 100;
                let fallback_y = after_escape.height as i32 * 60 / 100;
                native::click_at_client_position(fallback_x, fallback_y)?;
                wait_cancel(cancel, Duration::from_millis(700))?;
                let after_discard = capture_desktop(
                    &dir,
                    &id,
                    "full-auto-error-state-after-discard",
                    &mut screenshots,
                )?;
                current_url = ocr::recognize_current_url(&after_discard.png)?;
            } else {
                current_url = ocr::recognize_current_url(&after_escape.png)?;
            }
        }
        if !current_url.as_deref().is_some_and(ocr::is_x_home_address) {
            automation::log_event(&format!(
                "全自动恢复：当前地址未能确认回到 x.com/home（当前：{current_url:?}）；\
                 按 Escape 后重新截图检查，不再盲目点击屏幕中心。"
            ));

            let _ = native::press_escape();
            std::thread::sleep(Duration::from_millis(800));

            let after_escape = capture_desktop(
                &dir,
                &id,
                "full-auto-error-state-safe-check",
                &mut screenshots,
            )?;

            current_url = ocr::recognize_current_url(&after_escape.png)?;

            if current_url.as_deref().is_some_and(ocr::is_x_home_address) {
                automation::log_event("全自动恢复：按 Escape 后已确认回到 x.com/home。");
            } else if let Some((discard_x, discard_y)) =
                ocr::locate_discard_draft_button(&after_escape.png)?
            {
                automation::log_event("全自动恢复：识别到草稿放弃按钮，点击识别到的按钮坐标。");

                native::click_at_client_position(discard_x, discard_y)?;
                std::thread::sleep(Duration::from_millis(700));

                let after_discard = capture_desktop(
                    &dir,
                    &id,
                    "full-auto-error-state-safe-after-discard",
                    &mut screenshots,
                )?;

                current_url = ocr::recognize_current_url(&after_discard.png)?;

                if current_url.as_deref().is_some_and(ocr::is_x_home_address) {
                    automation::log_event("全自动恢复：点击放弃后已确认回到 x.com/home。");
                } else {
                    automation::log_event(&format!(
                        "全自动恢复：点击放弃后仍未确认首页，当前地址：{current_url:?}；\
                         不再执行盲目点击。"
                    ));
                }
            } else {
                automation::log_event(&format!(
                    "全自动恢复：未识别到可靠的放弃按钮，当前地址：{current_url:?}；\
                     为避免误点，不执行屏幕中心点击。"
                ));
            }
        }
    }
    let seed = next_random_u64();
    // 普通报错优先小步滚动，不随机刷新页面。
    // 只有非报错场景才保留原来的随机刷新策略。
    if !reason.contains("报错") && seed % 2 == 0 {
        automation::native::activate_browser()?;
        automation::log_event(&format!("全自动恢复：{reason}；随机选择 F5 刷新页面。"));
        automation::native::press_f5()?;
        wait_cancel(cancel, Duration::from_secs(3))?;
        let dir = PathBuf::from("data")
            .join("x_assistant")
            .join("screenshots");
        fs::create_dir_all(&dir)?;
        let id = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
        let mut screenshots = Vec::new();
        let check_shot = capture_desktop(&dir, &id, "full-auto-refresh-check", &mut screenshots)?;
        check_cancel(cancel)?;
        anyhow::ensure!(
            check_shot.width >= 800 && check_shot.height >= 600,
            "刷新确认检查截图尺寸异常（{}x{}），停止本轮以免漏处理确认弹窗",
            check_shot.width,
            check_shot.height
        );
        match ocr::recognize_reload_confirmation(&check_shot.png) {
            Ok(true) => {
                automation::log_event(
                    "全自动恢复：识别到浏览器重新加载确认提示，按 Enter 确认刷新。",
                );
                automation::native::press_enter()?;
                wait_cancel(cancel, Duration::from_secs(5))?;
            }
            Ok(false) => {
                automation::log_event("全自动恢复：全桌面截图未发现重新加载确认提示。");
            }
            Err(error) => {
                anyhow::bail!(
                    "无法识别全桌面刷新检查截图，停止本轮以免跳过可能存在的确认弹窗：{error:#}"
                );
            }
        }
        automation::log_event("全自动恢复：F5 刷新检查完成。");
        check_cancel(cancel)
    } else {
        scroll_down_in_small_steps(cancel, reason)
    }
}

fn scroll_down_in_small_steps(cancel: &AtomicBool, reason: &str) -> anyhow::Result<()> {
    scroll_down_random_steps(cancel, reason, 4, 10)
}

pub(super) fn scroll_down_to_complete_post(
    cancel: &AtomicBool,
    reason: &str,
) -> anyhow::Result<()> {
    scroll_down_random_steps(cancel, reason, 1, 3)
}

fn scroll_down_random_steps(
    cancel: &AtomicBool,
    reason: &str,
    min_steps: usize,
    max_steps: usize,
) -> anyhow::Result<()> {
    let scroll_steps = min_steps + (next_random_u64() as usize % (max_steps - min_steps + 1));
    automation::native::activate_browser()?;
    automation::log_event(&format!(
        "全自动准备：{reason}；本次滚动 {scroll_steps} 格，步间节奏随机。"
    ));
    for step in 0..scroll_steps {
        check_cancel(cancel)?;
        automation::native::scroll_down_once()?;
        if step + 1 < scroll_steps {
            let pause_ms = 160 + next_random_u64() % 541;
            wait_cancel(cancel, Duration::from_millis(pause_ms))?;
            if next_random_u64() % 4 == 0 {
                wait_cancel(cancel, Duration::from_millis(350 + next_random_u64() % 651))?;
            }
        }
    }
    let settle_ms = 350 + next_random_u64() % 651;
    wait_cancel(cancel, Duration::from_millis(settle_ms))?;
    automation::log_event(&format!(
        "全自动准备：随机小步滚动完成（{scroll_steps} 格，停稳 {settle_ms} ms），等待下一轮完整帖子识别。"
    ));
    check_cancel(cancel)
}

fn scroll_down_for_random_duration(
    cancel: &AtomicBool,
    min_seconds: u64,
    max_seconds: u64,
    reason: &str,
) -> anyhow::Result<()> {
    check_cancel(cancel)?;
    automation::native::activate_browser()?;
    let seed = next_random_u64();
    let duration_secs = min_seconds + seed % (max_seconds - min_seconds + 1);
    automation::log_event(&format!(
        "全自动准备：{reason}，随机向下滚动 {duration_secs} 秒。"
    ));
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(duration_secs) {
        check_cancel(cancel)?;
        automation::native::scroll_down_once()?;
        let pause_ms = 120 + (started.elapsed().as_nanos() % 281) as u64;
        wait_cancel(cancel, Duration::from_millis(pause_ms))?;
    }
    automation::log_event("全自动准备：向下滚动完成。");
    check_cancel(cancel)
}

pub fn run_visual_reply(
    options: VisionReplyOptions,
    cancel: &AtomicBool,
) -> anyhow::Result<VisionReplyResult> {
    let mut current_stage = "初始化".to_string();
    run_visual_reply_inner(options, cancel, &mut current_stage).map_err(|error| {
        let detailed = error.context(format!("{current_stage}失败"));
        automation::log_event(&format!("X 辅助流程中断：{detailed:#}"));
        detailed
    })
}

fn run_visual_reply_inner(
    options: VisionReplyOptions,
    cancel: &AtomicBool,
    current_stage: &mut String,
) -> anyhow::Result<VisionReplyResult> {
    automation::log_event("========== X 辅助回复流程开始 ==========");
    //wait_cancel(cancel, Duration::from_secs(5))?;

    if native::activate_browser().is_err() {
        super::option::open_chrome()?;
        wait_cancel(cancel, Duration::from_secs(5))?;
    }

    let mut browser_ready = false;
    for _ in 0..8 {
        if native::activate_browser().is_ok() {
            browser_ready = true;
            break;
        }
        wait_cancel(cancel, Duration::from_secs(1))?;
    }

    anyhow::ensure!(browser_ready, "浏览器启动失败");

    wait_cancel(cancel, Duration::from_millis(250))?;

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data")
        .join("x_assistant")
        .join("screenshots");
    fs::create_dir_all(&dir)?;
    let id = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let mut screenshots = Vec::new();
    let run_id = id.to_string();

    let model = resolve_model(&options)?;
    let options = VisionReplyOptions { model, ..options };
    automation::log_event(&format!(
        "视觉模型：{}，接口：{}",
        options.model, options.api_url
    ));

    *current_stage = "阶段 1/3（识别帖子并定位回复按钮）".to_string();
    automation::log_event("========== 阶段 1/3：定位并点击帖子回复图标 ==========");
    let feed_before_cursor_move =
        capture(&dir, &id, "01-feed-before-cursor-move", &mut screenshots)?;
    move_cursor_to_safe_blank_area(
        feed_before_cursor_move.width,
        feed_before_cursor_move.height,
    )?;
    let mut feed = capture(&dir, &id, "01-feed", &mut screenshots)?;
    if options.hover_only {
        automation::log_event("阶段 1/3：仅移动鼠标，不点击。");
        let prompt = action_bar_prompt(feed.width, feed.height);
        let (post, vision_ms) = ask(&options, &feed, &prompt, cancel)?;
        require_ok(&post, "未能识别帖子和操作栏")?;
        let reply_button = validate_or_refind_action_targets(&options, &feed, &post, cancel)?;
        let author_name = post.author_name.trim().to_string();
        let account_handle = post.account_handle.trim().to_string();
        let posted_at = post.posted_at.trim().to_string();
        let post_text = post.post_text.trim().to_string();
        check_cancel(cancel)?;
        native::move_cursor_to_reply(reply_button.x, reply_button.y)?;
        return Ok(VisionReplyResult {
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            reply_text: String::new(),
            completed: false,
            skipped_duplicate: false,
            summary: format!(
                "鼠标已停在回复按钮 ({}, {})，未点击。识图耗时 {} ms。",
                reply_button.x,
                reply_button.y,
                vision_ms.as_millis()
            ),
        });
    }
    let mut post = if options.stage2_ocr {
        const MAX_INCOMPLETE_POST_RETRIES: usize = 2;
        let mut retry = 0;
        loop {
            match identify_post_for_mode(&options, &feed, cancel) {
                Ok(post) => break post,
                Err(error)
                    if retry < MAX_INCOMPLETE_POST_RETRIES && is_incomplete_post_error(&error) =>
                {
                    retry += 1;
                    automation::log_event(&format!(
                        "阶段 1/3：第 {} 次 OCR 未获得完整帖子（{error:#}）；短距离补滚 1–3 格后重新截图识别（最多重试 {MAX_INCOMPLETE_POST_RETRIES} 次）。",
                        retry
                    ));
                    scroll_down_to_complete_post(cancel, "当前画面没有完整可识别的帖子")?;
                    move_cursor_to_safe_blank_area(feed.width, feed.height)?;
                    let stage = format!("01-complete-post-retry-{retry}");
                    feed = capture(&dir, &id, &stage, &mut screenshots)?;
                }
                Err(error) => {
                    if ocr::recognize_post_unavailable_error(&feed.png).unwrap_or(false) {
                        automation::log_event(
                            "阶段 1/3：检测到该帖子已被删除或不可用；安全关闭页面并跳过。",
                        );
                        let _ = close_reply_composer_safely(&dir, &id, &mut screenshots, cancel);
                        return Ok(VisionReplyResult {
                            screenshots,
                            author_name: String::new(),
                            account_handle: String::new(),
                            posted_at: String::new(),
                            post_text: String::new(),
                            reply_text: String::new(),
                            completed: false,
                            skipped_duplicate: true,
                            summary: "帖子已被删除或不可见，已跳过并继续。".to_string(),
                        });
                    }
                    return Err(error);
                }
            }
        }
    } else {
        identify_post_for_mode(&options, &feed, cancel)?
    };
    let mut post_text = post.post_text.trim().to_string();
    let mut author_name = post.author_name.trim().to_string();
    let mut account_handle = post.account_handle.trim().to_string();
    let mut posted_at = post.posted_at.trim().to_string();
    if posted_at.is_empty() {
        return skip_post_and_scroll(
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            cancel,
            false,
            "发布时间未识别；跳过当前帖子并滚动后重新截图",
        );
    }
    let first_is_duplicate =
        binance_tools::db::x_assistant::replied_to_account_within_day_blocking(&account_handle)?;
    if first_is_duplicate {
        automation::log_event(&format!(
            "首条可见帖子账号 {} 过去 24 小时已回复；尝试识别同一截图中的第二条完整帖子。",
            account_handle
        ));
        if options.stage2_ocr {
            match ocr::identify_post_and_action_bar_at(&feed.png, 1) {
                Ok(second_post) => {
                    let second_account = second_post.account_handle.trim().to_string();
                    if !binance_tools::db::x_assistant::replied_to_account_within_day_blocking(
                        &second_account,
                    )? {
                        post = decision_from_local_post(second_post);
                        post_text = post.post_text.trim().to_string();
                        author_name = post.author_name.trim().to_string();
                        account_handle = post.account_handle.trim().to_string();
                        posted_at = post.posted_at.trim().to_string();
                        automation::log_event(&format!(
                            "已跳过已回复账号，改选第二条完整帖子：作者={}，账号={}，时间={}。",
                            author_name, account_handle, posted_at
                        ));
                    } else {
                        automation::log_event(&format!(
                            "第二条完整帖子账号 {} 过去 24 小时也已回复。",
                            second_account
                        ));
                        return Ok(duplicate_post_result(
                            screenshots,
                            author_name,
                            account_handle,
                            posted_at,
                            post_text,
                            "当前截图中的前两条完整帖子账号均已回复，已跳过。",
                        ));
                    }
                }
                Err(error) => {
                    automation::log_event(&format!(
                        "未能安全识别第二条完整帖子，保持跳过行为：{error:#}"
                    ));
                    return Ok(duplicate_post_result(
                        screenshots,
                        author_name,
                        account_handle,
                        posted_at,
                        post_text,
                        "首条可见帖子已回复，但第二条帖子未能安全识别；已跳过当前截图。",
                    ));
                }
            }
        } else {
            return Ok(duplicate_post_result(
                screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                "首条可见帖子账号过去 24 小时已回复；视觉 AI 模式不自动切换到第二条帖子。",
            ));
        }
    }

    let mut reply_button = match validate_or_refind_action_targets(&options, &feed, &post, cancel) {
        Ok(point) => point,
        Err(error) => {
            return skip_post_and_scroll(
                screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                cancel,
                true,
                &format!("帖子与回复按钮关联校验失败：{error:#}"),
            );
        }
    };
    automation::log_event(&format!("成功获取帖子内容，长度：{}", post_text.len()));

    check_cancel(cancel)?;
    let mut target_is_stable = false;
    const MAX_REFRESH_RETRIES: usize = 3;
    for refresh_attempt in 0..=MAX_REFRESH_RETRIES {
        let pre_click_stage = if refresh_attempt == 0 {
            "01-preclick".to_string()
        } else {
            format!("01-preclick-retry-{refresh_attempt}")
        };
        let pre_click = capture(&dir, &id, &pre_click_stage, &mut screenshots)?;
        if target_region_unchanged(&feed, &pre_click, &reply_button, "帖子回复图标")? {
            target_is_stable = true;
            break;
        }

        automation::log_event(&format!(
            "阶段 1/3：第 {} 次复核发现时间线目标区域变化。",
            refresh_attempt + 1
        ));
        anyhow::ensure!(
            refresh_attempt < MAX_REFRESH_RETRIES,
            "时间线连续变化 {} 次，未能取得稳定截图；已停止以避免点击过期坐标",
            MAX_REFRESH_RETRIES + 1
        );

        let refreshed = click_show_new_posts(&options, &pre_click, cancel)?;
        if refreshed {
            check_cancel(cancel)?;
            let random_seed = next_random_u64() as u128;
            let safe_x = (pre_click.width as u128 * (3 + random_seed % 10) / 100) as i32;
            let safe_y = (pre_click.height as u128 * (20 + random_seed / 11 % 61) / 100) as i32;
            native::move_cursor_to_reply(safe_x, safe_y)?;
            automation::log_event(&format!(
                "阶段 1/3：点击‘显示帖子’后将鼠标随机移出时间线识别区域，停在侧栏安全位置 ({safe_x}, {safe_y})。"
            ));
        } else {
            automation::log_event(
                "阶段 1/3：未能确认‘显示帖子’按钮；不点击其他位置，等待后重新截图识别。",
            );
        }

        let wait_ms = random_wait_ms();
        automation::log_event(&format!(
            "阶段 1/3：等待 {wait_ms} ms 后进行第 {} 次重新识别。",
            refresh_attempt + 1
        ));
        wait_cancel(cancel, Duration::from_millis(wait_ms))?;
        let refreshed_feed_stage = format!("01-refreshed-feed-{}", refresh_attempt + 1);
        feed = capture(&dir, &id, &refreshed_feed_stage, &mut screenshots)?;
        move_cursor_to_safe_blank_area(feed.width, feed.height)?;
        let refreshed_post = identify_post_for_mode(&options, &feed, cancel)?;
        post_text = refreshed_post.post_text.trim().to_string();
        anyhow::ensure!(!post_text.is_empty(), "重新识别到的帖子正文为空，已停止");
        author_name = refreshed_post.author_name.trim().to_string();
        account_handle = refreshed_post.account_handle.trim().to_string();
        posted_at = refreshed_post.posted_at.trim().to_string();
        if posted_at.is_empty() {
            return skip_post_and_scroll(
                screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                cancel,
                false,
                "刷新后发布时间未识别；跳过当前帖子并滚动后重新截图",
            );
        }
        reply_button =
            match validate_or_refind_action_targets(&options, &feed, &refreshed_post, cancel) {
                Ok(point) => point,
                Err(error) => {
                    return skip_post_and_scroll(
                        screenshots,
                        author_name,
                        account_handle,
                        posted_at,
                        post_text,
                        cancel,
                        true,
                        &format!("重新识别后帖子与回复按钮关联校验失败：{error:#}"),
                    );
                }
            };
    }
    anyhow::ensure!(
        target_is_stable,
        "重新识别后帖子区域仍不稳定，已停止以避免点击错误帖子"
    );
    automation::log_event(&format!(
        "阶段 1/3：帖子作者识别结果：昵称={}，账号={}，时间={}；正文长度={}字符/{}字节。",
        author_name,
        account_handle,
        posted_at,
        post_text.chars().count(),
        post_text.len()
    ));

    automation::log_event(&format!(
        "阶段 1/3：悬停 AI 回复坐标 ({}, {})，等待 tooltip 后由本地 OCR 验证‘回复’提示。",
        reply_button.x, reply_button.y
    ));
    native::move_cursor_to_reply(reply_button.x, reply_button.y)?;
    wait_cancel(cancel, Duration::from_millis(900))?;
    let tooltip_shot = capture(&dir, &id, "01-reply-tooltip-hover", &mut screenshots)?;
    let tooltip_verified =
        match ocr::verify_reply_tooltip(&tooltip_shot.png, reply_button.x, reply_button.y) {
            Ok(verified) => verified,
            Err(error) => {
                automation::log_event(&format!("悬停回复提示 OCR 失败：{error:#}"));
                false
            }
        };
    if !tooltip_verified {
        automation::log_event(
            "阶段 1/3：悬停后未识别到‘回复’tooltip，发送 Escape 尝试关闭可能打开的回复窗口。",
        );
        native::press_escape()?;
        automation::log_event(
            "阶段 1/3：Escape 已按下并释放；等待 1500 ms 让页面完成关闭动画后再截图核验。",
        );
        wait_cancel(cancel, Duration::from_millis(1_500))?;
        let after_escape = capture(&dir, &id, "01-after-escape", &mut screenshots)?;
        let (close_check_ok, composer_open, close_reason) = if options.stage2_ocr {
            let open = ocr::reply_composer_is_open(&after_escape.png)?;
            (true, open, "由 ppocr-rs 检查回复输入占位文字".to_string())
        } else {
            let (close_check, _) = ask(
                &options,
                &after_escape,
                "检查这张截图中的页面状态。只判断目标帖子的回复弹窗/回复编辑窗口是否仍然打开。若能明确判断，返回 {\"ok\":true,\"composer_open\":true/false,\"reason\":\"简短说明\"}；若截图模糊、状态不确定或无法辨认，返回 {\"ok\":false,\"composer_open\":true,\"reason\":\"无法确认\"}。不要操作页面，不要输出坐标。",
                cancel,
            )?;
            (
                close_check.ok,
                close_check.composer_open,
                close_check.reason,
            )
        };
        anyhow::ensure!(
            close_check_ok && !composer_open,
            "未能确认 Escape 后回复窗口已关闭（{}）；停止本轮以避免在弹窗上滚动或继续操作",
            close_reason
        );
        automation::log_event(&format!(
            "阶段 1/3：截图确认 Escape 后回复窗口已关闭：{}。",
            close_reason
        ));
        return skip_post_and_scroll(
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            cancel,
            true,
            if options.stage2_ocr {
                "ppocr-rs 识别的回复坐标悬停后未发现‘回复’tooltip；已按 Escape 关闭回复窗口；未点击"
            } else {
                "AI 识别的回复坐标悬停后未发现‘回复’tooltip；已按 Escape 关闭回复窗口；未点击"
            },
        );
    }

    automation::log_event("当前选中账号过去 24 小时没有已完成回复记录，继续回复流程。");
    let address_check = capture(&dir, &id, "01-before-reply-click", &mut screenshots)?;
    anyhow::ensure!(
        target_region_unchanged(&feed, &address_check, &reply_button, "回复按钮")?,
        "点击前帖子区域发生变化，已停止"
    );
    let current_url = ocr::recognize_current_url(&address_check.png)?;
    automation::log_event(&format!(
        "点击回复前识别到的浏览器地址：{:?}。",
        current_url
    ));
    anyhow::ensure!(
        current_url.as_deref().is_some_and(ocr::is_x_home_address),
        "当前地址不是 https://x.com/home（或 OCR 无法确认地址），已停止，不点击回复"
    );
    binance_tools::db::x_assistant::insert_reply_record_blocking(
        &run_id,
        &author_name,
        &account_handle,
        &posted_at,
        &post_text,
    )?;
    automation::log_event(&format!(
        "SQLite 正文读回校验一致，流程记录 ID：{run_id}；长度={}字符/{}字节。",
        post_text.chars().count(),
        post_text.len()
    ));
    automation::log_event(&format!(
        "阶段 1/3：地址及帖子复核通过（x.com/home），即将点击回复点 ({}, {})；源截图={}（{}x{}），帖子作者={:?}，账号={:?}，正文={:?}。",
        reply_button.x,
        reply_button.y,
        feed.png.display(),
        feed.width,
        feed.height,
        author_name,
        account_handle,
        truncate(&post_text, 240)
    ));
    native::move_cursor_to_reply(reply_button.x, reply_button.y)?;
    thread::sleep(Duration::from_millis(500));
    native::click_at_client_position(reply_button.x, reply_button.y)?;
    automation::log_event(&format!(
        "阶段 1/3：回复点点击事件已发送，坐标=({}, {})。",
        reply_button.x, reply_button.y
    ));
    *current_stage = "阶段 2/3（识别回复框、生成并输入回复）".to_string();
    automation::log_event("========== 阶段 2/3：确认回复弹窗、生成并提交回复 ==========");
    let wait_ms = 2_000 + next_random_u64() % 1_001;
    automation::log_event(&format!("点击回复后等待 {wait_ms} ms，再截图确认回复弹窗"));
    wait_cancel(cancel, Duration::from_millis(wait_ms))?;

    let composer = capture(&dir, &id, "02-composer", &mut screenshots)?;
    let (input_point, reply_text) = if options.stage2_ocr {
        let ocr_started = std::time::Instant::now();
        let compose_page = ocr::recognize_compose_reply_page(&composer.png)?;

        automation::log_event(&format!(
            "阶段 2/3：回复页面局部 OCR JSON（地址栏及弹窗）：{}",
            serde_json::to_string(&compose_page)?
        ));
        let current_url = compose_page["current_url"].as_str();
        anyhow::ensure!(
            current_url.is_some_and(ocr::is_x_compose_post_address),
            "当前地址不是 https://x.com/compose/post（或 OCR 无法确认），停止识别和回复"
        );
        let expected_account = account_handle.trim().trim_start_matches('@');

        let actual_account = compose_page["reply_target_account"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_start_matches('@');

        anyhow::ensure!(
            !expected_account.is_empty()
                && !actual_account.is_empty()
                && expected_account.eq_ignore_ascii_case(actual_account),
            "回复对象不匹配：预期 @{}，实际 @{}，禁止发送",
            expected_account,
            actual_account
        );

        automation::log_event(&format!(
            "阶段 2/3：回复对象验证通过，预期 @{}，实际 @{}",
            expected_account, actual_account
        ));
        let api_input_point = compose_page["reply_input"].as_object().and_then(|point| {
            Some((
                i32::try_from(point.get("x")?.as_i64()?).ok()?,
                i32::try_from(point.get("y")?.as_i64()?).ok()?,
            ))
        });
        let point = match api_input_point {
            Some(point) => Some(point),
            None => ocr::locate_reply_input(&composer.png, composer.width, composer.height)?,
        };
        let point = match point {
            Some(point) => point,
            None => {
                return recover_from_ocr_composer_failure(
                    &dir,
                    &id,
                    &composer,
                    &mut screenshots,
                    author_name,
                    account_handle,
                    posted_at,
                    post_text,
                    cancel,
                    "弹窗局部 OCR 未识别到‘发布你的回复’占位文字",
                );
            }
        };
        automation::log_event(&format!(
            "阶段 2/3：OCR 匹配到回复输入占位文字，输入位置 ({}, {})，耗时 {} ms。",
            point.0,
            point.1,
            ocr_started.elapsed().as_millis()
        ));
        let prompt = format!(
            "严格遵守下面列出的每一条回复规则；规则之间不得取舍。生成一条不换行的中文回复，只返回回复正文，不要解释或加引号。输出前逐条核对，任一规则不满足就重新生成。\n帖子正文：{post_text}\n回复规则：{}",
            options.reply_rules.trim()
        );
        let reply_text = generate_compliant_reply(&options, &prompt, cancel)?;
        (
            Point {
                x: point.0,
                y: point.1,
            },
            reply_text,
        )
    } else {
        let (composer_decision, _) = ask(
            &options,
            &composer,
            &format!(
                "这是一项单次识图任务：在同一次响应中同时确认回复弹窗/原帖并生成回复，不要要求后续再调用模型。目标原帖正文应与以下内容相符：{post_text}。必须同时看见匹配的原帖上下文和占位文字“发布你的回复”；若页面处于发帖/投票编辑模式、上下文不匹配或状态不清楚，返回 ok=false、composer_open=false、reply_text 为空。确认后，在本次响应里同时提供：1) point：占位文字“发布你的回复”所在行的安全输入位置，使用当前截图像素坐标（x=0..{}，y=0..{}），不能在下方空白区、底部图标工具栏、投票图标或灰色“回复”提交按钮；2) reply_text：严格遵守以下每一条规则、结合原帖生成回复，不得忽略或取舍任何规则。reply_text 是必填字段，不得省略。\n---回复规则开始---\n{}\n---回复规则结束---\n严格返回单个 JSON：{{\"ok\":true,\"composer_open\":true,\"point\":[400,420],\"reply_text\":\"回复正文\",\"reason\":\"\"}}。point 数组顺序为 [x,y]，必须直接对应当前截图像素，不能归一化。截图尺寸为 {}x{}。",
                options.reply_rules.trim(),
                composer.width - 1,
                composer.height - 1,
                composer.width,
                composer.height
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
        validate_composer_input_point(&input_point, composer.width, composer.height, false)?;

        let reply_text = match sanitize_reply(&composer_decision.reply_text, &options.reply_rules) {
            Ok(reply) => reply,
            Err(error) => {
                automation::log_event(&format!(
                    "视觉模型生成的回复不符合规则，转文本模型重写：{error:#}"
                ));
                let prompt = format!(
                    "请严格遵守每一条规则，重写一条不换行的中文回复。上一版未通过校验：{error:#}。只返回回复正文，不要解释或加引号。\n帖子正文：{post_text}\n回复规则：{}",
                    options.reply_rules.trim()
                );
                generate_compliant_reply(&options, &prompt, cancel)?
            }
        };
        (input_point, reply_text)
    };
    if let Err(error) = validate_composer_input_point(
        &input_point,
        composer.width,
        composer.height,
        options.stage2_ocr,
    ) {
        if options.stage2_ocr {
            return recover_from_ocr_composer_failure(
                &dir,
                &id,
                &composer,
                &mut screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                cancel,
                &format!("OCR 占位文字已匹配，但坐标不在安全输入区：{error:#}"),
            );
        }
        return Err(error);
    }
    binance_tools::db::x_assistant::save_reply_text_blocking(&run_id, &reply_text)?;

    let pre_input = capture(&dir, &id, "02-preinput", &mut screenshots)?;
    anyhow::ensure!(
        target_region_unchanged(&composer, &pre_input, &input_point, "回复输入区")?,
        "回复弹窗内容发生变化，输入坐标已过期；已停止"
    );
    native::move_cursor_to_reply(input_point.x, input_point.y)?;
    native::click_at_client_position(input_point.x, input_point.y)?;
    thread::sleep(Duration::from_millis(100));
    native::type_text(&reply_text)?;
    thread::sleep(Duration::from_millis(200));

    automation::log_event("阶段 2/3：回复已输入，截图核对后识别带‘回复’文字的提交按钮。");
    let typed = capture(&dir, &id, "03-typed", &mut screenshots)?;
    let send_button = match ocr::locate_reply_submit_button(
        &typed.png,
        &reply_text,
        typed.width,
        typed.height,
    ) {
        Ok(Some((x, y))) => {
            automation::log_event(&format!(
                "阶段 2/3：本地 OCR 已核对回复文本，图像检测定位提交按钮：({}, {})。",
                x, y
            ));
            Point { x, y }
        }
        ocr_result if options.stage2_ocr => {
            let reason = match ocr_result {
                Ok(None) => "OCR 未能核对已输入的回复文本或定位提交按钮；未点击发送".to_string(),
                Err(error) => {
                    format!("提交前 OCR 识别失败：{error:#}；未点击发送")
                }
                Ok(Some(_)) => unreachable!(),
            };
            return recover_from_ocr_composer_failure(
                &dir,
                &id,
                &typed,
                &mut screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                cancel,
                &reason,
            );
        }
        ocr_result => {
            match ocr_result {
                Ok(None) => automation::log_event(
                    "阶段 2/3：ppocr-rs 未能同时核对回复文本和按钮，回退到 AI 识别。",
                ),
                Err(error) => automation::log_event(&format!(
                    "阶段 2/3：ppocr-rs 失败（{error}），回退到 AI 识别。"
                )),
                Ok(Some(_)) => unreachable!(),
            }
            let (typed_decision, _) = ask(
                &options,
                &typed,
                &format!(
                    "检查输入内容：{reply_text}。只有当这段文字确实显示在原帖下方的回复文本编辑区时，才设置 text_matches=true、reply_field_matches=true、poll_mode=false。若文字出现在投票选项、发帖正文或其他控件中，设置 ok=false、reply_field_matches=false；若是投票编辑界面，poll_mode=true，绝不返回可点击坐标。确认文本正确后，直接根据这张完整截图定位当前弹窗底部黑色“回复”文字按钮的可见边界，返回按钮矩形的几何中心；不要猜测、沿用示例坐标或依据其他运行的按钮位置，不要把回复气泡图标、工具栏或其他按钮当作目标。返回合法 JSON，字段为 ok、text_matches、reply_field_matches、poll_mode、point、reason；point 必须是本张截图中按钮中心的像素整数坐标 [x,y]，范围 x=0..{}、y=0..{}。不要点击任何按钮。截图尺寸 {}x{}。",
                    typed.width - 1,
                    typed.height - 1,
                    typed.width,
                    typed.height
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
            require_point(
                &typed_decision,
                typed.width,
                typed.height,
                "弹窗中的“回复”提交按钮",
            )?
        }
    };

    if !options.auto_send {
        return Ok(VisionReplyResult {
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            reply_text,
            completed: false,
            skipped_duplicate: false,
            summary: "回复已输入并通过截图核对。未点击发送，请人工确认后再发送。".to_string(),
        });
    }

    check_cancel(cancel)?;
    let pre_submit = capture(&dir, &id, "03-presubmit", &mut screenshots)?;
    anyhow::ensure!(
        target_region_unchanged(&typed, &pre_submit, &send_button, "弹窗中的‘回复’按钮")?,
        "回复弹窗内容发生变化，提交按钮坐标已过期；已停止"
    );
    native::move_cursor_to_reply(send_button.x, send_button.y)?;
    *current_stage = "阶段 3/3（点击发送并确认结果）".to_string();
    automation::log_event("========== 阶段 3/3：点击‘回复’并判断回复是否完成 ==========");
    native::click_at_client_position(send_button.x, send_button.y)?;
    automation::log_event("阶段 3/3：已点击‘回复’，等待 2 秒后采集结果截图。");
    thread::sleep(Duration::from_secs(2));

    let mut done = capture(&dir, &id, "04-result", &mut screenshots)?;
    check_cancel(cancel)?;

    // ============================================================
    // 第一次发送结果检查：同一张截图只执行一次 OCR
    // ============================================================

    let ocr_started = std::time::Instant::now();

    let status = match recognize_send_status(&done.png) {
        Ok(status) => {
            automation::log_event(&format!(
                "阶段 3/3：ppocr-rs 耗时 {} ms，发送状态识别完成。",
                ocr_started.elapsed().as_millis()
            ));
            status
        }
        Err(error) => {
            automation::log_event(&format!(
                "阶段 3/3：ppocr-rs 耗时 {} ms，OCR 识别失败：{error:#}；将使用 AI 复核，不自动重试。",
                ocr_started.elapsed().as_millis()
            ));

            // OCR 失败时不自动重试，避免重复发送。
            // 后续继续原有 AI 复核流程。
            // 此处不提前返回。
            Default::default()
        }
    };

    // 已经发送过
    if status.already_sent {
        automation::log_event("阶段 3/3：X 提示该回复已经发送过；标记该账号已处理，不再重发。");

        return already_sent_result(
            &dir,
            &id,
            screenshots,
            &run_id,
            author_name,
            account_handle,
            posted_at,
            post_text,
            reply_text,
            cancel,
        );
    }

    // 发送成功
    if status.sent {
        automation::log_event("阶段 3/3：PP-OCR 检测到发送成功提示。");

        if let Err(error) =
            binance_tools::db::x_assistant::mark_last_step_completed_blocking(&run_id)
        {
            automation::log_event(&format!(
                "回复已确认发送，但 SQLite 完成状态更新失败：{error:#}"
            ));
        }

        return Ok(VisionReplyResult {
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            reply_text,
            completed: true,
            skipped_duplicate: false,
            summary: "ppocr-rs 检测到发送成功提示。".to_string(),
        });
    }

    // 平台限流
    if status.rate_limited {
        automation::log_event("阶段 3/3：检测到平台限流提示；不重试、不刷新，停止全自动发送。");

        return Err(anyhow::Error::new(RateLimitDetected));
    }

    // 是否存在可重试发送错误
    let retryable_error = status.retryable_error;

    if retryable_error {
        automation::log_event("阶段 3/3：截图识别到发送错误；等待 5 秒后重新截图并确认发送状态。");

        wait_cancel(cancel, Duration::from_secs(5))?;

        let retry_ready = capture(&dir, &id, "04-retry-check", &mut screenshots)?;

        check_cancel(cancel)?;

        // ========================================================
        // 重试前状态检查：一次 OCR 判断全部状态
        // ========================================================

        let retry_status = match recognize_send_status(&retry_ready.png) {
            Ok(status) => status,
            Err(error) => {
                automation::log_event(&format!(
                    "阶段 3/3：重试前 OCR 识别失败：{error:#}；无法确认发送状态，放弃重试。"
                ));

                let _ = close_reply_composer_safely(&dir, &id, &mut screenshots, cancel);

                anyhow::bail!("重试前无法确认发送状态，已放弃重复提交：{error:#}");
            }
        };

        // 已经发送过
        if retry_status.already_sent {
            automation::log_event("阶段 3/3：重试前 X 已提示该回复发送过；不再点击提交。");

            return already_sent_result(
                &dir,
                &id,
                screenshots,
                &run_id,
                author_name,
                account_handle,
                posted_at,
                post_text,
                reply_text,
                cancel,
            );
        }

        // 限流
        if retry_status.rate_limited {
            automation::log_event(
                "阶段 3/3：等待期间检测到平台限流提示；不提交重试，停止全自动发送。",
            );

            return Err(anyhow::Error::new(RateLimitDetected));
        }

        // 已发送成功
        if retry_status.sent {
            automation::log_event("阶段 3/3：等待期间已出现发送成功提示，不再重复提交。");

            if let Err(error) =
                binance_tools::db::x_assistant::mark_last_step_completed_blocking(&run_id)
            {
                automation::log_event(&format!(
                    "回复已确认发送，但 SQLite 完成状态更新失败：{error:#}"
                ));
            }

            return Ok(VisionReplyResult {
                screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                reply_text,
                completed: true,
                skipped_duplicate: false,
                summary: "等待重试期间检测到发送成功，未重复提交。".to_string(),
            });
        }

        // 重试前再次检测到发送错误，允许执行唯一一次重试。
        if !retry_status.retryable_error {
            automation::log_event(
                "阶段 3/3：重试前没有检测到明确的发送错误；将使用 AI 复核，不自动重复提交。",
            );

            // 不能确认失败，就不重复点击。
            let _ = close_reply_composer_safely(&dir, &id, &mut screenshots, cancel);

            anyhow::bail!("重试前未能确认发送失败，已放弃重复提交，避免重复回复");
        }

        // ========================================================
        // 定位重试按钮
        // ========================================================

        let retry_button = match ocr::locate_reply_submit_button(
            &retry_ready.png,
            &reply_text,
            retry_ready.width,
            retry_ready.height,
        ) {
            Ok(Some((x, y))) => Point { x, y },

            Ok(None) => {
                let _ = native::press_escape();

                anyhow::bail!(
                    "发送错误后 5 秒截图中未能同时确认回复文本和提交按钮；已放弃重试并关闭回复弹窗"
                );
            }

            Err(error) => {
                let _ = native::press_escape();

                anyhow::bail!(
                    "发送错误后重新识别提交按钮失败：{error:#}；已放弃重试并关闭回复弹窗"
                );
            }
        };

        // 重试前坐标安全校验
        let retry_pre_submit = capture(&dir, &id, "04-retry-presubmit", &mut screenshots)?;

        anyhow::ensure!(
            target_region_unchanged(
                &retry_ready,
                &retry_pre_submit,
                &retry_button,
                "重试时的弹窗提交按钮"
            )?,
            "等待重试期间回复弹窗发生变化；已停止，避免点击过期坐标"
        );

        native::move_cursor_to_reply(retry_button.x, retry_button.y)?;

        automation::log_event(&format!(
            "阶段 3/3：间隔 5 秒后进行唯一一次重试，提交坐标 ({}, {})。",
            retry_button.x, retry_button.y
        ));

        native::click_at_client_position(retry_button.x, retry_button.y)?;

        thread::sleep(Duration::from_secs(2));

        done = capture(&dir, &id, "05-retry-result", &mut screenshots)?;

        check_cancel(cancel)?;

        // ========================================================
        // 重试结果检查：一次 OCR 判断全部状态
        // ========================================================

        let status = match recognize_send_status(&done.png) {
            Ok(status) => status,
            Err(error) => {
                automation::log_event(&format!(
                    "阶段 3/3：唯一一次重试后 OCR 识别失败：{error:#}；无法确认发送结果。"
                ));

                let _ = close_reply_composer_safely(&dir, &id, &mut screenshots, cancel);

                return Ok(VisionReplyResult {
                    screenshots,
                    author_name,
                    account_handle,
                    posted_at,
                    post_text,
                    reply_text,
                    completed: false,
                    skipped_duplicate: false,
                    summary: "重试后无法确认发送结果，已停止自动操作。".to_string(),
                });
            }
        };

        // 已经发送过
        if status.already_sent {
            automation::log_event("阶段 3/3：重试后 X 提示该回复已经发送过；标记完成，不再重发。");

            return already_sent_result(
                &dir,
                &id,
                screenshots,
                &run_id,
                author_name,
                account_handle,
                posted_at,
                post_text,
                reply_text,
                cancel,
            );
        }

        // 发送成功
        if status.sent {
            automation::log_event("阶段 3/3：重试后截图检测到发送成功提示。");

            if let Err(error) =
                binance_tools::db::x_assistant::mark_last_step_completed_blocking(&run_id)
            {
                automation::log_event(&format!(
                    "回复已确认发送，但 SQLite 完成状态更新失败：{error:#}"
                ));
            }

            return Ok(VisionReplyResult {
                screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                reply_text,
                completed: true,
                skipped_duplicate: false,
                summary: "发送错误后重试一次，已检测到发送成功。".to_string(),
            });
        }

        // 限流
        if status.rate_limited {
            automation::log_event("阶段 3/3：重试后检测到平台限流提示；立即停止全自动发送。");

            return Err(anyhow::Error::new(RateLimitDetected));
        }

        // 重试后仍然发送错误
        if status.retryable_error {
            automation::log_event(
                "阶段 3/3：唯一一次重试后仍检测到发送错误；已安全关闭回复弹窗并继续下一步，不中断程序。",
            );

            let _ = close_reply_composer_safely(&dir, &id, &mut screenshots, cancel);

            return Ok(VisionReplyResult {
                screenshots,
                author_name,
                account_handle,
                posted_at,
                post_text,
                reply_text,
                completed: false,
                skipped_duplicate: false,
                summary: "发送重试后仍出现错误，已安全关闭弹窗并继续。".to_string(),
            });
        }
    }

    let (done_decision, _) = ask(
        &options,
        &done,
        "判断刚才的回复是否已经成功发送。请优先检查页面底部或其他位置是否出现 X 的明确成功提示，例如‘你的帖子已发送’或‘你的回复已发送’；只要出现该提示，就必须返回 ok=true、completed=true，即使回复弹窗或草稿框仍然可见（页面可能尚未关闭弹窗）。如果没有成功提示，再检查回复框是否关闭或刚发送的回复是否已出现在原帖下方。只有看不到任何成功证据、且输入内容仍留在未发送框中时，才返回 ok=false、completed=false。严格返回 JSON：{\"ok\":true,\"completed\":true,\"reason\":\"\"}。",
        cancel,
    )?;
    if !done_decision.ok || !done_decision.completed {
        automation::log_event(&format!(
            "无法确认回复已发送（{}）；将自动关闭回复弹窗并继续下一步，不中断程序。",
            fallback_reason(&done_decision, "完成状态不明确")
        ));
        let _ = close_reply_composer_safely(&dir, &id, &mut screenshots, cancel);
        return Ok(VisionReplyResult {
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            reply_text,
            completed: false,
            skipped_duplicate: false,
            summary: format!("无法确认回复已发送，已安全关闭弹窗并继续。"),
        });
    }

    if let Err(error) = binance_tools::db::x_assistant::mark_last_step_completed_blocking(&run_id) {
        automation::log_event(&format!(
            "回复已确认发送，但 SQLite 完成状态更新失败：{error:#}"
        ));
    }
    Ok(VisionReplyResult {
        screenshots,
        author_name,
        account_handle,
        posted_at,
        post_text,
        reply_text,
        completed: true,
        skipped_duplicate: false,
        summary: "截图确认回复已发送。".to_string(),
    })
}

fn already_sent_result(
    dir: &Path,
    id: &impl std::fmt::Display,
    mut screenshots: Vec<PathBuf>,
    run_id: &str,
    author_name: String,
    account_handle: String,
    posted_at: String,
    post_text: String,
    reply_text: String,
    cancel: &AtomicBool,
) -> anyhow::Result<VisionReplyResult> {
    if let Err(error) = binance_tools::db::x_assistant::mark_last_step_completed_blocking(run_id) {
        automation::log_event(&format!(
            "平台已确认回复之前发送过，但 SQLite 去重记录更新失败：{error:#}"
        ));
    }
    if let Err(error) = close_reply_composer_safely(dir, id, &mut screenshots, cancel) {
        automation::log_event(&format!(
            "关闭已发送回复窗口时出现异常（已忽略并继续）：{error:#}"
        ));
    }
    Ok(VisionReplyResult {
        screenshots,
        author_name,
        account_handle,
        posted_at,
        post_text,
        reply_text,
        completed: false,
        skipped_duplicate: true,
        summary: "X 已提示该回复发送过；已关闭编辑框并跳过，未再次发送。".to_string(),
    })
}

fn close_reply_composer_safely(
    dir: &Path,
    id: &impl std::fmt::Display,
    screenshots: &mut Vec<PathBuf>,
    cancel: &AtomicBool,
) -> anyhow::Result<()> {
    for attempt in 0..2 {
        native::press_escape()?;
        wait_cancel(cancel, Duration::from_millis(700))?;
        let shot = capture(
            dir,
            id,
            &format!("05-network-error-close-{}", attempt + 1),
            screenshots,
        )?;

        if let Some((discard_x, discard_y)) = ocr::locate_discard_draft_button(&shot.png)? {
            automation::log_event(&format!(
                "阶段 3/3：网络错误后识别到草稿确认框，点击‘放弃’关闭未发送回复 ({discard_x}, {discard_y})。"
            ));
            native::click_at_client_position(discard_x, discard_y)?;
            wait_cancel(cancel, Duration::from_millis(700))?;
            let after_discard = capture(
                dir,
                id,
                &format!("05-network-error-discard-{}", attempt + 1),
                screenshots,
            )?;
            if !ocr::reply_composer_is_open(&after_discard.png)? {
                automation::log_event("阶段 3/3：草稿已放弃，确认回复窗口关闭。");
                return Ok(());
            }
        } else if ocr::recognize_save_draft_confirmation(&shot.png)? {
            automation::log_event(
                "阶段 3/3：发现草稿确认框但未精确匹配‘放弃’按钮坐标，尝试直接点击弹窗中央偏下区域‘放弃’。",
            );
            let fallback_discard_x = shot.width as i32 * 50 / 100;
            let fallback_discard_y = shot.height as i32 * 60 / 100;
            native::click_at_client_position(fallback_discard_x, fallback_discard_y)?;
            wait_cancel(cancel, Duration::from_millis(700))?;
            let after_fallback = capture(
                dir,
                id,
                &format!("05-network-error-fallback-discard-{}", attempt + 1),
                screenshots,
            )?;
            if !ocr::reply_composer_is_open(&after_fallback.png)? {
                automation::log_event("阶段 3/3：点击备用位置后草稿已放弃，确认回复窗口关闭。");
                return Ok(());
            }
        } else if !ocr::reply_composer_is_open(&shot.png)? {
            automation::log_event("阶段 3/3：确认网络错误后的回复窗口已关闭。");
            return Ok(());
        }

        if attempt == 0 {
            let dismiss_x = (shot.width as f32 * 0.23).round() as i32;
            let dismiss_y = (shot.height as f32 * 0.50).round() as i32;
            automation::log_event(
                "阶段 3/3：Escape 后回复窗口仍打开；点击窗口外安全区域，再尝试关闭一次。",
            );
            native::click_at_client_position(dismiss_x, dismiss_y)?;
            wait_cancel(cancel, Duration::from_millis(400))?;
        }
    }

    anyhow::bail!("网络错误重试失败；两次关闭尝试后回复窗口仍未关闭，已停止自动操作")
}

fn identify_post_for_mode(
    options: &VisionReplyOptions,
    feed: &Shot,
    cancel: &AtomicBool,
) -> anyhow::Result<VisionDecision> {
    check_cancel(cancel)?;
    if !options.stage2_ocr {
        return identify_post_once(options, feed, cancel);
    }

    let started = std::time::Instant::now();
    let local = ocr::identify_post_and_action_bar(&feed.png)?;
    automation::log_event(&format!(
        "阶段 1/3：ppocr-rs 本地识别帖子：作者={:?}，账号={:?}，时间={:?}，正文={:?}；评论/互动栏回复坐标={:?}，耗时={} ms。",
        local.author_name,
        local.account_handle,
        local.posted_at,
        truncate(&local.post_text, 400),
        local.reply_point,
        started.elapsed().as_millis()
    ));
    Ok(VisionDecision {
        ok: true,
        reason: "帖子内容与互动栏由 ppocr-rs 本地识别".to_string(),
        post_text: local.post_text,
        author_name: local.author_name,
        account_handle: local.account_handle,
        posted_at: local.posted_at,
        reply_text: String::new(),
        point: None,
        reply_point: Some(Point {
            x: local.reply_point.0,
            y: local.reply_point.1,
        }),
        repost_point: Some(Point {
            x: local.repost_point.0,
            y: local.repost_point.1,
        }),
        like_point: Some(Point {
            x: local.like_point.0,
            y: local.like_point.1,
        }),
        post_text_bottom_y: Some(local.post_text_bottom_y),
        post_bottom_y: Some(local.post_bottom_y),
        refresh_posts: false,
        composer_open: false,
        text_matches: false,
        reply_field_matches: false,
        poll_mode: false,
        completed: false,
    })
}

fn identify_post_once(
    options: &VisionReplyOptions,
    feed: &Shot,
    cancel: &AtomicBool,
) -> anyhow::Result<VisionDecision> {
    check_cancel(cancel)?;
    let prompt = action_bar_prompt(feed.width, feed.height);
    automation::log_event(&format!(
        "阶段 1/3：提交给 AI 的时间线截图：{}（{}x{}，stage={}）。",
        feed.png.display(),
        feed.width,
        feed.height,
        feed.stage
    ));
    let (post, _) = ask(options, feed, &prompt, cancel)?;
    automation::log_event(&format!(
        "阶段 1/3：AI 帖子识别结果：作者={:?}，账号={:?}，时间={:?}，正文={:?}；正文底部 y={:?}，整帖底部 y={:?}；回复点={:?}，转发点={:?}，点赞点={:?}；截图={}（{}x{}）。",
        post.author_name,
        post.account_handle,
        post.posted_at,
        truncate(&post.post_text, 400),
        post.post_text_bottom_y,
        post.post_bottom_y,
        post.reply_point,
        post.repost_point,
        post.like_point,
        feed.png.display(),
        feed.width,
        feed.height
    ));
    anyhow::ensure!(!post.post_text.trim().is_empty(), "AI 识别到的帖子内容为空");
    Ok(post)
}

fn recover_from_ocr_composer_failure(
    dir: &Path,
    id: &impl std::fmt::Display,
    composer: &Shot,
    screenshots: &mut Vec<PathBuf>,
    author_name: String,
    account_handle: String,
    posted_at: String,
    post_text: String,
    cancel: &AtomicBool,
    reason: &str,
) -> anyhow::Result<VisionReplyResult> {
    let dismiss_x = (composer.width as f32 * 0.23).round() as i32;
    let dismiss_y = (composer.height as f32 * 0.50).round() as i32;
    automation::log_event(&format!(
        "阶段 2/3：{reason}；将尝试关闭回复弹窗，并在确认关闭前禁止刷新或滚动。"
    ));
    let mut recovered_shot = None;
    for attempt in 0..2 {
        native::press_escape()?;
        wait_cancel(cancel, Duration::from_millis(700))?;
        let shot = capture(
            dir,
            id,
            &format!("02-ocr-recovery-{}", attempt + 1),
            screenshots,
        )?;
        if let Some((discard_x, discard_y)) = ocr::locate_discard_draft_button(&shot.png)? {
            automation::log_event(&format!(
                "阶段 2/3：OCR 确认 X 的‘保存帖子？’草稿提示及‘放弃’按钮；点击放弃未发送草稿 ({discard_x}, {discard_y})。"
            ));
            native::click_at_client_position(discard_x, discard_y)?;
            wait_cancel(cancel, Duration::from_millis(700))?;
            let after_discard = capture(dir, id, "02-ocr-discard-draft", screenshots)?;
            if !ocr::reply_composer_is_open(&after_discard.png)? {
                recovered_shot = Some(after_discard);
                break;
            }
            automation::log_event(
                "阶段 2/3：点击‘放弃’后 OCR 仍检测到回复框；继续执行受限关闭核验。",
            );
        } else if ocr::recognize_save_draft_confirmation(&shot.png)? {
            anyhow::bail!(
                "检测到 X 的‘保存帖子？’提示，但 OCR 未能安全定位‘放弃’按钮；停止操作，请手动选择是否丢弃草稿。"
            )
        } else if !ocr::reply_composer_is_open(&shot.png)? {
            recovered_shot = Some(shot);
            break;
        }
        automation::log_event(&format!(
            "阶段 2/3：第 {} 次关闭尝试后回复框仍可见{}。",
            attempt + 1,
            if attempt == 0 {
                "；点击弹窗外区域再试一次"
            } else {
                "；停止自动操作"
            }
        ));
        if attempt == 0 {
            native::click_at_client_position(dismiss_x, dismiss_y)?;
            wait_cancel(cancel, Duration::from_millis(400))?;
        }
    }
    let Some(recovered_shot) = recovered_shot else {
        anyhow::bail!(
            "OCR 回复框定位失败，且两次关闭尝试后弹窗仍未关闭；为避免刷新或滚动改变页面状态，已停止自动操作。请手动关闭回复弹窗后再重试。"
        )
    };
    if AUTO_SCROLL_ENABLED {
        scroll_down_for_random_duration(
            cancel,
            5,
            10,
            "OCR 回复框定位失败，关闭回复弹窗后下滑查找其他帖子",
        )?;
    } else {
        automation::log_event("自动滚动已关闭；OCR 恢复关闭弹窗后不滚动。");
    }
    Ok(VisionReplyResult {
        screenshots: std::mem::take(screenshots),
        author_name,
        account_handle,
        posted_at,
        post_text,
        reply_text: String::new(),
        completed: false,
        skipped_duplicate: false,
        summary: format!(
            "继续扫描下一帖；OCR 回复框定位失败，已关闭回复框并滚动。原因：{reason}。恢复截图：{}",
            recovered_shot.png.display()
        ),
    })
}

fn skip_post_and_scroll(
    screenshots: Vec<PathBuf>,
    author_name: String,
    account_handle: String,
    posted_at: String,
    post_text: String,
    cancel: &AtomicBool,
    reply_button_missed: bool,
    reason: &str,
) -> anyhow::Result<VisionReplyResult> {
    automation::log_event(&format!(
        "阶段 1/3：{reason}；不点击回复按钮、不生成或保存回复。"
    ));
    if AUTO_SCROLL_ENABLED {
        scroll_down_for_random_duration(
            cancel,
            5,
            10,
            "帖子与回复按钮关联不可靠，跳过并查找下一帖",
        )?;
    } else {
        automation::log_event("自动滚动已关闭；本轮跳过当前帖子，不滚动。");
    }
    let summary = if reply_button_missed {
        format!("继续扫描下一帖；回复按钮未识别；{reason}。未执行回复。")
    } else {
        format!("继续扫描下一帖；{reason}。未执行回复。")
    };
    Ok(VisionReplyResult {
        screenshots,
        author_name,
        account_handle,
        posted_at,
        post_text,
        reply_text: String::new(),
        completed: false,
        skipped_duplicate: false,
        summary,
    })
}

pub(super) fn is_incomplete_post_error(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    [
        "未识别到时间线帖子",
        "未识别到帖子作者",
        "未识别到帖子账号",
        "未识别到帖子发布时间",
        "帖子正文为空",
        "未识别到回复数量或浏览数量",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn decision_from_local_post(post: ocr::LocalPostIdentification) -> VisionDecision {
    VisionDecision {
        ok: true,
        reason: "本地 OCR 识别的第二条完整帖子".to_string(),
        post_text: post.post_text,
        author_name: post.author_name,
        account_handle: post.account_handle,
        posted_at: post.posted_at,
        reply_text: String::new(),
        point: None,
        reply_point: Some(Point {
            x: post.reply_point.0,
            y: post.reply_point.1,
        }),
        repost_point: Some(Point {
            x: post.repost_point.0,
            y: post.repost_point.1,
        }),
        like_point: Some(Point {
            x: post.like_point.0,
            y: post.like_point.1,
        }),
        post_text_bottom_y: Some(post.post_text_bottom_y),
        post_bottom_y: Some(post.post_bottom_y),
        refresh_posts: false,
        composer_open: false,
        text_matches: false,
        reply_field_matches: false,
        poll_mode: false,
        completed: false,
    }
}

fn duplicate_post_result(
    screenshots: Vec<PathBuf>,
    author_name: String,
    account_handle: String,
    posted_at: String,
    post_text: String,
    summary: &str,
) -> VisionReplyResult {
    VisionReplyResult {
        screenshots,
        author_name,
        account_handle,
        posted_at,
        post_text,
        reply_text: String::new(),
        completed: false,
        skipped_duplicate: true,
        summary: summary.to_string(),
    }
}

fn capture_desktop(
    dir: &Path,
    id: &impl std::fmt::Display,
    stage: &str,
    screenshots: &mut Vec<PathBuf>,
) -> anyhow::Result<Shot> {
    let bmp = dir.join(format!("{id}-{stage}.bmp"));
    automation::log_event(&format!("开始采集全桌面截图：{stage}"));
    let capture_started = std::time::Instant::now();
    native::capture_desktop(&bmp)?;
    let image = image::open(&bmp)?;
    let png = bmp.with_extension("png");
    image.save(&png)?;
    automation::log_event(&format!(
        "全桌面截图采集完成：{}，尺寸 {}x{}，耗时 {} ms",
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
    let elapsed_ms = capture_started.elapsed().as_millis();
    let file_size = std::fs::metadata(&png).map(|m| m.len()).unwrap_or(0);

    automation::log_event(&format!(
        "截图保存验证：工作目录={}，目标绝对路径={}，文件存在={}，文件大小={} bytes，耗时={} ms",
        std::env::current_dir()?.display(),
        std::path::absolute(&png)?.display(),
        png.is_file(),
        file_size,
        elapsed_ms
    ));

    automation::log_event(&format!(
        "阶段截图采集完成：{}，尺寸 {}x{}，耗时 {} ms",
        png.display(),
        image.width(),
        image.height(),
        elapsed_ms
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
    Ok(changed_ratio <= 0.05)
}

fn click_show_new_posts(
    options: &VisionReplyOptions,
    shot: &Shot,
    cancel: &AtomicBool,
) -> anyhow::Result<bool> {
    let prompt = format!(
        "检查截图顶部时间线区域。如果明确看见 X 的新帖子提示按钮，按钮文字类似‘显示 35 帖子’或‘显示帖子’，返回 ok=true、refresh_posts=true 和该按钮中心点；不要把‘显示更多’、导航、广告按钮识别成它。按钮中心必须在截图像素坐标 x={}..={}、y=0..={} 范围内；若不能确认按钮中心满足此范围，返回 refresh_posts=false 且不提供坐标。找不到这个确切的新帖子提示时返回 ok=true、refresh_posts=false、point=null。point 使用本截图的像素 [x,y] 坐标。只返回包含 ok、refresh_posts、point、reason 字段的 JSON。截图尺寸 {}x{}。",
        (shot.width as f32 * 0.20) as u32,
        (shot.width as f32 * 0.80) as u32,
        (shot.height as f32 * 0.40) as u32,
        shot.width,
        shot.height
    );
    let point = if options.stage2_ocr {
        ocr::locate_new_posts_notice(&shot.png)?.map(|(x, y)| Point { x, y })
    } else {
        let (decision, _) = ask(options, shot, &prompt, cancel)?;
        if !decision.ok || !decision.refresh_posts {
            automation::log_event("阶段 1/3：未确认‘显示帖子’更新按钮，不执行刷新点击。");
            return Ok(false);
        }
        Some(require_point(
            &decision,
            shot.width,
            shot.height,
            "显示帖子按钮",
        )?)
    };
    let Some(point) = point else {
        automation::log_event("阶段 1/3：OCR 未确认‘显示帖子’更新按钮，不执行刷新点击。");
        return Ok(false);
    };
    anyhow::ensure!(
        point.y <= (shot.height as f32 * 0.40) as i32
            && point.x >= (shot.width as f32 * 0.20) as i32
            && point.x <= (shot.width as f32 * 0.80) as i32,
        "AI 返回的‘显示帖子’坐标 ({}, {}) 超出顶部安全区域（x {}..={}，y <= {}，截图高度的 40%），未点击",
        point.x,
        point.y,
        (shot.width as f32 * 0.20) as i32,
        (shot.width as f32 * 0.80) as i32,
        (shot.height as f32 * 0.40) as i32
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

fn move_cursor_to_safe_blank_area(width: u32, height: u32) -> anyhow::Result<()> {
    let seed = next_random_u64() as u128;
    let on_right = seed % 2 == 1;
    // 停在窗口最外侧的留白边缘，避开左右侧栏和中间时间线内容。
    let x_percent = if on_right {
        98 + seed / 2 % 2
    } else {
        1 + seed / 2 % 2
    };
    let y_percent = 15 + seed / 19 % 71;
    let x = (width as u128 * x_percent / 100) as i32;
    let y = (height as u128 * y_percent / 100) as i32;
    native::move_cursor_to_reply(x, y)?;
    automation::log_event(&format!(
        "阶段 1/3：识别期间鼠标停在{}侧边缘安全区域 ({x}, {y})，避免遮挡时间线内容。",
        if on_right { "右" } else { "左" }
    ));
    Ok(())
}

fn random_wait_ms() -> u64 {
    2_000 + next_random_u64() % 3_001
}

fn next_random_u64() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut value = time ^ SEQUENCE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn ask(
    options: &VisionReplyOptions,
    shot: &Shot,
    prompt: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<(VisionDecision, Duration)> {
    check_cancel(cancel)?;
    let encoded = model_png(shot)?;

    automation::log_event(&format!(
        "[{}] AI 视觉请求输入：截图={}（{}x{}），提示词长度={} 字符。",
        phase_label(&shot.stage),
        shot.png.display(),
        shot.width,
        shot.height,
        prompt.chars().count()
    ));
    let started = std::time::Instant::now();
    let response = binance_tools::ai::send_openai_vision_cancellable_with_response(
        &options.api_url,
        &options.model,
        SYSTEM_PROMPT,
        prompt,
        &encoded,
        Duration::from_secs(120), // 恢复为 120 秒超时
        cancel,
    );
    let elapsed = started.elapsed();

    match &response {
        Ok((content, response_body)) => {
            automation::log_event(&format!(
                "[{}] AI HTTP response_body（{}ms）：{}",
                phase_label(&shot.stage),
                elapsed.as_millis(),
                response_body
            ));
            automation::log_event(&format!(
                "[{}] AI 返回 content（完整 JSON）：{}",
                phase_label(&shot.stage),
                content
            ));
        }
        Err(e) => automation::log_event(&format!(
            "[{}] AI 请求失败 ({}ms)：{}",
            phase_label(&shot.stage),
            elapsed.as_millis(),
            e
        )),
    }

    log_vision_time(shot, elapsed, response.is_ok());
    let (content, _response_body) = response?;
    let decision = match parse_decision(&content) {
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
    let mut decision = decision;
    convert_normalized_coordinates(&mut decision, shot.width, shot.height)?;
    automation::log_event(&format!(
        "[{}] AI 解析并换算后的完整数据：{:?}；坐标已从 0..1000 归一化值换算为截图像素，截图={}（{}x{}）。",
        phase_label(&shot.stage),
        decision,
        shot.png.display(),
        shot.width,
        shot.height
    ));

    Ok((decision, elapsed))
}

fn phase_label(stage: &str) -> &'static str {
    match stage {
        "01-feed" | "01-preclick" => "阶段 1/3",
        stage
            if stage.starts_with("01-refreshed-feed")
                || stage.starts_with("01-refreshed-preclick") =>
        {
            "阶段 1/3"
        }
        "02-composer" | "02-preinput" | "03-typed" => "阶段 2/3",
        "03-presubmit" | "04-result" => "阶段 3/3",
        _ => "未分类阶段",
    }
}

fn log_vision_time(shot: &Shot, elapsed: Duration, ok: bool) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data")
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
    let mut image = image::open(&shot.png)?.to_rgba8();
    draw_coordinate_grid(&mut image);
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgba8(image).write_to(
        &mut std::io::Cursor::new(&mut encoded),
        image::ImageFormat::Png,
    )?;
    Ok(encoded)
}

fn draw_coordinate_grid(image: &mut image::RgbaImage) {
    const GRID_STEP: u32 = 100;
    const GRID_COLOR: [u8; 3] = [0, 220, 255];
    const LABEL_BACKGROUND: [u8; 3] = [0, 0, 0];
    const LABEL_COLOR: [u8; 3] = [255, 255, 255];

    for x in (0..image.width()).step_by(GRID_STEP as usize) {
        for y in 0..image.height() {
            blend_grid_pixel(image, x, y, GRID_COLOR, 120);
        }
        draw_grid_number(
            image,
            x.saturating_add(2),
            2,
            x,
            LABEL_BACKGROUND,
            LABEL_COLOR,
        );
    }
    for y in (0..image.height()).step_by(GRID_STEP as usize) {
        for x in 0..image.width() {
            blend_grid_pixel(image, x, y, GRID_COLOR, 120);
        }
        draw_grid_number(
            image,
            2,
            y.saturating_add(2),
            y,
            LABEL_BACKGROUND,
            LABEL_COLOR,
        );
    }
}

fn blend_grid_pixel(image: &mut image::RgbaImage, x: u32, y: u32, color: [u8; 3], alpha: u16) {
    if let Some(pixel) = image.get_pixel_checked(x, y).copied() {
        let blended: [u8; 3] = std::array::from_fn(|index| {
            ((pixel[index] as u16 * (255 - alpha) + color[index] as u16 * alpha) / 255) as u8
        });
        image.put_pixel(x, y, image::Rgba([blended[0], blended[1], blended[2], 255]));
    }
}

fn draw_grid_number(
    image: &mut image::RgbaImage,
    left: u32,
    top: u32,
    value: u32,
    background: [u8; 3],
    foreground: [u8; 3],
) {
    let digits = value.to_string();
    let width = digits.len() as u32 * 4 + 2;
    for y in 0..7 {
        for x in 0..width {
            let px = left.saturating_add(x);
            let py = top.saturating_add(y);
            if px < image.width() && py < image.height() {
                image.put_pixel(
                    px,
                    py,
                    image::Rgba([background[0], background[1], background[2], 255]),
                );
            }
        }
    }
    for (index, digit) in digits.bytes().enumerate() {
        let Some(rows) = grid_digit(digit) else {
            continue;
        };
        let digit_left = left + 1 + index as u32 * 4;
        for (y, row) in rows.into_iter().enumerate() {
            for x in 0..3 {
                if row & (1 << (2 - x)) != 0 {
                    let px = digit_left + x;
                    let py = top + 1 + y as u32;
                    if px < image.width() && py < image.height() {
                        image.put_pixel(
                            px,
                            py,
                            image::Rgba([foreground[0], foreground[1], foreground[2], 255]),
                        );
                    }
                }
            }
        }
    }
}

fn grid_digit(digit: u8) -> Option<[u8; 5]> {
    Some(match digit {
        b'0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        b'1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        b'2' => [0b110, 0b001, 0b111, 0b100, 0b111],
        b'3' => [0b110, 0b001, 0b111, 0b001, 0b110],
        b'4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        b'5' => [0b111, 0b100, 0b110, 0b001, 0b110],
        b'6' => [0b011, 0b100, 0b111, 0b101, 0b111],
        b'7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        b'8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        b'9' => [0b111, 0b101, 0b111, 0b001, 0b110],
        _ => return None,
    })
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
        "角色与任务：你是精通网页 UI 自动化和计算机视觉的专家，需要在 X 网页截图中定位指定推文底部的回复、转发、点赞图标。\n\
        当前原图宽 {width}，高 {height}。所有坐标及 y 轴边界值必须输出为 0..1000 的归一化整数，x、y 分别按整张原图宽度和高度独立归一化；禁止输出像素坐标或缩略图坐标。\n\
        \n\
        【先识别 SVG 回复图标，再确认互动栏】\n\
        第一步只根据图形识别 X 推文互动栏中的回复 SVG：它是淡灰色、空心的聊天气泡轮廓，内部无填充，带有小尾部；不要依赖旁边是否出现“回复”文字。左侧导航栏的聊天图标即使外形相似，也不属于目标。\n\
        第二步必须确认该 SVG 属于推文互动栏，并同时满足：\n\
        1. 回复、转发、点赞三个图标在同一水平线上，Y 坐标彼此相差不超过 8 个归一化单位。\n\
        2. 空心气泡右侧紧挨着一个数字（通常 1~3 位）。\n\
        3. 再往右依次是转发箭头、心形图标。\n\
        4. 这三个图标下方不能是大块黑色卡片内部；黑色卡片、图片和视频属于推文媒体，不是互动栏。\n\
        \n\
        【禁止行为】\n\
        - 禁止把黑色卡片的上边缘、下边缘或卡片内部图标当成互动栏。\n\
        - 禁止把左侧导航栏的空白椭圆区域当成互动栏。左侧“更多”下方、黑色“发帖”按钮上方可能出现纯黑色空白椭圆（大约 x=230、y=840，位置仅供视觉参考）；它没有文字和图标，严禁返回其坐标。真正的发帖按钮必须能看见白色“发帖”文字。\n\
        - 禁止只凭推文文字结束位置或媒体边缘估算互动栏。必须逐个确认上述图标组。\n\
        - 如果找不到同时满足上述视觉特征的完整图标组，必须返回 ok=false，不要猜测坐标。\n\
        - 画面可能同时出现多条帖子及多个回复气泡。必须先选定目标帖，再找紧接在该帖正文/媒体下方的操作栏；禁止使用其他帖子的操作栏。\n\
        \n\
        【帖子识别与输出】只选择最上方完整可见的一条 X 帖子，准确识别作者、发布时间和正文。post_text_bottom_y 是正文文字底边的归一化 y；post_bottom_y 是整帖文字及媒体最底边的归一化 y。确认图标组属于该帖后，严格返回 JSON，字段为 ok、author_name、account_handle、posted_at、post_text、post_text_bottom_y、post_bottom_y、reply_point、repost_point、like_point；三个 point 均为 [x,y] 归一化整数。"
    )
}

fn validate_or_refind_action_targets(
    options: &VisionReplyOptions,
    shot: &Shot,
    decision: &VisionDecision,
    cancel: &AtomicBool,
) -> anyhow::Result<Point> {
    match validate_action_targets(decision, shot.width, shot.height) {
        Ok(point) => return Ok(point),
        Err(first_error) => {
            if options.stage2_ocr {
                anyhow::bail!("ppocr-rs 本地识别到的帖子互动栏未通过安全校验：{first_error:#}");
            }
            automation::log_event(&format!(
                "阶段 1/3：首轮操作栏校验失败，尝试在目标帖底边下方局部重找：{first_error:#}"
            ));
        }
    }

    check_cancel(cancel)?;
    let post_bottom_y = decision
        .post_bottom_y
        .ok_or_else(|| anyhow::anyhow!("AI 未返回帖子底边，无法进行操作栏局部重找"))?;
    anyhow::ensure!(
        post_bottom_y >= 0 && (post_bottom_y as u32) < shot.height,
        "帖子底边坐标超出截图，无法进行操作栏局部重找"
    );
    let crop_top = post_bottom_y.saturating_sub(40).max(0) as u32;
    // 只保留帖子底边附近，避免把后续帖子的操作栏也交给模型作为候选。
    let crop_height = (shot.height - crop_top).min(100);
    anyhow::ensure!(crop_height > 0, "帖子底边以下没有可供重识别的截图区域");

    let image = image::open(&shot.png)?.to_rgba8();
    let crop = image::imageops::crop_imm(&image, 0, crop_top, shot.width, crop_height).to_image();
    let crop_path = shot.png.with_file_name(format!(
        "{}-action-bar-refind.png",
        shot.png
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("screenshot")
    ));
    crop.save(&crop_path)?;
    let crop_shot = Shot {
        png: crop_path,
        width: shot.width,
        height: crop_height,
        stage: format!("{}-action-bar-refind", shot.stage),
    };
    let prompt = format!(
        "这是一张从完整截图 y={} 开始裁出的局部图，高度仅覆盖目标帖子底边附近；目标帖子底边约在局部 y=0..40，操作栏应紧接其下。只在本图中定位该操作栏，不要选择其他帖子或更下方的图标组。目标作者={:?}，账号={:?}，正文开头={:?}。回复必须是操作栏最左侧的空心聊天气泡，右侧紧邻回复数，之后依次是转发和点赞图标。请返回 ok、reason、reply_point、repost_point、like_point；点坐标使用本张裁剪图的 0..1000 归一化 [x,y]。找不到完整图标组就返回 ok=false，不要猜测。截图尺寸 {}x{}。",
        crop_top,
        decision.author_name,
        decision.account_handle,
        truncate(&decision.post_text, 120),
        crop_shot.width,
        crop_shot.height
    );
    let (refound, _) = ask(options, &crop_shot, &prompt, cancel)?;
    require_ok(&refound, "局部截图中未找到目标帖操作栏")?;

    let mut adjusted = decision.clone();
    let translate_y = |point: &mut Option<Point>, label: &str| -> anyhow::Result<()> {
        let point = point
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("局部重识别未返回 {label} 坐标"))?;
        point.y = point
            .y
            .checked_add(crop_top as i32)
            .ok_or_else(|| anyhow::anyhow!("{label} y 坐标换算溢出"))?;
        Ok(())
    };
    adjusted.reply_point = refound.reply_point;
    adjusted.repost_point = refound.repost_point;
    adjusted.like_point = refound.like_point;
    translate_y(&mut adjusted.reply_point, "回复按钮")?;
    translate_y(&mut adjusted.repost_point, "转发按钮")?;
    translate_y(&mut adjusted.like_point, "点赞按钮")?;

    automation::log_event(&format!(
        "阶段 1/3：局部重识别坐标已换算回原截图，crop_top={}，reply={:?}, repost={:?}, like={:?}。",
        crop_top, adjusted.reply_point, adjusted.repost_point, adjusted.like_point
    ));
    validate_action_targets(&adjusted, shot.width, shot.height)
        .map_err(|error| anyhow::anyhow!("局部重识别结果仍未通过校验：{error:#}"))
}

fn validate_action_targets(
    decision: &VisionDecision,
    width: u32,
    height: u32,
) -> anyhow::Result<Point> {
    require_ok(decision, "AI 无法确认完整帖子及其回复操作栏")
        .map_err(|error| anyhow::anyhow!("帖子与回复按钮关联校验失败：{error:#}"))?;
    let reply = require_named_point(decision.reply_point.as_ref(), width, height, "回复按钮")?;
    let repost = require_named_point(decision.repost_point.as_ref(), width, height, "转发按钮")?;
    let like = require_named_point(decision.like_point.as_ref(), width, height, "点赞按钮")?;

    let _post_text_bottom_y = decision.post_text_bottom_y.ok_or_else(|| {
        anyhow::anyhow!("AI 未返回所选帖子正文底部坐标，无法确认帖子内容；已停止")
    })?;
    let post_bottom_y = decision.post_bottom_y.ok_or_else(|| {
        anyhow::anyhow!("AI 未返回整条帖子（含媒体）底部坐标，不能确认操作栏位置；已停止")
    })?;
    anyhow::ensure!(
        post_bottom_y >= 0 && (post_bottom_y as u32) < height,
        "整条帖子底部 y={} 超出截图范围，帖子或媒体可能未完整显示；已停止",
        post_bottom_y
    );
    const POST_BOTTOM_Y_TOLERANCE: i32 = 40;
    anyhow::ensure!(
        reply.y >= post_bottom_y.saturating_sub(POST_BOTTOM_Y_TOLERANCE),
        "回复按钮 y={} 明显高于所选帖子底部 y={}（容差 {}px），疑似属于其他帖子；已停止",
        reply.y,
        post_bottom_y,
        POST_BOTTOM_Y_TOLERANCE
    );

    anyhow::ensure!(
        reply.x < repost.x && repost.x < like.x,
        "回复、转发、点赞坐标的水平顺序异常，已停止"
    );
    anyhow::ensure!(
        (reply.y - repost.y).abs() <= 8 && (reply.y - like.y).abs() <= 8,
        "回复、转发、点赞坐标的 Y 轴相差超过 8 像素，不在同一水平操作栏；已停止"
    );
    automation::log_event(&format!(
        "阶段 1/3：AI 操作栏坐标通过结构校验（同一行容差 8px、帖子底边关联容差 40px），整帖底部 y={}，reply=({}, {})，repost=({}, {})，like=({}, {})。",
        post_bottom_y, reply.x, reply.y, repost.x, repost.y, like.x, like.y
    ));
    Ok(reply)
}

fn require_ok(decision: &VisionDecision, fallback: &str) -> anyhow::Result<()> {
    anyhow::ensure!(decision.ok, "{}", fallback_reason(decision, fallback));
    Ok(())
}

fn validate_composer_input_point(
    point: &Point,
    width: u32,
    height: u32,
    located_by_ocr: bool,
) -> anyhow::Result<()> {
    let max_y_ratio = if located_by_ocr { 0.60 } else { 0.52 };
    anyhow::ensure!(
        point.x >= (width as f32 * 0.20) as i32
            && point.x <= (width as f32 * 0.80) as i32
            && point.y >= (height as f32 * 0.20) as i32
            && point.y <= (height as f32 * max_y_ratio) as i32,
        "输入点 ({}, {}) 未落在允许的回复文字区域（定位方式={}，y 上限={}），可能点到工具栏或提交按钮，已停止",
        point.x,
        point.y,
        if located_by_ocr {
            "OCR 占位文字"
        } else {
            "视觉 AI"
        },
        (height as f32 * max_y_ratio) as i32
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

fn generate_compliant_reply(
    options: &VisionReplyOptions,
    initial_prompt: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    const MAX_ATTEMPTS: usize = 3;
    let mut prompt = initial_prompt.to_string();
    let mut last_error = None;
    for attempt in 1..=MAX_ATTEMPTS {
        let started = std::time::Instant::now();
        let generated = text_ai::send_openai_text_cancellable(
            &options.api_url,
            &options.model,
            "你是一名长期活跃在 X 的中文普通网友，熟悉币圈、科技、互联网热点和日常生活。不是客服、营销号或内容总结机器人，只在帖子下面自然接话。",
            &prompt,
            Duration::from_secs(120),
            cancel,
        )?;
        automation::log_event(&format!(
            "回复生成请求 {attempt}/{MAX_ATTEMPTS} 完成，耗时 {} ms。",
            started.elapsed().as_millis()
        ));
        match sanitize_reply(&generated, &options.reply_rules) {
            Ok(reply) => return Ok(reply),
            Err(error) => {
                automation::log_event(&format!(
                    "回复生成请求 {attempt}/{MAX_ATTEMPTS} 未通过规则校验：{error:#}"
                ));
                let limit = reply_char_limit(&options.reply_rules).unwrap_or(280);
                let previous_reply = generated.trim();
                last_error = Some(error);
                prompt = format!(
                    "上一版回复不符合规则，必须重写。硬性要求：回复最多 {limit} 个字符，不能超限；发送前逐字计数，不合规就继续缩短。违规原因：{}。上一版回复：{previous_reply}。结合原帖重新生成一条满足回复规则的内容，只返回正文，不要解释。\n原始任务：{}",
                    last_error.as_ref().unwrap(),
                    initial_prompt
                );
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("AI 回复未通过规则校验")))
}

fn sanitize_reply(text: &str, rules: &str) -> anyhow::Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    anyhow::ensure!(!text.is_empty(), "AI 生成的回复为空");
    let char_count = text.chars().count();
    let limit = reply_char_limit(rules).unwrap_or(280);
    anyhow::ensure!(
        char_count <= limit,
        "AI 生成的回复有 {char_count} 字，超过回复规则要求的 {limit} 字；已拦截，未输入或发送"
    );
    if rules.contains("不能有表情") || rules.contains("不要有表情") {
        anyhow::ensure!(
            !text.chars().any(is_emoji),
            "AI 生成的回复包含表情符号，不符合回复规则；已拦截，未输入或发送"
        );
    }
    if rules.contains("不能有感叹号") || rules.contains("不要有感叹号") {
        anyhow::ensure!(
            !text.contains('!') && !text.contains('！'),
            "AI 生成的回复包含感叹号，不符合回复规则；已拦截，未输入或发送"
        );
    }
    if rules.contains("不能有问号") || rules.contains("不要有问号") {
        anyhow::ensure!(
            !text.contains('?') && !text.contains('？'),
            "AI 生成的回复包含问号，不符合回复规则；已拦截，未输入或发送"
        );
    }
    Ok(text)
}

fn reply_char_limit(rules: &str) -> Option<usize> {
    // 强制硬性限制：如果要求不超过 12 个汉字或未在规则中指定，统一最高限制为 12 个字符
    let parsed_limit = (|| {
        let chars: Vec<char> = rules.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            if !chars[index].is_ascii_digit() {
                index += 1;
                continue;
            }
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            if chars.get(index..index + 2) == Some(&['个', '字']) {
                if let Ok(limit) = chars[start..index].iter().collect::<String>().parse() {
                    return Some(limit);
                }
            }
        }
        None
    })();
    Some(parsed_limit.map_or(12, |l: usize| l.min(12)))
}

fn is_emoji(ch: char) -> bool {
    matches!(ch as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE00..=0xFE0F)
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
        Point, parse_decision, sanitize_reply, validate_action_targets,
        validate_composer_input_point,
    };

    #[test]
    fn manual_move_button_accepts_reply_point_below_visible_media() {
        let decision = parse_decision(
            r#"{"ok":true,"post_text_bottom_y":900,"post_bottom_y":980,"reply_point":[668,1005],"repost_point":[787,1005],"like_point":[902,1005]}"#,
        )
        .unwrap();

        let reply = validate_action_targets(&decision, 1920, 1032).unwrap();
        assert_eq!((reply.x, reply.y), (668, 1005));
    }

    #[test]
    fn rejects_reply_button_above_selected_post_bottom() {
        let decision = parse_decision(
            r#"{"ok":true,"post_text_bottom_y":480,"post_bottom_y":513,"reply_point":[599,450],"repost_point":[700,450],"like_point":[800,450]}"#,
        )
        .unwrap();

        assert!(validate_action_targets(&decision, 1920, 1032).is_err());
    }

    #[test]
    fn accepts_reply_point_within_tolerant_post_bottom_margin() {
        let decision = parse_decision(
            r#"{"ok":true,"post_text_bottom_y":480,"post_bottom_y":938,"reply_point":[668,922],"repost_point":[787,922],"like_point":[902,922]}"#,
        )
        .unwrap();

        let reply = validate_action_targets(&decision, 1920, 1032).unwrap();
        assert_eq!((reply.x, reply.y), (668, 922));
    }

    #[test]
    fn rejects_action_icons_more_than_eight_pixels_apart_vertically() {
        let decision = parse_decision(
            r#"{"ok":true,"post_text_bottom_y":480,"post_bottom_y":513,"reply_point":[668,520],"repost_point":[787,529],"like_point":[902,520]}"#,
        )
        .unwrap();

        assert!(validate_action_targets(&decision, 1920, 1032).is_err());
    }

    #[test]
    fn fully_automatic_after_scroll_rejects_reply_point_inside_video() {
        let decision = parse_decision(
            r#"{"ok":true,"post_text_bottom_y":490,"post_bottom_y":960,"reply_point":[672,691],"repost_point":[787,691],"like_point":[902,691]}"#,
        )
        .unwrap();

        assert!(validate_action_targets(&decision, 1920, 1032).is_err());
    }

    #[test]
    fn rejects_composer_points_on_the_bottom_toolbar() {
        let placeholder = Point { x: 768, y: 433 };
        let expanded_composer_placeholder = Point { x: 786, y: 561 };
        let toolbar = Point { x: 768, y: 685 };
        assert!(validate_composer_input_point(&placeholder, 1920, 1032, false).is_ok());
        assert!(
            validate_composer_input_point(&expanded_composer_placeholder, 1920, 1032, true).is_ok()
        );
        assert!(
            validate_composer_input_point(&expanded_composer_placeholder, 1920, 1032, false)
                .is_err()
        );
        assert!(validate_composer_input_point(&toolbar, 1920, 1032, true).is_err());
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
        let rules = "10个字内，回答要有深度，不能有表情符号，感叹号，问号。";
        assert_eq!(sanitize_reply("  你好世界  ", rules).unwrap(), "你好世界");
        assert!(sanitize_reply("这是超过十个字的回复内容", rules).is_err());
        assert!(sanitize_reply("你好！", rules).is_err());
        assert!(sanitize_reply("   ", rules).is_err());
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

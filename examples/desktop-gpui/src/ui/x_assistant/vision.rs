use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Deserializer};

use super::automation::{self, native};
use super::ocr;
use super::ocr_status::recognize_send_status;
use super::reply_filter;

#[path = "text_ai.rs"]
pub(super) mod text_ai;

pub const DEFAULT_OLLAMA_URL: &str = "http://192.168.31.60:11434/v1";
pub const DEFAULT_VISION_MODEL: &str = "qwen3.5:4b";

// 连续滚动达到阈值后刷新时间线，避免反复处理旧帖子。
const SCROLLS_BEFORE_REFRESH: u64 = 8;
static FEED_SCROLL_COUNT: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug)]
pub(super) struct SendOutcomeUncertain;

impl std::fmt::Display for SendOutcomeUncertain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("提交后的发送状态无法可靠确认，已停止自动回复以避免重复发送")
    }
}

impl std::error::Error for SendOutcomeUncertain {}

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
    // 视觉模型必须区分原帖下方已发布回复与编辑框中的草稿。
    #[serde(default)]
    reply_post_state: Option<String>,
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

fn refresh_timeline(cancel: &AtomicBool) -> anyhow::Result<()> {
    check_cancel(cancel)?;
    automation::native::activate_browser()?;
    automation::native::press_f5()?;
    wait_cancel(cancel, Duration::from_secs(3))?;

    let dir = PathBuf::from("data")
        .join("x_assistant")
        .join("screenshots");
    fs::create_dir_all(&dir)?;
    let id = app_log::beijing_now().format("%Y%m%d-%H%M%S-%3f");
    let mut screenshots = Vec::new();
    let check_shot = capture_desktop(&dir, &id, "feed-refresh-check", &mut screenshots)?;
    check_cancel(cancel)?;
    anyhow::ensure!(
        check_shot.width >= 800 && check_shot.height >= 600,
        "刷新确认检查截图尺寸异常（{}x{}），停止本轮以免漏处理确认弹窗",
        check_shot.width,
        check_shot.height
    );

    match ocr::recognize_reload_confirmation(&check_shot.png) {
        Ok(true) => {
            automation::log_event("时间线刷新：识别到重新加载确认提示，按 Enter 确认。");
            automation::native::press_enter()?;
            wait_cancel(cancel, Duration::from_secs(5))?;
        }
        Ok(false) => {
            automation::log_event("时间线刷新：未发现重新加载确认提示。");
            wait_cancel(cancel, Duration::from_secs(2))?;
        }
        Err(error) => anyhow::bail!("无法识别刷新确认提示，停止本轮：{error:#}"),
    }

    check_cancel(cancel)
}

pub(super) fn recover_by_error(
    cancel: &AtomicBool,
    error: &anyhow::Error,
) -> anyhow::Result<()> {
    let message = format!("{error:#}");

    if message.contains("账号二次 OCR") || message.contains("账号不一致") {
        automation::log_event(
            "分类恢复：账号二次核验存在歧义，跳过当前帖子并滚动；不执行全桌面 OCR。",
        );
        return scroll_down_in_small_steps(cancel, "账号识别存在歧义，安全跳过当前帖子");
    }

    if message.contains("截图尺寸异常")
        || message.contains("浏览器客户区尺寸异常")
        || message.contains("截图尺寸与浏览器客户区不一致")
    {
        automation::log_event(
            "分类恢复：浏览器窗口尺寸异常，重新激活并检查客户区；不对异常截图执行 OCR。",
        );
        native::activate_browser()?;
        wait_cancel(cancel, Duration::from_millis(700))?;
        let (width, height) = native::get_window_client_size()?;
        anyhow::ensure!(
            width >= 800 && height >= 600,
            "浏览器恢复后客户区仍异常（{}x{}），停止自动操作",
            width,
            height
        );
        automation::log_event(&format!(
            "分类恢复：浏览器客户区已恢复为 {}x{}，下一轮重新截图。",
            width, height
        ));
        return Ok(());
    }

    if message.contains("局部 OCR")
        || message.contains("互动栏")
        || message.contains("回复坐标")
        || message.contains("PP-OCR")
    {
        automation::log_event(
            "分类恢复：帖子或互动栏 OCR 失败，跳过当前帖子并滚动；不执行昂贵的全桌面状态 OCR。",
        );
        return scroll_down_in_small_steps(cancel, "帖子或互动栏 OCR 失败，安全跳过当前帖子");
    }

    randomized_recovery(cancel, "本轮报错，按通用流程恢复")
}

pub(super) fn randomized_recovery(cancel: &AtomicBool, reason: &str) -> anyhow::Result<()> {
    check_cancel(cancel)?;
    if reason.contains("报错") {
        automation::native::activate_browser()?;
        let dir = PathBuf::from("data")
            .join("x_assistant")
            .join("screenshots");
        fs::create_dir_all(&dir)?;
        let id = app_log::beijing_now().format("%Y%m%d-%H%M%S-%3f");
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
        let id = app_log::beijing_now().format("%Y%m%d-%H%M%S-%3f");
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
    // 每轮随机滚动 5～12 格，每段随机滚动 2～5 格。
    scroll_down_random_steps(cancel, reason, 5, 12)?;

    let count = FEED_SCROLL_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
    automation::log_event(&format!(
        "时间线滚动计数：{count}/{SCROLLS_BEFORE_REFRESH}"
    ));

    if count >= SCROLLS_BEFORE_REFRESH {
        check_cancel(cancel)?;
        automation::log_event("时间线连续滚动达到阈值，刷新页面以获取较新的帖子。");
        refresh_timeline(cancel)?;
        FEED_SCROLL_COUNT.store(0, Ordering::Release);
        automation::log_event("时间线刷新完成，滚动计数已归零。");
    }

    Ok(())
}

pub(super) fn scroll_down_to_complete_post(
    cancel: &AtomicBool,
    reason: &str,
) -> anyhow::Result<()> {
    // 补齐帖子时同样随机滚动 5～12 格，每段 2～5 格。
    scroll_down_random_steps(cancel, reason, 5, 12)
}

fn activate_browser_for_scroll(reason: &str) -> anyhow::Result<()> {
    let started = Instant::now();
    automation::log_event(&format!("滚动前：开始激活浏览器，原因={reason}。"));

    match automation::native::activate_browser() {
        Ok(()) => {
            automation::log_event(&format!(
                "滚动前：浏览器激活成功，耗时 {:.2} 秒。",
                started.elapsed().as_secs_f64()
            ));
            Ok(())
        }
        Err(error) => {
            automation::log_event(&format!(
                "滚动前：浏览器激活失败，耗时 {:.2} 秒：{error:#}",
                started.elapsed().as_secs_f64()
            ));
            Err(error.context("滚动前激活浏览器失败"))
        }
    }
}

fn scroll_down_random_steps(
    cancel: &AtomicBool,
    reason: &str,
    min_steps: u32,
    max_steps: u32,
) -> anyhow::Result<()> {
    check_cancel(cancel)?;
    activate_browser_for_scroll(reason)?;
    let range = max_steps.saturating_sub(min_steps).saturating_add(1);
    let total = min_steps + (next_random_u64() % u64::from(range)) as u32;
    automation::log_event(&format!(
        "全自动准备：{reason}；本轮随机滚动 {total} 格，分段执行。"
    ));

    let mut moved = 0;
    while moved < total {
        check_cancel(cancel)?;
        let remaining = total - moved;
        let batch = (2 + (next_random_u64() % 4) as u32).min(remaining);
        automation::native::scroll_down_by_notches(batch as i32)?;
        moved += batch;

        // 每段等待时间不同，偶尔停顿更久；取消时最多约 100ms 响应。
        let delay_ms = 160 + (next_random_u64() % 541);
        wait_cancel(cancel, Duration::from_millis(delay_ms))?;
        if moved < total && next_random_u64() % 4 == 0 {
            let pause_ms = 350 + (next_random_u64() % 651);
            wait_cancel(cancel, Duration::from_millis(pause_ms))?;
        }
    }

    automation::log_event(&format!(
        "全自动准备：随机滚动完成，共 {total} 格，页面稳定后识别。"
    ));
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
        // 自动回复必须从 X 首页开始；open_chrome() 是自动发帖专用入口，会打开 compose/post。
        super::option::open_chrome_home()?;
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
    let id = app_log::beijing_now().format("%Y%m%d-%H%M%S-%3f");
    let mut screenshots = Vec::new();
    let run_id = id.to_string();

    // 浏览器窗口存在不代表当前标签页就是 X 首页。
    // 每轮回复开始前先 OCR 当前地址；非 X 首页时主动导航到首页，并再次验证。
    let initial_route = capture_desktop(
        &dir,
        &id,
        "reply-startup-route-check",
        &mut screenshots,
    )?;
    let mut current_url = ocr::recognize_current_url(&initial_route.png)?;
    if !current_url.as_deref().is_some_and(ocr::is_x_home_address) {
        automation::log_event(&format!(
            "自动回复启动：当前标签页不是已确认的 X 首页（地址：{current_url:?}），切换当前浏览器标签页到 https://x.com/home。"
        ));
        native::navigate_to_x_home()?;
        let mut home_confirmed = false;
        for attempt in 1..=12 {
            wait_cancel(cancel, Duration::from_secs(1))?;
            let route_check = capture_desktop(
                &dir,
                &id,
                &format!("reply-home-route-check-{attempt:02}"),
                &mut screenshots,
            )?;
            current_url = ocr::recognize_current_url(&route_check.png)?;
            if current_url.as_deref().is_some_and(ocr::is_x_home_address) {
                automation::log_event(&format!(
                    "自动回复启动：第 {attempt}/12 次地址栏 OCR 已确认 X 首页：{:?}",
                    current_url
                ));
                home_confirmed = true;
                break;
            }
        }
        anyhow::ensure!(
            home_confirmed,
            "自动回复启动失败：已尝试导航，但连续 12 次 OCR 仍无法确认当前页面为 x.com/home；为避免误操作已停止"
        );
    } else {
        automation::log_event(&format!(
            "自动回复启动：地址栏 OCR 已确认当前页面为 X 首页：{:?}",
            current_url
        ));
    }

    let model = resolve_model(&options)?;
    let options = VisionReplyOptions { model, ..options };
    automation::log_event(&format!(
        "视觉模型：{}，接口：{}",
        options.model, options.api_url
    ));

    *current_stage = "阶段 1/3（识别帖子并定位回复按钮）".to_string();
    automation::log_event("========== 阶段 1/3：定位并点击帖子回复图标 ==========");
    // 直接获取浏览器客户区尺寸，不再为了获取宽高额外采集截图。
    let (window_width, window_height) = native::get_window_client_size()?;

    // 保持原有鼠标避让逻辑，避免鼠标遮挡帖子文字和互动按钮。
    move_cursor_to_safe_blank_area(window_width, window_height)?;

    // 鼠标移动完成后，采集正式用于帖子识别的截图。
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
        match identify_post_for_mode(&options, &feed, cancel) {
            Ok(post) => post,
            Err(error) => {
                // 局部互动栏坐标失败与帖子删除无关，不要再对同一张截图执行一次完整 OCR。
                let error_detail = format!("{error:#}");
                automation::log_event(&format!(
                    "阶段 1/3：首次帖子/互动栏识别失败，原始错误：{error_detail}"
                ));
                let action_coordinate_failed =
                    error_detail.contains("局部 OCR 两轮均未识别到回复坐标");

                if cancel.load(Ordering::Acquire) {
                    automation::log_event(
                        "阶段 1/3：首次识别失败后发现取消信号；返回首次识别错误，不再执行删除状态复核。",
                    );
                    return Err(error);
                }
                if !action_coordinate_failed {
                    let unavailable_check_start = Instant::now();
                    automation::log_event("阶段 1/3：开始复核帖子是否已删除或不可用。");
                    let unavailable = ocr::recognize_post_unavailable_error(&feed.png)
                        .unwrap_or(false);
                    automation::log_event(&format!(
                        "阶段 1/3：帖子删除状态复核结束，结果={}，耗时={} ms。",
                        if unavailable { "已删除或不可用" } else { "未发现删除提示" },
                        unavailable_check_start.elapsed().as_millis()
                    ));
                    if cancel.load(Ordering::Acquire) {
                        automation::log_event(
                            "阶段 1/3：删除状态复核期间收到取消信号；返回首次识别错误，避免取消信号覆盖根因。",
                        );
                        return Err(error);
                    }
                    if unavailable {
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
                } else {
                    automation::log_event(
                        "阶段 1/3：回复坐标识别失败，跳过重复的整页删除状态 OCR，直接返回错误。",
                    );
                }

                if is_incomplete_post_error(&error) {
                    automation::log_event(&format!(
                        "阶段 1/3：当前截图未能识别完整帖子（{error:#}）；不再重复截图和 OCR，安全跳过。"
                    ));
                    return skip_post_and_scroll(
                        screenshots,
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        cancel,
                        false,
                        &format!("当前截图未能识别完整帖子：{error:#}"),
                    );
                }

                return Err(error);
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

    let mut tooltip_verified = false;
    let mut verified_reply_y = reply_button.y;
    for (attempt, offset_y) in [0, 5, 10].into_iter().enumerate() {
        let hover_y = reply_button.y + offset_y;
        automation::log_event(&format!(
            "阶段 1/3：第 {} 次悬停回复坐标 ({}, {})，Y 偏移={}px。",
            attempt + 1,
            reply_button.x,
            hover_y,
            offset_y
        ));
        automation::log_event(&format!(
            "阶段 1/3：悬停前取消状态={}。",
            cancel.load(Ordering::Acquire)
        ));
        check_cancel(cancel)?;
        native::move_cursor_to_reply(reply_button.x, hover_y)?;
        automation::log_event(&format!(
            "阶段 1/3：鼠标移动完成，取消状态={}；开始等待悬停提示。",
            cancel.load(Ordering::Acquire)
        ));
        wait_cancel(
            cancel,
            Duration::from_millis(if attempt == 0 { 900 } else { 700 }),
        )
        .map_err(|error| error.context("回复按钮悬停等待期间收到取消信号或等待失败"))?;
        automation::log_event("阶段 1/3：悬停等待完成，准备采集提示截图。");
        let label = format!("01-reply-tooltip-hover-{}", attempt + 1);
        let tooltip_shot = capture(&dir, &id, &label, &mut screenshots)?;
        tooltip_verified =
            match ocr::verify_reply_tooltip(&tooltip_shot.png, reply_button.x, hover_y) {
                Ok(verified) => verified,
                Err(error) => {
                    automation::log_event(&format!(
                        "第 {} 次悬停回复提示 OCR 失败：{error:#}",
                        attempt + 1
                    ));
                    false
                }
            };
        if tooltip_verified {
            verified_reply_y = hover_y;
            automation::log_event(&format!(
                "阶段 1/3：第 {} 次悬停成功识别‘回复’提示，后续点击复用坐标 ({}, {})。",
                attempt + 1,
                reply_button.x,
                verified_reply_y
            ));
            break;
        }
    }
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
    // 外部任务（例如自动发帖）不得中断本轮回复；但用户停止或页面销毁仍必须在点击前生效。
    check_cancel(cancel)?;
    native::move_cursor_to_reply(reply_button.x, verified_reply_y)?;
    wait_cancel(cancel, Duration::from_millis(500))?;
    check_cancel(cancel)?;
    native::click_at_client_position(reply_button.x, verified_reply_y)?;
    automation::log_event(&format!(
        "阶段 1/3：回复点点击事件已发送，使用悬停验证坐标=({}, {})（原始识别坐标 y={}）。",
        reply_button.x, verified_reply_y, reply_button.y
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
        let mut current_url = compose_page["current_url"].as_str().map(str::to_owned);

        // 局部 OCR 能识别出有效的 X 页面地址时，使用通用地址栏识别作为备用。
        // 仍然要求识别到明确的 X 路由，不因地址栏为空而放行。
        let has_valid_x_route = current_url
            .as_deref()
            .is_some_and(|url| ocr::is_x_compose_post_address(url) || ocr::is_x_home_address(url));

        if !has_valid_x_route {
            automation::log_event(
                "阶段 2/3：局部 OCR 未识别到有效 X 地址，尝试通用地址栏 OCR 备用识别。",
            );

            current_url = ocr::recognize_current_url(&composer.png)?;

            automation::log_event(&format!(
                "阶段 2/3：通用地址栏 OCR 备用识别结果：{current_url:?}。",
            ));
        }

        let is_compose_url = current_url
            .as_deref()
            .is_some_and(ocr::is_x_compose_post_address);

        let is_home_url = current_url.as_deref().is_some_and(ocr::is_x_home_address);

        let expected_account = account_handle.trim().trim_start_matches('@');

        let actual_account = compose_page["reply_target_account"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_start_matches('@');

        let reply_input_detected = compose_page["reply_input"].is_object();

        let display_name_matched = identity_text_matches(
            &author_name,
            compose_page["dialog_text"].as_str().unwrap_or(""),
        );
        let account_matched = reply_target_account_matches(
            expected_account,
            actual_account,
            display_name_matched,
        );

        // compose/post 可以继续；
        // home 必须同时确认回复输入框和回复对象账号。
        let composer_confirmed =
            is_compose_url || (is_home_url && reply_input_detected && account_matched);

        anyhow::ensure!(
            composer_confirmed,
            "回复弹窗状态无法确认：地址={:?}，输入框识别={}，回复对象匹配={}，停止发送",
            current_url,
            reply_input_detected,
            account_matched
        );
        let expected_account = account_handle.trim().trim_start_matches('@');

        let actual_account = compose_page["reply_target_account"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_start_matches('@');

        anyhow::ensure!(
            reply_target_account_matches(
                expected_account,
                actual_account,
                display_name_matched,
            ),
            "回复对象不匹配：预期 @{}，实际 @{}，昵称匹配={}，禁止发送",
            expected_account,
            actual_account,
            display_name_matched
        );

        automation::log_event(&format!(
            "阶段 2/3：回复对象验证通过，预期 @{}，实际 @{}，昵称匹配={}",
            expected_account, actual_account, display_name_matched
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
            "严格遵守下面列出的每一条回复规则；规则之间不得取舍。生成一条不换行、使用原帖主要语言的回复，只返回回复正文，不要解释或加引号。输出前逐条核对，任一规则不满足就重新生成。\n帖子正文：{post_text}\n回复规则：{}",
            options.reply_rules.trim()
        );
        let reply_text = reply_filter::generate_compliant_reply(&options, &prompt, &post_text, cancel)?;
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

        let reply_text = match reply_filter::sanitize_reply(&composer_decision.reply_text, &options.reply_rules, &post_text) {
            Ok(reply) => reply,
            Err(error) => {
                automation::log_event(&format!(
                    "视觉模型生成的回复不符合规则，转文本模型重写：{error:#}"
                ));
                let prompt = format!(
                    "请严格遵守每一条规则，重写一条不换行、使用原帖主要语言的回复。上一版未通过校验：{error:#}。只返回回复正文，不要解释或加引号。\n帖子正文：{post_text}\n回复规则：{}",
                    options.reply_rules.trim()
                );
                reply_filter::generate_compliant_reply(&options, &prompt, &post_text, cancel)?
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
                "阶段 2/3：本地 OCR 已核对回复正文及弹窗右下方提交按钮：({}, {})。",
                x, y
            ));
            Point { x, y }
        }
        Ok(None) => anyhow::bail!(
            "本地 OCR 未能在已核验回复弹窗右下方定位提交按钮；禁止使用 AI 猜测坐标，未点击发送"
        ),
        Err(error) => anyhow::bail!(
            "提交按钮局部 OCR 校验失败：{error:#}；禁止使用 AI 坐标兜底，未点击发送"
        ),
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
    let refreshed_button = ocr::locate_reply_submit_button(
        &pre_submit.png,
        &reply_text,
        pre_submit.width,
        pre_submit.height,
    )?
    .ok_or_else(|| anyhow::anyhow!(
        "点击前二次 OCR 未能在回复弹窗右下方重新确认提交按钮；已停止，未点击发送"
    ))?;
    anyhow::ensure!(
        send_button.x.abs_diff(refreshed_button.0) <= 12
            && send_button.y.abs_diff(refreshed_button.1) <= 12,
        "提交按钮位置发生变化：首次=({}, {})，二次=({}, {})；禁止点击过期坐标",
        send_button.x, send_button.y, refreshed_button.0, refreshed_button.1
    );
    automation::log_event(&format!(
        "阶段 3/3：点击前二次 OCR 坐标复核通过，按钮位置=({}, {})。",
        refreshed_button.0, refreshed_button.1
    ));
    native::move_cursor_to_reply(send_button.x, send_button.y)?;
    *current_stage = "阶段 3/3（点击发送并确认结果）".to_string();
    automation::log_event("========== 阶段 3/3：点击‘回复’并判断回复是否完成 ==========");
    // 点击前先持久化提交意图；数据库失败时绝不触发真实发送。
    binance_tools::db::x_assistant::mark_reply_submitting_blocking(&run_id)?;
    native::click_at_client_position(send_button.x, send_button.y)
        .map_err(|error| uncertain_send_error(&run_id, format!("首次提交点击返回错误，无法确认鼠标事件是否已生效：{error:#}")))?;
    automation::log_event("阶段 3/3：已点击‘回复’，等待 2 秒后采集结果截图。");
    thread::sleep(Duration::from_secs(2));

    let mut done = capture(&dir, &id, "04-result", &mut screenshots)
        .map_err(|error| uncertain_send_error(&run_id, format!("提交后结果截图采集失败：{error:#}")))?;
    // 提交点击已经发生：即使此时收到停止信号，也必须先做只读路由与 OCR 核验。
    // 后续是否允许重试由取消状态决定；取消后绝不再点击、关闭弹窗或处理下一帖。
    // 结果截图可能仍显示回复弹窗，这是正常的；但必须确认仍处于 X 首页/回复路由。
    // 如果意外跳到 Grok 或其他页面，不再对该截图执行状态点击或恢复操作。
    ensure_reply_page_route(&done, "发送结果截图")
        .map_err(|error| uncertain_send_error(&run_id, format!("提交后页面路由核验失败：{error:#}")))?;

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
                "阶段 3/3：ppocr-rs 耗时 {} ms，OCR 识别失败：${error:#}；继续执行专用成功提示 OCR，不自动重试。",
                ocr_started.elapsed().as_millis()
            ));
            Default::default()
        }
    };

    // X 的蓝色“你的帖子已发送”提示可能位于页面底部，整页 OCR 容易漏掉。
    // 使用已存在的底部全宽、多阈值专用 OCR 再识别一次；它适用于帖子和回复。
    let sent_toast = match ocr::recognize_post_sent_toast(&done.png) {
        Ok(true) => {
            automation::log_event("阶段 3/3：底部全宽专用 OCR 明确识别到 X 的“你的帖子已发送”提示。");
            true
        }
        Ok(false) => false,
        Err(error) => {
            automation::log_event(&format!(
                "阶段 3/3：底部全宽成功提示 OCR 失败：${error:#}；继续只读视觉核验。"
            ));
            false
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

    // 发送成功：整页 OCR 或底部专用 OCR 任一明确识别成功提示即可。
    // 成功提示优先于稍后出现的网络错误 toast；提交动作已发生，不再进入重试流程。
    if status.sent || sent_toast {
        automation::log_event("阶段 3/3：OCR 明确检测到 X 发送成功提示；标记完成，不再调用视觉模型推翻成功结果，也不执行关闭弹窗操作。");

        mark_reply_completed_or_stop(&run_id)?;

        return Ok(VisionReplyResult {
            screenshots,
            author_name,
            account_handle,
            posted_at,
            post_text,
            reply_text,
            completed: true,
            skipped_duplicate: false,
            summary: "OCR 检测到 X 的发送成功提示，回复已确认发送。".to_string(),
        });
    }

    // 平台限流
    if status.rate_limited {
        automation::log_event("阶段 3/3：检测到平台限流提示；不重试、不刷新，停止全自动发送。");

        return Err(anyhow::Error::new(RateLimitDetected));
    }

    // 停止信号若在提交后到达，不能进入等待、视觉重试或任何后续点击。
    // OCR 未明确确认成功时，保守标记 uncertain，避免重复发送。
    if cancel.load(Ordering::Relaxed) {
        return Err(uncertain_send_error(
            &run_id,
            "提交按钮已点击，停止信号在结果核验期间到达，OCR 未明确确认成功；禁止重试",
        ));
    }

    // 是否存在可重试发送错误
    let retryable_error = status.retryable_error;

    if retryable_error {
        automation::log_event("阶段 3/3：截图识别到发送错误；等待 5 秒后重新截图并确认发送状态。");

        if let Err(error) = wait_cancel(cancel, Duration::from_secs(5)) {
            return Err(uncertain_send_error(
                &run_id,
                format!("首次提交后等待重试核验时收到停止信号：{error:#}"),
            ));
        }

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

            mark_reply_completed_or_stop(&run_id)?;

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

        // X 的错误 toast 可能在数秒后自动消失；toast 消失本身不代表发送成功，
        // 也不代表发送失败。无论错误提示是否仍在，都必须继续进行视觉核验：
        // 只有明确确认原帖下没有这条回复，后续才允许唯一一次重试。
        if !retry_status.retryable_error {
            automation::log_event(
                "阶段 3/3：重试前错误提示已消失且未识别成功提示；继续视觉核验原帖，禁止仅凭 toast 消失判定状态。",
            );
        } else {
            automation::log_event(
                "阶段 3/3：重试前仍识别到发送错误提示；继续视觉核验原帖后再决定是否重试。",
            );
        }

        // 视觉模型必须明确确认原帖下方没有出现该回复。编辑框里的文字不算已发布。
        let (retry_visual, _) = ask(
            &options,
            &retry_ready,
            &format!(
                "这是发送错误后的重试安全核验。待发送回复全文：{reply_text}。先检查截图中是否出现 X 的蓝色发送成功提示。如果清晰出现“你的帖子已发送”“你的贴已发送”“你的回复已发送”或英文“Your post was sent”“Your reply was sent”，且没有同时显示明确的本次发送失败提示，必须返回 ok=true、reply_post_state=\"sent_confirmation\"；X 对回复也可能使用通用的“你的帖子已发送”提示，这代表本次回复已提交成功，不要求同时看见回复气泡。只有没有成功提示时，才检查原帖下方的已发布回复区域，判断是否已经出现这段回复。绝对不能把编辑框或草稿输入区中的文字算作已发布。返回 JSON：{{\"ok\":true,\"reply_post_state\":\"not_visible\"}} 表示明确看见原帖下方没有这条已发布回复；若已出现则 reply_post_state=\"visible\"；若原帖区域被遮挡、无法判断或不确定，返回 ok=false 或 reply_post_state=\"uncertain\"。不得猜测。",
            ),
            cancel,
        )
        .map_err(|error| uncertain_send_error(&run_id, format!("重试前视觉核验失败：{error:#}")))?;

        match retry_visual.reply_post_state.as_deref() {
            Some("sent_confirmation") if retry_visual.ok => {
                automation::log_event("阶段 3/3：视觉模型确认 X 显示蓝色发送成功提示（包括通用“你的帖子已发送”）；按本次回复已发送处理，不再重试。");
                mark_reply_completed_or_stop(&run_id)?;
                return Ok(VisionReplyResult {
                    screenshots,
                    author_name,
                    account_handle,
                    posted_at,
                    post_text,
                    reply_text,
                    completed: true,
                    skipped_duplicate: false,
                    summary: "检测到 X 的发送成功提示，已确认回复发送完成，未重复提交。".to_string(),
                });
            }
            Some("visible") if retry_visual.ok => {
                automation::log_event("阶段 3/3：视觉复核明确确认回复已出现在原帖下方；不再提交。");
                return already_sent_result(
                    &dir, &id, screenshots, &run_id, author_name, account_handle,
                    posted_at, post_text, reply_text, cancel,
                );
            }
            Some("not_visible") if retry_visual.ok => {
                automation::log_event("阶段 3/3：视觉复核确认原帖下方没有该回复，继续检查重试按钮。");
            }
            _ => {
                close_reply_composer_safely(&dir, &id, &mut screenshots, cancel)
                    .map_err(|error| {
                        uncertain_send_error(
                            &run_id,
                            format!("视觉核验不确定且关闭弹窗失败：{error:#}"),
                        )
                    })?;
                return Err(uncertain_send_error(&run_id, "无法确认原帖下方是否已有该回复，禁止重试"));
            }
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
                close_reply_composer_safely(&dir, &id, &mut screenshots, cancel)
                    .map_err(|error| {
                        uncertain_send_error(
                            &run_id,
                            format!("未找到重试按钮且关闭弹窗失败：{error:#}"),
                        )
                    })?;
                return Err(uncertain_send_error(&run_id, 
                    "未能同时确认回复文本和提交按钮，已放弃重试"
                ));
            }

            Err(error) => {
                close_reply_composer_safely(&dir, &id, &mut screenshots, cancel)
                    .map_err(|close_error| {
                        uncertain_send_error(
                            &run_id,
                            format!(
                                "重新识别按钮失败（{error:#}），且关闭弹窗失败：{close_error:#}"
                            ),
                        )
                    })?;
                return Err(uncertain_send_error(&run_id, format!("重新识别提交按钮失败：{error:#}")));
            }
        };

        // 重试前坐标安全校验
        let retry_pre_submit = capture(&dir, &id, "04-retry-presubmit", &mut screenshots)?;

        if !target_region_unchanged(
            &retry_ready,
            &retry_pre_submit,
            &retry_button,
            "重试时的弹窗提交按钮",
        )? {
            close_reply_composer_safely(&dir, &id, &mut screenshots, cancel)
                .map_err(|error| {
                    uncertain_send_error(
                        &run_id,
                        format!("按钮区域变化且关闭弹窗失败：{error:#}"),
                    )
                })?;
            return Err(uncertain_send_error(&run_id, 
                "重试前回复弹窗发生变化，已停止，未使用过期坐标"
            ));
        }

        if let Err(error) = check_cancel(cancel) {
            return Err(uncertain_send_error(
                &run_id,
                format!("唯一一次重试点击前收到停止信号，未再次提交：{error:#}"),
            ));
        }
        native::move_cursor_to_reply(retry_button.x, retry_button.y)?;

        automation::log_event(&format!(
            "阶段 3/3：间隔 5 秒后进行唯一一次重试，提交坐标 ({}, {})。",
            retry_button.x, retry_button.y
        ));

        // 第二次点击前再次持久化尝试次数与北京时间提交时间。
        binance_tools::db::x_assistant::mark_reply_submitting_blocking(&run_id)?;
        native::click_at_client_position(retry_button.x, retry_button.y)
            .map_err(|error| uncertain_send_error(&run_id, format!("唯一一次重试点击返回错误，无法确认鼠标事件是否已生效：{error:#}")))?;

        thread::sleep(Duration::from_secs(2));

        done = capture(&dir, &id, "05-retry-result", &mut screenshots)
            .map_err(|error| uncertain_send_error(&run_id, format!("唯一一次重试提交后截图采集失败：{error:#}")))?;

        // 第二次提交后同样进入只读结果核验，不允许取消信号跳过 OCR。

        // ========================================================
        // 重试结果检查：一次 OCR 判断全部状态
        // ========================================================

        let status = match recognize_send_status(&done.png) {
            Ok(status) => status,
            Err(error) => {
                automation::log_event(&format!(
                    "阶段 3/3：唯一一次重试后 OCR 识别失败：{error:#}；无法确认发送结果。"
                ));

                close_reply_composer_safely(&dir, &id, &mut screenshots, cancel)
                    .map_err(|close_error| {
                        uncertain_send_error(
                            &run_id,
                            format!("重试后 OCR 失败且关闭弹窗失败：{close_error:#}"),
                        )
                    })?;
                return Err(uncertain_send_error(&run_id, format!("重试后 OCR 无法确认发送结果：{error:#}")));
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

            mark_reply_completed_or_stop(&run_id)?;

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

        if cancel.load(Ordering::Relaxed) {
            return Err(uncertain_send_error(
                &run_id,
                "唯一一次重试已点击，停止信号在结果核验期间到达，OCR 未明确确认成功",
            ));
        }

        // 重试后仍然发送错误
        if status.retryable_error {
            automation::log_event(
                "阶段 3/3：唯一一次重试后仍检测到发送错误；只有确认弹窗关闭后才继续下一条。",
            );

            close_reply_composer_safely(&dir, &id, &mut screenshots, cancel)
                .map_err(|error| {
                    uncertain_send_error(
                        &run_id,
                        format!("重试仍失败且无法确认关闭回复弹窗：{error:#}"),
                    )
                })?;
            binance_tools::db::x_assistant::mark_reply_failed_blocking(
                &run_id,
                "X 在唯一一次重试后仍显示发送错误",
            )
            .map_err(|error| uncertain_send_error(&run_id, format!("保存发送失败状态失败：{error:#}")))?;

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
        "判断刚才的回复是否已经成功发送。优先检查 X 的明确成功提示；也可检查回复是否出现在原帖下方。只有明确成功才返回 ok=true、completed=true。若没有明确成功证据、无法判断或截图状态含糊，必须返回 ok=false、completed=false。严格返回 JSON：{\"ok\":true,\"completed\":true,\"reason\":\"\"}。",
        cancel,
    )
    .map_err(|error| uncertain_send_error(&run_id, format!("提交后视觉状态核验失败：{error:#}")))?;
    if !done_decision.ok || !done_decision.completed {
        automation::log_event(&format!(
            "无法确认回复已发送（{}）；停止自动回复，不继续处理下一条。",
            fallback_reason(&done_decision, "完成状态不明确")
        ));
        // 提交动作已经发生，且页面可能因发送成功自动关闭弹窗。
        // 不再按 Escape 或尝试“放弃草稿”，避免误操作已发送内容；保留页面供人工核验。
        automation::log_event(
            "阶段 3/3：视觉模型未确认发送结果；保留当前页面，不执行关闭/放弃草稿操作，也绝不重复提交。"
        );
        return Err(uncertain_send_error(&run_id, format!(
            "无法确认回复是否已发送（{}）；保留页面供人工核验，本轮停止，不继续处理下一条",
            fallback_reason(&done_decision, "完成状态不明确")
        )));
    }

    mark_reply_completed_or_stop(&run_id)?;
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


fn uncertain_send_error(run_id: &str, reason: impl std::fmt::Display) -> anyhow::Error {
    let reason = reason.to_string();
    if let Err(error) =
        binance_tools::db::x_assistant::mark_reply_uncertain_blocking(run_id, &reason)
    {
        automation::log_event(&format!(
            "发送状态不明确，SQLite 写入 uncertain 状态也失败：{error:#}"
        ));
    }
    anyhow::Error::new(SendOutcomeUncertain).context(reason)
}

/// 发送结果确认后，短暂重试数据库写入；仍失败则停止自动回复，避免重复处理。
fn mark_reply_completed_or_stop(run_id: &str) -> anyhow::Result<()> {
    let mut last_error = None;
    for attempt in 1..=3 {
        match binance_tools::db::x_assistant::mark_last_step_completed_blocking(run_id) {
            Ok(()) => return Ok(()),
            Err(error) => {
                automation::log_event(&format!(
                    "发送结果已确认，SQLite 完成状态写入第 {attempt}/3 次失败：{error:#}"
                ));
                last_error = Some(error);
                if attempt < 3 {
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
    Err(uncertain_send_error(&run_id, format!(
        "X 已确认回复发送，但 SQLite 连续 3 次无法保存完成状态；run_id={run_id}；最后错误：{}",
        last_error.map(|error| format!("{error:#}")).unwrap_or_else(|| "未知错误".to_string())
    )))
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
    binance_tools::db::x_assistant::mark_reply_duplicate_blocking(run_id)
        .map_err(|error| uncertain_send_error(run_id, format!("保存重复发送状态失败：{error:#}")))?;
    let cancelled = cancel.load(Ordering::Relaxed);
    if cancelled {
        automation::log_event("发送结果已确认，但收到停止信号；跳过关闭编辑框，不再执行任何 UI 点击。");
    } else {
        close_reply_composer_safely(dir, id, &mut screenshots, cancel)
            .map_err(|error| uncertain_send_error(&run_id, format!("X 已确认该回复发送过，但关闭编辑框失败：{error:#}")))?;
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
        summary: if cancelled {
            "X 已提示该回复发送过；收到停止信号，未执行关闭弹窗操作，也未再次发送。".to_string()
        } else {
            "X 已提示该回复发送过；已关闭编辑框并跳过，未再次发送。".to_string()
        },
    })
}

fn ensure_reply_page_route(shot: &Shot, phase: &str) -> anyhow::Result<()> {
    let current_url = ocr::recognize_current_url(&shot.png)?;
    let allowed = current_url.as_deref().is_some_and(|url| {
        ocr::is_x_home_address(url) || ocr::is_x_compose_post_address(url)
    });
    automation::log_event(&format!(
        "{phase}：页面路由核验，识别地址={current_url:?}，允许继续={allowed}。"
    ));
    anyhow::ensure!(
        allowed,
        "{phase}：当前页面不是允许的 X 首页/回复页面（地址：{current_url:?}）；可能误进入 Grok 或其他页面，停止所有点击"
    );
    Ok(())
}

fn close_reply_composer_safely(
    dir: &Path,
    id: &impl std::fmt::Display,
    screenshots: &mut Vec<PathBuf>,
    cancel: &AtomicBool,
) -> anyhow::Result<()> {
    for attempt in 0..2 {
        check_cancel(cancel)?;
        // 只发送 Escape，不再点击固定比例的“窗口外坐标”。
        // 固定坐标可能命中侧栏里的 Grok 导航入口，导致自动回复跳转页面。
        native::press_escape()?;
        wait_cancel(cancel, Duration::from_millis(900))?;
        let shot = capture(
            dir,
            id,
            &format!("05-network-error-close-{}", attempt + 1),
            screenshots,
        )?;
        ensure_reply_page_route(&shot, "网络错误关闭回复框")?;

        if let Some((discard_x, discard_y)) = ocr::locate_discard_draft_button(&shot.png)? {
            automation::log_event(&format!(
                "阶段 3/3：识别到草稿确认框，点击 OCR 精确定位的‘放弃’按钮 ({discard_x}, {discard_y})。"
            ));
            ensure_reply_page_route(&shot, "点击放弃草稿前")?;
            native::click_at_client_position(discard_x, discard_y)?;
            wait_cancel(cancel, Duration::from_millis(900))?;
            let after_discard = capture(
                dir,
                id,
                &format!("05-network-error-discard-{}", attempt + 1),
                screenshots,
            )?;
            ensure_reply_page_route(&after_discard, "放弃草稿后")?;
            if !ocr::reply_composer_is_open(&after_discard.png)? {
                automation::log_event("阶段 3/3：草稿确认后回复框文字已消失；页面路由仍在 X，结束关闭流程。");
                return Ok(());
            }
        } else if ocr::recognize_save_draft_confirmation(&shot.png)? {
            // 不能在无法定位按钮时猜测坐标；保留页面供人工处理。
            anyhow::bail!(
                "识别到草稿确认框，但未能精确定位‘放弃’按钮；为避免误点 Grok/其他入口，停止自动操作"
            );
        } else if !ocr::reply_composer_is_open(&shot.png)? {
            // OCR 未识别到输入框不是关闭成功的证据。再按一次 Escape 并重新截图，
            // 不执行任何固定坐标点击。
            if attempt == 0 {
                automation::log_event(
                    "阶段 3/3：OCR 未识别到回复输入框，但不据此判断已关闭；再次按 Escape 后重新核验。",
                );
                continue;
            }
            anyhow::bail!(
                "连续两次未识别到回复输入框，且无法取得关闭成功的正向证据；保留页面供人工检查"
            );
        }
    }

    anyhow::bail!("网络错误后回复窗口仍未能确认关闭；已停止自动操作，未执行盲目点击")
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
        repost_point: local.repost_point.map(|(x, y)| Point { x, y }),
        like_point: local.like_point.map(|(x, y)| Point { x, y }),
        post_text_bottom_y: Some(local.post_text_bottom_y),
        post_bottom_y: Some(local.post_bottom_y),
        refresh_posts: false,
        composer_open: false,
        text_matches: false,
        reply_field_matches: false,
        poll_mode: false,
        completed: false,
        reply_post_state: None,
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
    _cancel: &AtomicBool,
    reply_button_missed: bool,
    reason: &str,
) -> anyhow::Result<VisionReplyResult> {
    automation::log_event(&format!(
        "阶段 1/3：{reason}；不点击回复按钮、不生成或保存回复。"
    ));
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
        repost_point: post.repost_point.map(|(x, y)| Point { x, y }),
        like_point: post.like_point.map(|(x, y)| Point { x, y }),
        post_text_bottom_y: Some(post.post_text_bottom_y),
        post_bottom_y: Some(post.post_bottom_y),
        refresh_posts: false,
        composer_open: false,
        text_matches: false,
        reply_field_matches: false,
        poll_mode: false,
        completed: false,
        reply_post_state: None,
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
    crate::tasks::request_screenshot_cleanup();
    let bmp = dir.join(format!("{id}-{stage}.bmp"));
    automation::log_event(&format!("开始采集全桌面截图：{stage}"));
    let capture_started = std::time::Instant::now();
    native::capture_desktop(&bmp)?;
    let image = image::open(&bmp)?;
    // BMP 仅作为 GDI 截图的临时中间文件，解码进内存后立即删除，避免每张图保留双份。
    let _ = std::fs::remove_file(&bmp);
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
    crate::tasks::request_screenshot_cleanup();
    let bmp = dir.join(format!("{id}-{stage}.bmp"));
    automation::log_event(&format!("开始采集阶段截图：{stage}"));
    let (expected_width, expected_height) = native::get_window_client_size()?;
    anyhow::ensure!(
        expected_width >= 800 && expected_height >= 600,
        "浏览器客户区尺寸异常（{}x{}），停止 OCR，先恢复浏览器窗口",
        expected_width,
        expected_height
    );
    let capture_started = std::time::Instant::now();
    native::capture_window(&bmp)?;
    let image = image::open(&bmp)?;
    // BMP 仅用于临时解码，马上删除；后续 OCR 使用压缩后的 PNG。
    let _ = std::fs::remove_file(&bmp);
    anyhow::ensure!(
        image.width() >= 800 && image.height() >= 600,
        "截图尺寸异常（{}x{}），停止 OCR，避免对最小化或未就绪窗口继续识别",
        image.width(),
        image.height()
    );
    anyhow::ensure!(
        image.width().abs_diff(expected_width) <= 4
            && image.height().abs_diff(expected_height) <= 4,
        "截图尺寸与浏览器客户区不一致：截图={}x{}，窗口={}x{}；停止 OCR",
        image.width(),
        image.height(),
        expected_width,
        expected_height
    );
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
    let common_width = before.width.min(after.width);
    let common_height = before.height.min(after.height);
    if before.width != after.width || before.height != after.height {
        automation::log_event(&format!(
            "目标区域复核：前后客户区尺寸不同（{}x{} -> {}x{}），仅比较共同可见区域。",
            before.width, before.height, after.width, after.height
        ));
    }
    if target.x < 0
        || target.y < 0
        || target.x >= common_width as i32
        || target.y >= common_height as i32
    {
        automation::log_event(&format!(
            "目标区域复核失败：{label}坐标 ({}, {}) 不在前后截图共同区域 {}x{} 内。",
            target.x, target.y, common_width, common_height
        ));
        return Ok(false);
    }
    let before_image = image::open(&before.png)?.to_rgb8();
    let after_image = image::open(&after.png)?.to_rgb8();
    let radius_x = 120i32;
    let radius_y = 72i32;
    let left = (target.x - radius_x).max(0) as u32;
    let right = (target.x + radius_x).min(common_width as i32) as u32;
    let top = (target.y - radius_y).max(0) as u32;
    let bottom = (target.y + radius_y).min(common_height as i32) as u32;
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
    let message = format!(
        "视觉识别 {} ms {} {} {}",
        phase_label(&shot.stage),
        elapsed.as_millis(),
        if ok { "完成" } else { "已停止或失败" },
        shot.png.display()
    );
    automation::log_event(&message);
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
        if let Some(point) = point.as_mut() {
            point.y = point
                .y
                .checked_add(crop_top as i32)
                .ok_or_else(|| anyhow::anyhow!("{label} y 坐标换算溢出"))?;
        }

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

    // 回复坐标必须存在。
    let reply = require_named_point(decision.reply_point.as_ref(), width, height, "回复按钮")?;

    // 转发和点赞坐标允许缺失，但如果提供了，就必须在截图范围内。
    let repost = decision
        .repost_point
        .as_ref()
        .map(|point| require_named_point(Some(point), width, height, "转发按钮"))
        .transpose()?;

    let like = decision
        .like_point
        .as_ref()
        .map(|point| require_named_point(Some(point), width, height, "点赞按钮"))
        .transpose()?;

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

    // 只校验实际存在的辅助坐标。
    if let Some(repost) = &repost {
        anyhow::ensure!(reply.x < repost.x, "回复与转发坐标水平顺序异常，已停止");

        anyhow::ensure!(
            reply.y.abs_diff(repost.y) <= 8,
            "回复与转发坐标不在同一水平操作栏，已停止"
        );
    }

    if let Some(like) = &like {
        anyhow::ensure!(reply.x < like.x, "回复与点赞坐标水平顺序异常，已停止");

        anyhow::ensure!(
            reply.y.abs_diff(like.y) <= 8,
            "回复与点赞坐标不在同一水平操作栏，已停止"
        );
    }

    if let (Some(repost), Some(like)) = (&repost, &like) {
        anyhow::ensure!(repost.x < like.x, "转发与点赞坐标水平顺序异常，已停止");
    }

    automation::log_event(&format!(
        "阶段 1/3：操作栏坐标通过校验，整帖底部 y={}，reply=({}, {})，repost={:?}，like={:?}",
        post_bottom_y, reply.x, reply.y, repost, like
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

fn normalize_identity_text(value: &str) -> String {
    value.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

fn identity_text_matches(expected_name: &str, dialog_text: &str) -> bool {
    let expected = normalize_identity_text(expected_name);
    let actual = normalize_identity_text(dialog_text);
    !expected.is_empty() && actual.contains(&expected)
}

fn reply_target_account_matches(
    expected: &str,
    actual: &str,
    display_name_matched: bool,
) -> bool {
    if expected.is_empty() || actual.is_empty() {
        return false;
    }
    if expected.eq_ignore_ascii_case(actual) {
        return true;
    }

    // 仅当显示名称也在当前回复弹窗的现有 OCR 文本中匹配时，
    // 才容忍 OCR 漏掉/多识别下划线；其他账号差异仍然拒绝发送。
    display_name_matched
        && expected.replace('_', "").eq_ignore_ascii_case(&actual.replace('_', ""))
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
        Point, identity_text_matches, parse_decision, reply_target_account_matches,
        validate_action_targets, validate_composer_input_point,
    };

    #[test]
    fn accepts_missing_underscore_only_when_display_name_matches() {
        assert!(identity_text_matches("特立独行", "回复给 @Ai_SoloWalker\n特立独行"));
        assert!(reply_target_account_matches("AiSoloWalker", "Ai_SoloWalker", true));
        assert!(!reply_target_account_matches("AiSoloWalker", "Ai_SoloWalker", false));
    }

    #[test]
    fn rejects_other_account_mismatches_even_when_display_name_matches() {
        assert!(!reply_target_account_matches("AiSoloWalker", "AiSoloWalkerr", true));
        assert!(!reply_target_account_matches("", "Ai_SoloWalker", true));
    }

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
    fn parses_fenced_json_and_array_points() {
        let decision =
            parse_decision("```json\n{\"ok\":true,\"point\":{\"x\":12,\"y\":34}}\n```").unwrap();
        assert!(decision.ok);
        assert_eq!(decision.point.unwrap().x, 12);
        let array_point = parse_decision(
            r#"{"ok":true,"reply_point":[350,500],"repost_point":[410,500],"like_point":[470,500]}"#,
        )
        .unwrap();
        assert_eq!(array_point.reply_point.unwrap().y, 500);
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

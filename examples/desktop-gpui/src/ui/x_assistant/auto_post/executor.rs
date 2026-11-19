//! 自动发帖执行器。
//!
//! 内容准备在浏览器锁外完成；所有浏览器操作通过 core::task_queue 串行执行。
//! 未确认的提交状态会立即停止调度，等待人工核验。

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, ensure};

use super::{
    content::{NewsFilter, prepare_next_post_blocking},
    submit::{SubmitOutcome, SubmitResult, submit_prepared_post},
};
use crate::ui::x_assistant::core::{scheduler::next_post_delay, task_queue::run_browser_task};

/// 记录自动发帖/回复联动线程触发的共享取消，避免来源被误判为页面 Drop。
fn mark_linked_cancel(cancel: &AtomicBool, source: &str) {
    let was_cancelled = cancel.swap(true, Ordering::AcqRel);
    if !was_cancelled {
        app_log::x_warn!("自动化任务联动停止：{}，已通知相关任务退出", source);
    }
}

/// 发帖前同步所选类别的 RSS 新闻；所有订阅源都失败时阻止本次发帖。
fn sync_latest_news(filter: NewsFilter) -> anyhow::Result<(usize, usize)> {
    use crypto_rss::{Collector, FeedConfig};

    let mut feeds = Vec::new();
    if matches!(filter, NewsFilter::Crypto | NewsFilter::All) {
        feeds.extend([
            FeedConfig {
                name: "Odaily 快讯".into(),
                url: "https://rss.odaily.news/rss/newsflash".into(),
            },
            FeedConfig {
                name: "BlockBeats 快讯".into(),
                url: "https://api.theblockbeats.news/v2/rss/newsflash".into(),
            },
            FeedConfig {
                name: "ChainCatcher 快讯".into(),
                url: "https://www.chaincatcher.com/rss/clist".into(),
            },
            FeedConfig {
                name: "PANews".into(),
                url: "https://rss.panewslab.com/zh/tvsq/rss".into(),
            },
        ]);
    }
    if matches!(filter, NewsFilter::Entertainment | NewsFilter::All) {
        feeds.extend([
            FeedConfig {
                name: "新浪娱乐要闻".into(),
                url: "https://rss.sina.com.cn/ent/hot_roll.xml".into(),
            },
            FeedConfig {
                name: "新浪明星全接触".into(),
                url: "https://rss.sina.com.cn/ent/star/focus7.xml".into(),
            },
            FeedConfig {
                name: "热摸爽综合热点".into(),
                url: "https://remoshuang.com/feed.xml".into(),
            },
            FeedConfig {
                name: "新浪内地明星".into(),
                url: "https://rss.sina.com.cn/ent/inner.xml".into(),
            },
            FeedConfig {
                name: "中国新闻网文娱".into(),
                url: "https://www.chinanews.com.cn/rss/culture.xml".into(),
            },
        ]);
    }

    let sync_started = std::time::Instant::now();
    app_log::post_info!("X 自动发帖：开始同步最新 RSS，订阅源数量={}", feeds.len());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (items, reports) = runtime.block_on(async {
        let collector = Collector::new()?;
        Ok::<_, anyhow::Error>(collector.collect_all(&feeds).await)
    })?;

    let successful_feeds = reports
        .iter()
        .filter(|report| report.error.is_none())
        .count();
    let failed_feeds = reports.len().saturating_sub(successful_feeds);
    for report in &reports {
        match &report.error {
            Some(error) => {
                app_log::post_warn!("RSS 同步失败：{}：{}", report.source, error);
            }
            None => {
                app_log::post_info!(
                    "RSS 同步完成：{}，获取 {} 条",
                    report.source,
                    report.fetched
                );
            }
        }
    }
    anyhow::ensure!(
        successful_feeds > 0,
        "RSS 同步失败：所选新闻类别的所有订阅源均不可用，已停止发帖，不使用数据库旧新闻"
    );

    let mut crypto_items = Vec::new();
    let mut entertainment_items = Vec::new();
    for item in items {
        let is_entertainment = item.source.contains("新浪")
            || item.source.contains("中国新闻网")
            || item.source.contains("热摸爽");
        if is_entertainment {
            entertainment_items.push(binance_tools::db::entertainment_news::NewsItemEntity {
                source: item.source,
                title: item.title,
                url: item.url,
                summary: item.summary,
                published_at: item.published_at.map(|value| value.to_rfc3339()),
                collected_at: item.collected_at.to_rfc3339(),
            });
        } else {
            crypto_items.push(binance_tools::db::crypto_news::NewsItemEntity {
                source: item.source,
                title: item.title,
                url: item.url,
                summary: item.summary,
                published_at: item.published_at.map(|value| value.to_rfc3339()),
                collected_at: item.collected_at.to_rfc3339(),
            });
        }
    }

    let connection = binance_tools::db::open_default_connection()?;
    let mut inserted = 0usize;
    if !crypto_items.is_empty() {
        inserted += binance_tools::db::crypto_news::insert_news_items(&connection, &crypto_items)?;
    }
    if !entertainment_items.is_empty() {
        inserted += binance_tools::db::entertainment_news::insert_news_items(
            &connection,
            &entertainment_items,
        )?;
    }
    app_log::post_info!(
        "X 自动发帖：RSS 同步结束，成功订阅源={}，失败订阅源={}，新增新闻={} 条，耗时={} 秒",
        successful_feeds,
        failed_feeds,
        inserted,
        sync_started.elapsed().as_secs_f32()
    );
    Ok((inserted, failed_feeds))
}

fn wait_or_cancel(cancel: &AtomicBool, duration: Duration) -> anyhow::Result<()> {
    let mut remaining = duration;
    while !remaining.is_zero() {
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let slice = remaining.min(Duration::from_secs(1));
        thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
    Ok(())
}

/// 后台最多预热一条待发文案；预热只准备内容，不会点击发布。
const PREFETCH_TARGET: usize = 1;
const RSS_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const PREFETCH_RETRY_INTERVAL: Duration = Duration::from_secs(30);

fn binance_key_configured() -> bool {
    binance_tools::db::square::load_square_api_key_blocking()
        .ok()
        .flatten()
        .map(|key| !key.api_key.trim().is_empty())
        .unwrap_or(false)
}

fn send_binance_news_task(
    post_id: i64,
    content: &str,
) -> anyhow::Result<binance_tools::square::SquareSendStatus> {
    let result = binance_tools::square::send_square_message_now_blocking(content.to_string())?;
    let (status, error) = match &result.status {
        binance_tools::square::SquareSendStatus::Success => ("sent", None),
        binance_tools::square::SquareSendStatus::Skipped => {
            ("skipped", result.error_message.as_deref())
        }
        _ => ("failed", result.error_message.as_deref()),
    };
    binance_tools::db::news_send_tasks::update_task_status_blocking(
        post_id,
        binance_tools::db::news_send_tasks::PLATFORM_BINANCE,
        status,
        error,
    )?;
    Ok(result.status)
}
fn prefetch_loop(cancel: Arc<AtomicBool>, stop: Arc<AtomicBool>, filter: NewsFilter) {
    let mut last_sync: Option<std::time::Instant> = None;
    loop {
        if cancel.load(Ordering::Acquire) || stop.load(Ordering::Acquire) {
            break;
        }
        let target_mode = binance_tools::db::news_send_tasks::load_target_mode_blocking()
            .unwrap_or_else(|_| "all".to_string());
        let key_configured = binance_key_configured();
        if last_sync
            .map(|instant| instant.elapsed() >= RSS_REFRESH_INTERVAL)
            .unwrap_or(true)
        {
            match sync_latest_news(filter) {
                Ok((inserted, failed)) => {
                    app_log::post_info!(
                        "后台预热：RSS 同步新增 {} 条，失败订阅源 {} 个",
                        inserted,
                        failed
                    );
                }
                Err(error) => {
                    app_log::post_warn!("后台预热：RSS 同步失败：{error:#}");
                }
            }
            last_sync = Some(std::time::Instant::now());
        }
        if let Err(error) =
            binance_tools::db::news_send_tasks::ensure_tasks_for_pending_posts_blocking(
                &target_mode,
                key_configured,
            )
        {
            app_log::post_warn!("后台预热：补充平台发送任务失败：{error:#}");
        }
        let ready = binance_tools::db::news_send_tasks::count_ready_posts_blocking(&target_mode)
            .unwrap_or(0);
        if ready < PREFETCH_TARGET {
            match prepare_next_post_blocking(filter, &cancel) {
                Ok(Some(post)) => {
                    if let Err(error) =
                        binance_tools::db::news_send_tasks::ensure_tasks_for_post_blocking(
                            post.post_id,
                            &target_mode,
                            key_configured,
                        )
                    {
                        app_log::post_warn!(
                            "后台预热：创建平台任务失败 post_id={}：{error:#}",
                            post.post_id
                        );
                    } else {
                        app_log::post_info!(
                            "后台预热：仅生成待发文案，尚未点击发布；post_id={}，预热队列约 {}/{} 条",
                            post.post_id,
                            ready + 1,
                            PREFETCH_TARGET
                        );
                    }
                }
                Ok(None) => {
                    let _ = wait_or_cancel(&cancel, PREFETCH_RETRY_INTERVAL);
                }
                Err(error) => {
                    app_log::post_warn!("后台预热：准备文案失败：{error:#}");
                    let _ = wait_or_cancel(&cancel, PREFETCH_RETRY_INTERVAL);
                }
            }
        } else {
            for _ in 0..10 {
                if cancel.load(Ordering::Acquire) || stop.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn run_once_with_mode(
    cancel: &AtomicBool,
    filter: NewsFilter,
    require_enabled: bool,
) -> anyhow::Result<Option<SubmitResult>> {
    ensure!(!cancel.load(Ordering::Acquire), "自动发帖已停止");
    let settings = binance_tools::db::x_posting::load_task_settings_blocking()?;
    if require_enabled && !settings.auto_post_enabled {
        return Ok(None);
    }
    let target_mode = binance_tools::db::news_send_tasks::load_target_mode_blocking()?;
    let hourly_count =
        binance_tools::db::news_send_tasks::count_sent_posts_last_hour_blocking(&target_mode)?;
    ensure!(
        hourly_count < u64::from(settings.posts_per_hour.clamp(1, 10)),
        "已达到每小时发帖上限"
    );
    let filter_name = match filter {
        NewsFilter::Crypto => "crypto",
        NewsFilter::Entertainment => "entertainment",
        NewsFilter::All => "all",
    };
    let Some((post_id, content)) =
        binance_tools::db::news_send_tasks::load_next_pending_by_filter_blocking(
            &target_mode,
            filter_name,
        )?
    else {
        app_log::post_info!("自动发帖：预热队列为空，等待后台准备数据");
        return Ok(None);
    };
    let post = super::content::PreparedPost { post_id, content };
    let key_configured = binance_key_configured();
    binance_tools::db::news_send_tasks::ensure_tasks_for_post_blocking(
        post_id,
        &target_mode,
        key_configured,
    )?;
    if cancel.load(Ordering::Acquire) {
        return Ok(None);
    }
    if require_enabled
        && !binance_tools::db::x_posting::load_task_settings_blocking()?.auto_post_enabled
    {
        return Ok(None);
    }
    let x_enabled = binance_tools::db::news_send_tasks::target_mode_includes(
        &target_mode,
        binance_tools::db::news_send_tasks::PLATFORM_X,
    );
    let binance_enabled = binance_tools::db::news_send_tasks::target_mode_includes(
        &target_mode,
        binance_tools::db::news_send_tasks::PLATFORM_BINANCE,
    ) && key_configured;
    let x_claimed = x_enabled
        && binance_tools::db::news_send_tasks::claim_task_blocking(
            post_id,
            binance_tools::db::news_send_tasks::PLATFORM_X,
        )?;
    let binance_claimed = binance_enabled
        && binance_tools::db::news_send_tasks::claim_task_blocking(
            post_id,
            binance_tools::db::news_send_tasks::PLATFORM_BINANCE,
        )?;
    if !x_claimed && !binance_claimed {
        return Ok(None);
    }
    let binance_content = post.content.clone();
    let (x_result, binance_result) = std::thread::scope(|scope| {
        let binance = binance_claimed
            .then(|| scope.spawn(|| send_binance_news_task(post_id, &binance_content)));
        let x = x_claimed.then(|| run_browser_task(|| submit_prepared_post(&post, cancel)));
        let binance = binance.map(|handle| {
            handle
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("币安发送线程异常退出")))
        });
        (x, binance)
    });
    if let Some(Err(error)) = &binance_result {
        let _ = binance_tools::db::news_send_tasks::update_task_status_blocking(
            post_id,
            binance_tools::db::news_send_tasks::PLATFORM_BINANCE,
            "failed",
            Some(&format!("{error:#}")),
        );
    }
    if let Some(Ok(result)) = x_result {
        let (status, error) = match result.outcome {
            SubmitOutcome::Published => ("sent", None),
            SubmitOutcome::Unknown => ("unknown", Some(result.message.as_str())),
        };
        binance_tools::db::news_send_tasks::update_task_status_blocking(
            post_id,
            binance_tools::db::news_send_tasks::PLATFORM_X,
            status,
            error,
        )?;
        return Ok(Some(result));
    }
    if let Some(Err(error)) = x_result {
        return Err(error).with_context(|| format!("X 发布失败 post_id={post_id}"));
    }
    match binance_result {
        Some(Ok(status))
            if status == binance_tools::square::SquareSendStatus::Success
                || status == binance_tools::square::SquareSendStatus::Skipped =>
        {
            Ok(Some(SubmitResult {
                post_id,
                outcome: SubmitOutcome::Published,
                message: format!("币安发送状态：{}", status.as_str()),
            }))
        }
        Some(Ok(status)) => anyhow::bail!("币安发送失败：{}", status.as_str()),
        Some(Err(error)) => Err(error),
        None => Ok(None),
    }
}
pub fn run_once(cancel: &AtomicBool, filter: NewsFilter) -> anyhow::Result<Option<SubmitResult>> {
    run_once_with_mode(cancel, filter, true)
}

/// 测试按钮只准备并提交一条真实帖子，不启动持续循环；测试帖子同样计入小时限额。
pub fn run_test_once(cancel: &AtomicBool, filter: NewsFilter) -> anyhow::Result<SubmitResult> {
    let result = run_once_with_mode(cancel, filter, false)?
        .context("所选新闻类型没有可用于测试的未发布候选")?;
    Ok(result)
}

/// 按数据库中的每小时上限持续发布。未知结果会跳过当前记录并保守等待后继续；
/// 不会自动重试可能已提交的内容。
pub fn run_loop(cancel: Arc<AtomicBool>, filter: NewsFilter) -> anyhow::Result<u64> {
    let prefetch_stop = Arc::new(AtomicBool::new(false));
    let cancel_for_prefetch = Arc::clone(&cancel);
    let stop_for_prefetch = Arc::clone(&prefetch_stop);
    let prefetch_worker =
        thread::spawn(move || prefetch_loop(cancel_for_prefetch, stop_for_prefetch, filter));
    app_log::post_info!("自动发帖：后台预热线程已启动");
    let result = run_loop_inner(&cancel, filter);
    prefetch_stop.store(true, Ordering::Release);
    if prefetch_worker.join().is_err() {
        app_log::post_warn!("自动发帖：后台预热线程异常退出");
    }
    result
}
fn run_loop_inner(cancel: &AtomicBool, filter: NewsFilter) -> anyhow::Result<u64> {
    let mut published_this_run = 0_u64;

    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(published_this_run);
        }

        let settings = binance_tools::db::x_posting::load_task_settings_blocking()?;
        if !settings.auto_post_enabled {
            return Ok(published_this_run);
        }

        let hourly_limit = u64::from(settings.posts_per_hour.clamp(1, 10));
        let published_last_hour = binance_tools::db::x_posting::count_posts_last_hour_blocking()?;
        if published_last_hour >= hourly_limit {
            crate::ui::x_assistant::floating_status::set_waiting_for_post(true);
            crate::ui::x_assistant::automation::log_post_event(&format!(
                "自动发帖达到每小时上限 {hourly_limit}，等待 60 秒后重新检查。"
            ));
            if wait_or_cancel(cancel, Duration::from_secs(60)).is_err() {
                return Ok(published_this_run);
            }
            crate::ui::x_assistant::floating_status::set_waiting_for_post(false);
            continue;
        }

        let age = binance_tools::db::x_posting::seconds_since_last_post_blocking()?;
        let delay = next_post_delay(settings.posts_per_hour, age);
        if !delay.is_zero() {
            crate::ui::x_assistant::floating_status::set_waiting_for_post(true);
            crate::ui::x_assistant::automation::log_post_event(&format!(
                "自动发帖频率控制：还需等待 {} 秒。",
                delay.as_secs()
            ));
            if wait_or_cancel(cancel, delay).is_err() {
                return Ok(published_this_run);
            }
            crate::ui::x_assistant::floating_status::set_waiting_for_post(false);
            continue;
        }

        let next_result = match run_once(cancel, filter) {
            Ok(result) => result,
            Err(error) if cancel.load(Ordering::Acquire) => {
                crate::ui::x_assistant::automation::log_post_event(&format!(
                    "收到停止指令，结束当前自动发帖循环：{error:#}"
                ));
                return Ok(published_this_run);
            }
            Err(error) => return Err(error),
        };

        match next_result {
            Some(result) if result.outcome == SubmitOutcome::Published => {
                published_this_run += 1;
                crate::ui::x_assistant::automation::log_post_event(&format!(
                    "自动发帖成功：post_id={}，本轮已发布 {} 条。",
                    result.post_id, published_this_run
                ));
                // 下一轮会根据刚写入的 published_at 重新计算间隔。
            }
            Some(result) if result.outcome == SubmitOutcome::Unknown => {
                crate::ui::x_assistant::automation::log_post_event(&format!(
                    "发帖结果不确定（post_id={}），已记为 unknown 并跳过该条，不会重发；自动发帖将在 60 秒冷却后继续处理其他待发记录。详情：{}",
                    result.post_id, result.message
                ));
                if wait_or_cancel(cancel, Duration::from_secs(60)).is_err() {
                    return Ok(published_this_run);
                }
            }
            Some(result) => {
                anyhow::bail!("未预期的发帖结果：{}", result.message);
            }
            None => {
                crate::ui::x_assistant::automation::log_post_event(
                    "待发队列为空，后台继续预热，60 秒后重新检查。",
                );
                if wait_or_cancel(cancel, Duration::from_secs(60)).is_err() {
                    return Ok(published_this_run);
                }
            }
        }
    }
}

/// 自动回复每完成 10 轮时调用一次：最多发布一条帖子，然后立即交还浏览器。
/// 未到发布间隔或暂时没有候选时跳过本次发帖机会，不阻塞自动回复的后续 10 轮。
fn run_scheduled_post_turn(
    cancel: &AtomicBool,
    filter: NewsFilter,
    published_count: &mut u64,
) -> anyhow::Result<bool> {
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }

    let settings = binance_tools::db::x_posting::load_task_settings_blocking()?;
    if !settings.auto_post_enabled {
        crate::ui::x_assistant::automation::log_post_event(
            "自动发帖开关已关闭，退出 10:1 联动发帖调度。",
        );
        return Ok(false);
    }

    let hourly_limit = u64::from(settings.posts_per_hour.clamp(1, 10));
    let published_last_hour = binance_tools::db::x_posting::count_posts_last_hour_blocking()?;
    if published_last_hour >= hourly_limit {
        crate::ui::x_assistant::automation::log_post_event(&format!(
            "自动发帖达到每小时上限 {hourly_limit}，本次让出浏览器，不影响自动回复。"
        ));
        return Ok(true);
    }

    let age = binance_tools::db::x_posting::seconds_since_last_post_blocking()?;
    let delay = next_post_delay(settings.posts_per_hour, age);
    if !delay.is_zero() {
        crate::ui::x_assistant::automation::log_post_event(&format!(
            "自动发帖频率控制：还需等待 {} 秒，本次跳过发帖并交还自动回复。",
            delay.as_secs()
        ));
        return Ok(true);
    }

    let result = match run_once(cancel, filter) {
        Ok(result) => result,
        Err(error) if cancel.load(Ordering::Acquire) => {
            crate::ui::x_assistant::automation::log_post_event(&format!(
                "自动发帖调度期间收到停止指令：{error:#}"
            ));
            return Ok(false);
        }
        Err(error) => return Err(error),
    };

    match result {
        Some(result) if result.outcome == SubmitOutcome::Published => {
            *published_count += 1;
            crate::ui::x_assistant::automation::log_post_event(&format!(
                "自动发帖成功：post_id={}，本轮累计发布 {} 条；浏览器操作权交还自动回复。",
                result.post_id, published_count
            ));
            Ok(true)
        }
        Some(result) if result.outcome == SubmitOutcome::Unknown => {
            crate::ui::x_assistant::automation::log_post_event(&format!(
                "发帖结果不确定（post_id={}），已记为 unknown 并跳过该条，不会重发；自动回复继续运行，后续发帖轮次继续调度。详情：{}",
                result.post_id, result.message
            ));
            Ok(true)
        }
        Some(result) => anyhow::bail!("未预期的发帖结果：{}", result.message),
        None => {
            crate::ui::x_assistant::automation::log_post_event(
                "待发队列为空，本轮交还浏览器；回复任务继续，下一轮再检查。",
            );
            Ok(true)
        }
    }
}

/// 启动后由自动回复线程按 10:1 比例发出发帖请求。
/// 两个任务通过双向通道明确交接浏览器使用权，不再竞争同一把 Mutex。
pub fn run_with_auto_reply(
    cancel: Arc<AtomicBool>,
    reply_options: crate::ui::x_assistant::vision::VisionReplyOptions,
    filter: NewsFilter,
) -> anyhow::Result<u64> {
    // 双向同步通道保证：回复线程发出一次请求后等待发帖完成；
    // 发帖线程每次只执行一次发帖调度，不再启动独立无限循环争抢浏览器锁。
    let prefetch_stop = Arc::new(AtomicBool::new(false));
    let prefetch_cancel = Arc::clone(&cancel);
    let prefetch_stop_worker = Arc::clone(&prefetch_stop);
    let prefetch_worker =
        thread::spawn(move || prefetch_loop(prefetch_cancel, prefetch_stop_worker, filter));
    let (post_turn_sender, post_turn_receiver) = std::sync::mpsc::sync_channel::<()>(0);
    let (post_done_sender, post_done_receiver) = std::sync::mpsc::sync_channel::<bool>(0);

    let post_cancel = Arc::clone(&cancel);
    let post_worker = thread::spawn(move || {
        let mut published_count = 0u64;

        while post_turn_receiver.recv().is_ok() {
            if post_cancel.load(Ordering::Acquire) {
                break;
            }

            match run_scheduled_post_turn(&post_cancel, filter, &mut published_count) {
                Ok(active) => {
                    if post_done_sender.send(active).is_err() || !active {
                        break;
                    }
                }
                Err(error) => {
                    crate::ui::x_assistant::automation::log_post_event(&format!(
                        "自动发帖调度失败，已停止后续发帖；自动回复继续运行：{error:#}"
                    ));
                    let _ = post_done_sender.send(false);
                    return Err(error);
                }
            }
        }

        Ok(published_count)
    });

    let reply_cancel = Arc::clone(&cancel);
    let reply_worker = thread::spawn(move || {
        crate::ui::x_assistant::auto_reply::run_with_post_scheduler(
            reply_options,
            &reply_cancel,
            post_turn_sender,
            post_done_receiver,
        )
    });

    let reply_result = reply_worker
        .join()
        .map_err(|_| anyhow::anyhow!("自动回复工作线程异常退出"))?;
    if reply_result.is_err() {
        mark_linked_cancel(&cancel, "主线程收到自动回复错误");
    }

    // reply_worker 退出后请求通道关闭，发帖线程会从 recv() 正常退出。
    prefetch_stop.store(true, Ordering::Release);
    let _ = prefetch_worker.join();
    let post_result = post_worker
        .join()
        .map_err(|_| anyhow::anyhow!("自动发帖工作线程异常退出"))?;

    match post_result {
        Err(error) => Err(error),
        Ok(published_count) => {
            reply_result?;
            Ok(published_count)
        }
    }
}

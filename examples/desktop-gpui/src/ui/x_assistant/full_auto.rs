use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::Context;

use super::{
    automation,
    vision::{self, VisionReplyOptions, VisionReplyResult},
};

// 连续 10 轮未完成回复或报错时停止，避免异常状态下无限空转。
const MAX_CONSECUTIVE_FAILURES: usize = 10;
const MAX_RUNTIME: Duration = Duration::from_secs(4 * 60 * 60);

struct RuntimeWatchdog(Arc<AtomicBool>);

impl Drop for RuntimeWatchdog {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn start_runtime_watchdog() -> RuntimeWatchdog {
    let cancelled = Arc::new(AtomicBool::new(false));
    let thread_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        let started = std::time::Instant::now();
        while started.elapsed() < MAX_RUNTIME {
            if thread_cancelled.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
        if !thread_cancelled.load(Ordering::Relaxed) {
            automation::log_event("全自动运行满 4 小时，强制结束整个应用进程。");
            std::process::exit(0);
        }
    });
    RuntimeWatchdog(cancelled)
}

pub fn run(options: VisionReplyOptions, cancel: &AtomicBool) -> anyhow::Result<VisionReplyResult> {
    anyhow::ensure!(options.auto_send, "全自动回复需要开启自动发送");
    anyhow::ensure!(!options.hover_only, "全自动回复不能使用仅移动模式");

    let _runtime_watchdog = start_runtime_watchdog();
    let mut completed_count = 0usize;
    let mut consecutive_failures = 0usize;
    let mut last_result: Option<VisionReplyResult> = None;

    // 循环按轮处理；停止按钮通过 cancel 让当前识图、等待或滚动尽快结束。
    loop {
        if cancel.load(Ordering::Relaxed) {
            return stopped_result(last_result, completed_count);
        }

        match vision::run_visual_reply(options.clone(), cancel) {
            Ok(result) => {
                if result.skipped_duplicate {
                    consecutive_failures = 0;
                    automation::log_event(
                        "全自动：该账号 24 小时内已回复，按正常跳过处理；不计失败、不刷新，改为滚动寻找下一帖。",
                    );
                    last_result = Some(result);

                    if let Err(error) =
                        vision::scroll_to_next_post(cancel, "跳过已回复账号，向下查找下一条帖子")
                    {
                        if cancel.load(Ordering::Relaxed) {
                            return stopped_result(last_result, completed_count);
                        }
                        return Err(error).context("跳过重复账号后的滚动失败");
                    }
                    continue;
                }

                // 成功计数；其他未完成情况记入连续未完成次数。
                if result.completed {
                    completed_count += 1;
                    consecutive_failures = 0;
                    automation::log_event(&format!("全自动：已完成第 {completed_count} 条回复。"));
                } else {
                    consecutive_failures += 1;
                    automation::log_event(&format!(
                        "全自动：本轮未完成发送（连续未完成 {consecutive_failures}/{MAX_CONSECUTIVE_FAILURES}）。"
                    ));
                }
                last_result = Some(result);

                // 正常完成后只滚动查找下一帖，避免每轮 F5 和全桌面刷新检查的额外开销。
                if let Err(error) =
                    vision::scroll_to_next_post(cancel, "本轮处理结束，向下查找下一条帖子")
                {
                    if cancel.load(Ordering::Relaxed) {
                        return stopped_result(last_result, completed_count);
                    }
                    return Err(error).context("全自动流程恢复动作失败");
                }
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    anyhow::bail!(
                        "连续 {} 轮未完成回复，已自动停止以避免持续空转",
                        MAX_CONSECUTIVE_FAILURES
                    );
                }
            }
            // 用户停止导致的错误不是运行故障，返回最近一轮结果。
            Err(error) if cancel.load(Ordering::Relaxed) => {
                automation::log_event(&format!("全自动：收到手动停止请求：{error:#}"));
                return stopped_result(last_result, completed_count);
            }
            Err(error) if error.downcast_ref::<vision::RateLimitDetected>().is_some() => {
                return Err(error).context(format!(
                    "检测到平台限流，全自动已停止；累计完成 {completed_count} 条回复"
                ));
            }
            Err(error) => {
                // 普通故障随机刷新或滚动恢复；连续失败达到上限后终止循环。
                consecutive_failures += 1;

                automation::log_event(&format!(
                    "全自动：本轮报错（连续失败 {consecutive_failures}/{MAX_CONSECUTIVE_FAILURES}）：{error:#}"
                ));
                let recovery = if vision::is_incomplete_post_error(&error) {
                    vision::scroll_down_to_complete_post(cancel, "帖子信息不完整，短距离补滚后重试")
                } else {
                    vision::randomized_recovery(cancel, "本轮报错，尝试刷新或滚动后恢复")
                };
                if let Err(recovery_error) = recovery {
                    if cancel.load(Ordering::Relaxed) {
                        return stopped_result(last_result, completed_count);
                    }
                    return Err(recovery_error)
                        .with_context(|| format!("全自动错误恢复失败；上一轮错误：{error:#}"));
                }
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    anyhow::bail!(
                        "连续 {} 轮未能完成，已自动停止以避免持续误操作；最后错误：{error:#}",
                        MAX_CONSECUTIVE_FAILURES
                    );
                }
            }
        }
    }
}

// 手动停止或运行时限到达时，保留最近一轮结果和累计完成数。
fn stopped_result(
    last_result: Option<VisionReplyResult>,
    completed_count: usize,
) -> anyhow::Result<VisionReplyResult> {
    if let Some(mut result) = last_result {
        result.summary = format!(
            "已手动停止全自动流程，累计完成 {completed_count} 条回复。{}",
            result.summary
        );
        return Ok(result);
    }
    anyhow::bail!("全自动流程已手动停止，尚未完成任何一轮")
}

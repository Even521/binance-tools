use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use super::super::{
    automation,
    vision::{self, VisionReplyOptions, VisionReplyResult},
};
use crate::ui::x_assistant::core::task_queue::run_browser_task;

const REPLY_ROUNDS_PER_POST: usize = 10;

/// 全自动回复运行器。
///
/// 集中管理回复状态、失败次数和最后一轮结果。
#[derive(Default)]
struct AutoReplyRunner {
    rounds: usize,
    completed: usize,
    failures: usize,
    last_result: Option<VisionReplyResult>,
}

impl AutoReplyRunner {
    /// 执行全自动回复主循环。
    ///
    /// 统一负责取消检查、单轮调度及最终退出。
    fn run(
        &mut self,
        options: VisionReplyOptions,
        cancel: &AtomicBool,
        post_scheduler: Option<(SyncSender<()>, Receiver<bool>)>,
    ) -> Result<VisionReplyResult> {
        automation::log_event("自动回复循环启动");
        let mut first_reply_completed = false;
        let mut rounds_since_post = 0usize;
        let mut post_scheduler_active = post_scheduler.is_some();

        loop {
            if Self::is_cancelled(cancel) {
                automation::log_event("自动回复收到停止信号，退出循环");
                return self.stop();
            }

            ensure_daily_reply_capacity()?;

            let completed_before = self.completed;
            let result = self.run_once(&options, cancel);

            // 至少有一条回复真正发送成功后，才允许联动自动发帖。
            // 重复账号、识别不完整或未发送成功都不会提前开启发帖调度。
            if result.is_ok() && self.completed > completed_before {
                ensure_daily_reply_capacity()?;
                if !first_reply_completed {
                    first_reply_completed = true;
                    automation::log_event("自动回复首条成功，已启用 10:1 自动发帖调度");
                }
            }

            // 用户取消优先于当前轮次产生的错误。
            if Self::is_cancelled(cancel) {
                return self.stop();
            }

            result?;

            // 每完成 10 个自动回复轮次，才向自动发帖线程交出一次浏览器使用权。
            // 轮次包括成功、跳过重复内容以及已恢复的普通失败；每轮浏览器操作已完整结束。
            if post_scheduler_active {
                rounds_since_post += 1;
                if first_reply_completed && rounds_since_post >= REPLY_ROUNDS_PER_POST {
                    automation::log_event(&format!(
                        "自动回复已完成 {REPLY_ROUNDS_PER_POST} 轮，交出浏览器操作权，等待自动发帖完成 1 次调度。"
                    ));

                    let mut scheduler_available = false;
                    if let Some((request_sender, completion_receiver)) = post_scheduler.as_mut() {
                        if request_sender.send(()).is_ok() {
                            loop {
                                if Self::is_cancelled(cancel) {
                                    return self.stop();
                                }
                                match completion_receiver.recv_timeout(Duration::from_millis(200)) {
                                    Ok(active) => {
                                        scheduler_available = active;
                                        break;
                                    }
                                    Err(RecvTimeoutError::Timeout) => continue,
                                    Err(RecvTimeoutError::Disconnected) => break,
                                }
                            }
                        }
                    }

                    rounds_since_post = 0;
                    post_scheduler_active = scheduler_available;
                    if post_scheduler_active {
                        automation::log_event("自动发帖调度结束，浏览器操作权已交还自动回复。");
                    } else {
                        automation::log_event(
                            "自动发帖调度已退出或不可用，自动回复将继续独立运行。",
                        );
                    }
                }
            }
        }
    }

    /// 执行单轮视觉回复。
    ///
    /// 调用视觉识别模块，并根据执行结果分发处理。
    fn run_once(&mut self, options: &VisionReplyOptions, cancel: &AtomicBool) -> Result<()> {
        run_browser_task(|| {
            self.rounds += 1;
            let round = self.rounds;
            let started = Instant::now();
            let _screenshot_round = match vision::begin_screenshot_round(round) {
                Ok(guard) => guard,
                Err(error) => {
                    log_round_error(round, &error, cancel, started.elapsed());
                    return self.handle_error(error, cancel);
                }
            };

            match vision::run_visual_reply(options.clone(), cancel) {
                Ok(result) => {
                    log_round_result(round, &result, started.elapsed());
                    self.handle_result(result, cancel)
                }
                Err(error) => {
                    log_round_error(round, &error, cancel, started.elapsed());
                    self.handle_error(error, cancel)
                }
            }
        })
    }

    /// 处理正常返回的识图结果。
    ///
    /// 重复账号跳过；成功则计数；未完成则累计失败。
    /// 最后滚动寻找下一条帖子。
    fn handle_result(&mut self, result: VisionReplyResult, cancel: &AtomicBool) -> Result<()> {
        if result.skipped_duplicate {
            self.failures = 0;
            self.last_result = Some(result);

            return self.scroll_or_recover(
                cancel,
                "跳过已回复账号，向下查找下一条帖子",
            );
        }

        if result.completed {
            self.completed += 1;
            self.failures = 0;

        } else if result.summary.starts_with("提交结果未知") {
            // 仅当视觉流程已确认回复弹窗和草稿确认框都关闭、且 uncertain 已成功写入数据库时，
            // vision 才会以正常结果返回到这里。此时当前帖子已被持久化去重保护，可以安全跳过。
            // uncertain 不是发送成功，也不是普通失败：不计入成功数、不累计失败次数，并继续下一轮。
            self.failures = 0;
        } else {
            self.failures += 1;
        }

        self.last_result = Some(result);

        // 正常轮次只滚动，不刷新页面。
        self.scroll_or_recover(cancel, "本轮处理结束，向下查找下一条帖子")
    }

    /// 滚动失败属于普通恢复错误：再尝试一次分类恢复，成功后回到主循环。
    /// 取消信号仍由底层检查并立即生效；恢复失败才向上返回错误。
    fn scroll_or_recover(&self, cancel: &AtomicBool, reason: &str) -> Result<()> {
        match vision::scroll_to_next_post(cancel, reason) {
            Ok(()) => Ok(()),
            Err(error) if Self::is_cancelled(cancel) => Err(error),
            Err(error) => {
                automation::log_event(&format!(
                    "全自动：滚动下一帖失败，尝试分类恢复后继续：{error:#}"
                ));
                self.recover(&error, cancel)
            }
        }
    }

    /// 处理单轮执行异常。
    ///
    /// 取消请求立即停止；
    /// 限流或发送状态不明时安全退出；
    /// 普通错误恢复后继续主循环，不设置连续失败上限。
    fn handle_error(&mut self, error: anyhow::Error, cancel: &AtomicBool) -> Result<()> {
        if Self::is_cancelled(cancel) {
            automation::log_event(&format!("自动回复已停止：{error:#}"));
            return Ok(());
        }

        if error.downcast_ref::<vision::RateLimitDetected>().is_some() {
            return Err(error).context(format!(
                "检测到平台限流，全自动已停止；累计完成 {} 条回复",
                self.completed
            ));
        }

        // 点击提交后状态不明时不能滚动到下一帖；停止整个自动回复，等待人工核验。
        if error.downcast_ref::<vision::SendOutcomeUncertain>().is_some() {
            automation::log_event(&format!(
                "全自动：发送结果或数据库状态无法确认，立即停止，不再处理下一条：{error:#}"
            ));
            return Err(error).context(format!(
                "发送状态不明确，全自动已安全停止；累计完成 {} 条回复",
                self.completed
            ));
        }

        self.failures += 1;

        self.recover(&error, cancel)?;
        automation::log_event("全自动：当前轮次恢复完成，继续主循环处理下一条帖子。");
        Ok(())
    }

    /// 根据错误类型恢复运行环境。
    ///
    /// 帖子信息不完整时短距离补滚；
    /// 其他错误执行随机刷新或滚动。
    fn recover(&self, error: &anyhow::Error, cancel: &AtomicBool) -> Result<()> {
        let recovery = if vision::is_incomplete_post_error(error) {
            vision::scroll_down_to_complete_post(cancel, "帖子信息不完整，短距离补滚后重试")
        } else {
            vision::recover_by_error(cancel, error)
        };
        recovery.with_context(|| format!("全自动错误恢复失败；上一轮错误：{error:#}"))
    }

    /// 检查用户是否请求停止。
    fn is_cancelled(cancel: &AtomicBool) -> bool {
        cancel.load(Ordering::Relaxed)
    }

    /// 结束运行并返回最近一轮结果。
    ///
    /// 保留累计完成数量及最后一轮执行摘要。
    fn stop(&mut self) -> Result<VisionReplyResult> {
        stopped_result(self.last_result.take(), self.completed)
    }
}


/// 输出每轮唯一的最终结果日志；详细 OCR、截图和点击诊断仍由原流程保留。
fn log_round_result(round: usize, result: &VisionReplyResult, elapsed: std::time::Duration) {
    let duration = format!("{:.1}s", elapsed.as_secs_f64());
    let reason = compact_log_value(&result.summary);

    if result.completed {
        app_log::x_info!(
            "[X助手][第{round}轮] 回复成功 | 耗时={duration}"
        );
    } else if result.summary.starts_with("提交结果未知") {
        app_log::x_warn!(
            "[X助手][第{round}轮] 发送状态不确定 | 原因={reason} | 耗时={duration}"
        );
    } else if result.skipped_duplicate || result.summary.starts_with("继续扫描下一帖") {
        app_log::x_info!(
            "[X助手][第{round}轮] 跳过 | 原因={reason} | 耗时={duration}"
        );
    } else {
        app_log::x_error!(
            "[X助手][第{round}轮] 回复失败 | 原因={reason} | 耗时={duration}"
        );
    }
}

/// 异常路径也只输出一条轮次结果；用户主动停止不记作失败。
fn log_round_error(
    round: usize,
    error: &anyhow::Error,
    cancel: &AtomicBool,
    elapsed: std::time::Duration,
) {
    let reason = compact_log_value(&format!("{error:#}"));
    let duration = format!("{:.1}s", elapsed.as_secs_f64());

    if cancel.load(Ordering::Acquire) {
        app_log::x_info!(
            "[X助手][第{round}轮] 已停止 | 原因={reason} | 耗时={duration}"
        );
    } else if error.downcast_ref::<vision::SendOutcomeUncertain>().is_some() {
        app_log::x_warn!(
            "[X助手][第{round}轮] 发送状态不确定 | 原因={reason} | 耗时={duration}"
        );
    } else {
        app_log::x_error!(
            "[X助手][第{round}轮] 回复失败 | 原因={reason} | 耗时={duration}"
        );
    }
}

/// 将错误或摘要压成单行，避免换行破坏日志检索。
fn compact_log_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ensure_daily_reply_capacity() -> Result<()> {
    let settings = binance_tools::db::x_posting::load_task_settings_blocking()?;
    let completed = binance_tools::db::x_assistant::today_sent_reply_count_blocking()?;
    let limit = u64::from(settings.daily_reply_limit);
    if completed >= limit {
        let message = format!(
            "今日已成功回复 {completed}/{limit} 条，达到每日上限；自动化已停止。请到「任务设置」手动调高每日最大回复数后再启动。"
        );
        automation::log_event(&message);
        anyhow::bail!(message);
    }
    Ok(())
}

/// 全自动回复公开入口。
///
/// 校验参数后创建运行器，启动自动回复。
pub fn run(options: VisionReplyOptions, cancel: &AtomicBool) -> Result<VisionReplyResult> {
    anyhow::ensure!(options.auto_send, "全自动回复需要开启自动发送");
    anyhow::ensure!(!options.hover_only, "全自动回复不能使用仅移动模式");

    AutoReplyRunner::default().run(options, cancel, None)
}

/// 自动化联动入口：每完成 10 轮回复，交出一次浏览器操作权给自动发帖。
pub fn run_with_post_scheduler(
    options: VisionReplyOptions,
    cancel: &AtomicBool,
    post_turn_sender: SyncSender<()>,
    post_turn_receiver: Receiver<bool>,
) -> Result<VisionReplyResult> {
    anyhow::ensure!(options.auto_send, "全自动回复需要开启自动发送");
    anyhow::ensure!(!options.hover_only, "全自动回复不能使用仅移动模式");

    AutoReplyRunner::default().run(
        options,
        cancel,
        Some((post_turn_sender, post_turn_receiver)),
    )
}

/// 生成手动停止结果。
///
/// 有最近一轮结果时追加累计完成数量；
/// 尚未产生任何结果时返回停止错误。
fn stopped_result(
    last_result: Option<VisionReplyResult>,
    completed_count: usize,
) -> Result<VisionReplyResult> {
    if let Some(mut result) = last_result {
        result.summary = format!(
            "已手动停止全自动流程，累计完成 {completed_count} 条回复。{}",
            result.summary
        );

        return Ok(result);
    }

    anyhow::bail!("全自动流程已手动停止，尚未完成任何一轮")
}

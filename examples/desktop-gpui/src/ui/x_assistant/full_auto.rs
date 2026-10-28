use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use super::{
    automation,
    vision::{self, VisionReplyOptions, VisionReplyResult},
};

/// 连续失败上限，防止异常状态下无限空转。
const MAX_CONSECUTIVE_FAILURES: usize = 10;

/// 全自动运行最长时间：4 小时。
const MAX_RUNTIME: Duration = Duration::from_secs(4 * 60 * 60);

/// 运行超时守护器。
///
/// 超过运行时限后强制结束进程；
/// 正常退出时通过 Drop 通知守护线程停止。
struct RuntimeWatchdog(Arc<AtomicBool>);

impl RuntimeWatchdog {
    /// 启动独立守护线程，监控运行时长。
    fn start() -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancelled);

        thread::spawn(move || {
            let started = Instant::now();

            while started.elapsed() < MAX_RUNTIME {
                if signal.load(Ordering::Relaxed) {
                    return;
                }

                thread::sleep(Duration::from_secs(1));
            }

            if !signal.load(Ordering::Relaxed) {
                automation::log_event("全自动运行满 4 小时，强制结束整个应用进程。");
                std::process::exit(0);
            }
        });

        Self(cancelled)
    }
}

impl Drop for RuntimeWatchdog {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// 全自动回复运行器。
///
/// 集中管理回复状态、失败次数和最后一轮结果。
#[derive(Default)]
struct AutoReplyRunner {
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
    ) -> Result<VisionReplyResult> {
        let _watchdog = RuntimeWatchdog::start();

        loop {
            if Self::is_cancelled(cancel) {
                return self.stop();
            }

            let result = self.run_once(&options, cancel);

            // 用户取消优先于当前轮次产生的错误。
            if Self::is_cancelled(cancel) {
                return self.stop();
            }

            result?;
        }
    }

    /// 执行单轮视觉回复。
    ///
    /// 调用视觉识别模块，并根据执行结果分发处理。
    fn run_once(&mut self, options: &VisionReplyOptions, cancel: &AtomicBool) -> Result<()> {
        match vision::run_visual_reply(options.clone(), cancel) {
            Ok(result) => self.handle_result(result, cancel),
            Err(error) => self.handle_error(error, cancel),
        }
    }

    /// 处理正常返回的识图结果。
    ///
    /// 重复账号跳过；成功则计数；未完成则累计失败。
    /// 最后滚动寻找下一条帖子。
    fn handle_result(&mut self, result: VisionReplyResult, cancel: &AtomicBool) -> Result<()> {
        if result.skipped_duplicate {
            self.failures = 0;
            self.last_result = Some(result);

            automation::log_event(
                "全自动：该账号 24 小时内已回复，按正常跳过处理；不计失败、不刷新，改为滚动寻找下一帖。",
            );

            return vision::scroll_to_next_post(cancel, "跳过已回复账号，向下查找下一条帖子")
                .context("跳过重复账号后的滚动失败");
        }

        if result.completed {
            self.completed += 1;
            self.failures = 0;

            automation::log_event(&format!("全自动：已完成第 {} 条回复。", self.completed));
        } else {
            self.failures += 1;

            automation::log_event(&format!(
                "全自动：本轮未完成发送（连续未完成 {}/{}）。",
                self.failures, MAX_CONSECUTIVE_FAILURES
            ));
        }

        self.last_result = Some(result);

        // 正常轮次只滚动，不刷新页面。
        vision::scroll_to_next_post(cancel, "本轮处理结束，向下查找下一条帖子")
            .context("全自动流程恢复动作失败")?;

        self.check_failure_limit(None)
    }

    /// 处理单轮执行异常。
    ///
    /// 取消请求直接交回主循环；
    /// 限流立即退出；
    /// 普通错误尝试恢复后检查失败上限。
    fn handle_error(&mut self, error: anyhow::Error, cancel: &AtomicBool) -> Result<()> {
        if Self::is_cancelled(cancel) {
            automation::log_event(&format!("全自动：收到手动停止请求：{error:#}"));
            return Ok(());
        }

        if error.downcast_ref::<vision::RateLimitDetected>().is_some() {
            return Err(error).context(format!(
                "检测到平台限流，全自动已停止；累计完成 {} 条回复",
                self.completed
            ));
        }

        self.failures += 1;

        automation::log_event(&format!(
            "全自动：本轮报错（连续失败 {}/{}）：{error:#}",
            self.failures, MAX_CONSECUTIVE_FAILURES
        ));

        self.recover(&error, cancel)?;
        self.check_failure_limit(Some(&error))
    }

    /// 根据错误类型恢复运行环境。
    ///
    /// 帖子信息不完整时短距离补滚；
    /// 其他错误执行随机刷新或滚动。
    fn recover(&self, error: &anyhow::Error, cancel: &AtomicBool) -> Result<()> {
        let recovery = if vision::is_incomplete_post_error(error) {
            vision::scroll_down_to_complete_post(cancel, "帖子信息不完整，短距离补滚后重试")
        } else {
            vision::randomized_recovery(cancel, "本轮报错，尝试刷新或滚动后恢复")
        };

        recovery.with_context(|| format!("全自动错误恢复失败；上一轮错误：{error:#}"))
    }

    /// 检查连续失败次数是否达到上限。
    ///
    /// 达到上限后返回错误，终止主循环。
    fn check_failure_limit(&self, error: Option<&anyhow::Error>) -> Result<()> {
        if self.failures < MAX_CONSECUTIVE_FAILURES {
            return Ok(());
        }

        if let Some(error) = error {
            anyhow::bail!(
                "连续 {} 轮未能完成，已自动停止以避免持续误操作；最后错误：{error:#}",
                MAX_CONSECUTIVE_FAILURES
            );
        }

        anyhow::bail!(
            "连续 {} 轮未完成回复，已自动停止以避免持续空转",
            MAX_CONSECUTIVE_FAILURES
        )
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

/// 全自动回复公开入口。
///
/// 校验参数后创建运行器，启动自动回复。
pub fn run(options: VisionReplyOptions, cancel: &AtomicBool) -> Result<VisionReplyResult> {
    anyhow::ensure!(options.auto_send, "全自动回复需要开启自动发送");
    anyhow::ensure!(!options.hover_only, "全自动回复不能使用仅移动模式");

    AutoReplyRunner::default().run(options, cancel)
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

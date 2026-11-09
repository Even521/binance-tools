use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
};

use anyhow::{Context, Result};

use super::super::{
    automation,
    vision::{self, VisionReplyOptions, VisionReplyResult},
};
use crate::ui::x_assistant::core::task_queue::run_browser_task;

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
        mut first_cycle_sender: Option<SyncSender<()>>,
    ) -> Result<VisionReplyResult> {
        automation::log_event("自动回复循环启动");
        loop {
            if Self::is_cancelled(cancel) {
                automation::log_event("自动回复收到停止信号，退出循环");
                return self.stop();
            }

            ensure_daily_reply_capacity()?;

            let completed_before = self.completed;
            let result = self.run_once(&options, cancel);

            // 只有真正成功发送并计入 completed 的回复，才允许启动自动发帖。
            // 跳过重复账号、识别不完整或未发送成功，都不能触发发帖调度。
            if result.is_ok() && self.completed > completed_before {
                ensure_daily_reply_capacity()?;
                if let Some(sender) = first_cycle_sender.take() {
                    let _ = sender.send(());
                }
            }

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
        run_browser_task(|| {
            match vision::run_visual_reply(options.clone(), cancel) {
                Ok(result) => self.handle_result(result, cancel),
                Err(error) => self.handle_error(error, cancel),
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

            automation::log_event(
                "全自动：该账号 24 小时内已回复，按正常跳过处理；不计失败、不刷新，改为滚动寻找下一帖。",
            );

            return self.scroll_or_recover(
                cancel,
                "跳过已回复账号，向下查找下一条帖子",
            );
        }

        if result.completed {
            self.completed += 1;
            self.failures = 0;

            automation::log_event(&format!("全自动：已完成第 {} 条回复。", self.completed));
        } else if result.summary.starts_with("提交结果未知") {
            // uncertain 不是明确失败，不累计连续失败次数；当前帖子仍然跳过，继续主循环。
            automation::log_event(&format!(
                "全自动：本轮提交结果未知；已记录 uncertain，不重复提交当前回复，跳过当前帖子并继续主循环。{}",
                result.summary
            ));
        } else {
            self.failures += 1;
            automation::log_event(&format!(
                "全自动：本轮未完成发送（连续未完成 {} 次）；跳过当前帖子，主循环继续。{}",
                self.failures,
                result.summary
            ));
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

        automation::log_event(&format!(
            "全自动：本轮报错（连续失败 {} 次）：{error:#}",
            self.failures
        ));

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

/// 自动化联动入口：首轮回复完成后通知外部调度器，再继续持续回复。
pub fn run_with_first_cycle(
    options: VisionReplyOptions,
    cancel: &AtomicBool,
    first_cycle_sender: SyncSender<()>,
) -> Result<VisionReplyResult> {
    anyhow::ensure!(options.auto_send, "全自动回复需要开启自动发送");
    anyhow::ensure!(!options.hover_only, "全自动回复不能使用仅移动模式");

    AutoReplyRunner::default().run(options, cancel, Some(first_cycle_sender))
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

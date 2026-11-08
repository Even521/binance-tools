//! Single-threaded X assistant task scheduling primitives.
//!
//! This module deliberately does not spawn workers or touch the browser.
//! The existing X assistant main loop owns the browser and calls these
//! selectors between safe task boundaries.

use std::time::Duration;

use binance_tools::db::x_posting::XAssistantTaskSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    AutoPost,
    AutoComment,
    OwnPostReply,
    AutoReply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStage {
    Pending,
    WaitingForBrowser,
    RecognizingPage,
    FillingContent,
    ReadyToSubmit,
    Submitting,
    VerifyingResult,
    Published,
    Commenting,
    Completed,
    Failed,
    Unknown,
    Cancelled,
}

impl TaskStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::WaitingForBrowser => "waiting_for_browser",
            Self::RecognizingPage => "recognizing_page",
            Self::FillingContent => "filling_content",
            Self::ReadyToSubmit => "ready_to_submit",
            Self::Submitting => "submitting",
            Self::VerifyingResult => "verifying_result",
            Self::Published => "published",
            Self::Commenting => "commenting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }

    /// Returns true only when the task can safely restart without repeating a
    /// potentially submitted action. Published/commenting states still require
    /// checking the corresponding post or comment on X.
    pub fn can_resume_without_remote_verification(self) -> bool {
        matches!(
            self,
            Self::Pending
                | Self::WaitingForBrowser
                | Self::RecognizingPage
                | Self::FillingContent
                | Self::ReadyToSubmit
        )
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "waiting_for_browser" => Self::WaitingForBrowser,
            "recognizing_page" => Self::RecognizingPage,
            "filling_content" => Self::FillingContent,
            "ready_to_submit" => Self::ReadyToSubmit,
            "submitting" => Self::Submitting,
            "verifying_result" => Self::VerifyingResult,
            "published" => Self::Published,
            "commenting" => Self::Commenting,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "unknown" => Self::Unknown,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDecision {
    ResumeSafely,
    VerifyPostOnX,
    VerifyCommentOnX,
    RetryAfterConfirmedFailure,
    SkipCompleted,
    ManualReview,
}

pub fn recovery_decision(stage: TaskStage) -> RecoveryDecision {
    match stage {
        TaskStage::Pending
        | TaskStage::WaitingForBrowser
        | TaskStage::RecognizingPage
        | TaskStage::FillingContent
        | TaskStage::ReadyToSubmit => RecoveryDecision::ResumeSafely,
        TaskStage::Submitting | TaskStage::VerifyingResult | TaskStage::Unknown => {
            RecoveryDecision::VerifyPostOnX
        }
        TaskStage::Published => RecoveryDecision::VerifyCommentOnX,
        TaskStage::Commenting => RecoveryDecision::VerifyCommentOnX,
        TaskStage::Failed => RecoveryDecision::RetryAfterConfirmedFailure,
        TaskStage::Completed | TaskStage::Cancelled => RecoveryDecision::SkipCompleted,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SchedulerSignals {
    pub post_is_due: bool,
    pub has_pending_post_comment: bool,
    pub has_pending_own_post_reply: bool,
    pub has_reply_candidate: bool,
}

/// Selects exactly one next action. The caller must finish the current
/// browser interaction and release its browser lease before invoking this.
pub fn select_next_task(
    settings: &XAssistantTaskSettings,
    signals: SchedulerSignals,
    published_in_last_hour: u64,
) -> Option<TaskKind> {
    if settings.auto_post_enabled
        && signals.post_is_due
        && published_in_last_hour < u64::from(settings.posts_per_hour.clamp(1, 10))
    {
        return Some(TaskKind::AutoPost);
    }

    if settings.auto_comment_enabled && signals.has_pending_post_comment {
        return Some(TaskKind::AutoComment);
    }

    if settings.own_post_reply_enabled && signals.has_pending_own_post_reply {
        return Some(TaskKind::OwnPostReply);
    }

    if settings.auto_reply_enabled && signals.has_reply_candidate {
        return Some(TaskKind::AutoReply);
    }

    None
}

/// Evenly spaces a configured number of posts across a rolling hour.
/// Returns the remaining wait from the age of the most recent published post.
pub fn next_post_delay(
    posts_per_hour: u8,
    seconds_since_last_post: Option<u64>,
) -> Duration {
    let count = posts_per_hour.clamp(1, 10) as u64;
    let interval = 3_600_u64 / count;
    let Some(age) = seconds_since_last_post else {
        return Duration::ZERO;
    };
    Duration::from_secs(interval.saturating_sub(age))
}

#[cfg(test)]
mod tests {
    use super::{RecoveryDecision, SchedulerSignals, TaskKind, TaskStage, next_post_delay, recovery_decision, select_next_task};
    use binance_tools::db::x_posting::XAssistantTaskSettings;
    use std::time::Duration;

    #[test]
    fn posting_has_priority_and_is_limited_by_rolling_hour_count() {
        let settings = XAssistantTaskSettings {
            auto_reply_enabled: true,
            auto_post_enabled: true,
            auto_comment_enabled: true,
            own_post_reply_enabled: true,
            posts_per_hour: 2,
        };
        let signals = SchedulerSignals {
            post_is_due: true,
            has_pending_post_comment: true,
            has_pending_own_post_reply: true,
            has_reply_candidate: true,
        };

        assert_eq!(select_next_task(&settings, signals, 0), Some(TaskKind::AutoPost));
        assert_eq!(select_next_task(&settings, signals, 2), Some(TaskKind::AutoComment));
    }

    #[test]
    fn task_priority_falls_through_to_auto_reply() {
        let settings = XAssistantTaskSettings {
            auto_reply_enabled: true,
            auto_post_enabled: false,
            auto_comment_enabled: false,
            own_post_reply_enabled: false,
            posts_per_hour: 3,
        };
        assert_eq!(
            select_next_task(
                &settings,
                SchedulerSignals { has_reply_candidate: true, ..Default::default() },
                0
            ),
            Some(TaskKind::AutoReply)
        );
    }

    #[test]
    fn unknown_submit_state_requires_remote_reconciliation() {
        assert!(!TaskStage::Unknown.can_resume_without_remote_verification());
        assert!(!TaskStage::Submitting.can_resume_without_remote_verification());
        assert!(TaskStage::ReadyToSubmit.can_resume_without_remote_verification());
        assert!(!TaskStage::Commenting.can_resume_without_remote_verification());
        assert!(!TaskStage::Published.can_resume_without_remote_verification());
        assert_eq!(recovery_decision(TaskStage::Submitting), RecoveryDecision::VerifyPostOnX);
        assert_eq!(recovery_decision(TaskStage::Published), RecoveryDecision::VerifyCommentOnX);
    }

    #[test]
    fn spreads_posts_over_the_hour() {
        assert_eq!(next_post_delay(3, None), Duration::ZERO);
        assert_eq!(next_post_delay(3, Some(500)), Duration::from_secs(700));
        assert_eq!(next_post_delay(3, Some(1_500)), Duration::ZERO);
    }
}

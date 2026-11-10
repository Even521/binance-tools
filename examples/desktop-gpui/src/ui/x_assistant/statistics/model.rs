//! Shared, UI-independent data structures for the daily statistics report.

/// Final classification of one deduplicated automation round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReplyOutcome {
    Success,
    Failed,
    Uncertain,
    Skipped,
}

/// Automation stage used when attributing a failed round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReplyStage {
    BrowserOpen,
    TimelineLoad,
    PostLocate,
    ReplyGenerate,
    DialogOpen,
    ReplyTextVerify,
    SubmitButtonLocate,
    SendConfirmation,
}

/// Number of failed rounds attributed to one stage.
#[derive(Clone, Debug, Default)]
pub(super) struct StageFailure {
    pub stage: Option<ReplyStage>,
    pub count: u32,
}

/// Number of failed rounds observed during one Beijing-time hour.
#[derive(Clone, Debug, Default)]
pub(super) struct HourlyFailure {
    pub hour: u8,
    pub count: u32,
}

/// Aggregated statistics for exactly one Beijing calendar day.
#[derive(Clone, Debug, Default)]
pub(super) struct DailyReplyStats {
    pub total_rounds: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub uncertain: u32,
    pub skipped: u32,
    pub stage_failures: Vec<StageFailure>,
    pub hourly_failures: Vec<HourlyFailure>,
}

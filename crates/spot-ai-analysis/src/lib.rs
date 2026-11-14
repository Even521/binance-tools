//! 现货 AI 分析领域包：独立于桌面 UI 的数据模型、评分与提示词。
mod model;
mod prompt;
mod scoring;

pub use model::{AnalysisResult, DailyMaSnapshot, MarketSnapshot};
pub use prompt::build_report_prompt;
pub use scoring::{analyze_market, SCORE_WEIGHTS};

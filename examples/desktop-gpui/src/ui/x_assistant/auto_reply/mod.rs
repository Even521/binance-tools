//! 自动回复模块：独立维护自动回复主循环。
mod executor;

pub use executor::{run, run_with_first_cycle};

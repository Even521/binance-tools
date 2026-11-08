//! 自动发帖模块。只负责发帖业务，不直接拥有浏览器；由 core 调度执行。
pub mod content;
pub mod executor;
pub mod history;
pub mod submit;

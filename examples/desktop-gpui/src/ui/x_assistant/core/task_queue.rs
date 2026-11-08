//! 串行执行入口。任务执行期间持有全局浏览器租约，避免自动回复/发帖/评论并发操作页面。
use anyhow::Result;

pub fn run_browser_task<T>(task: impl FnOnce() -> Result<T>) -> Result<T> {
    let _browser_lease = super::browser::acquire_browser()?;
    task()
}

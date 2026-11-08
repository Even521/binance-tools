//! 浏览器操作互斥锁。所有会截图、点击、输入或滚动的任务应持有同一把锁。
use std::sync::{Mutex, MutexGuard, OnceLock};

static BROWSER_LEASE: OnceLock<Mutex<()>> = OnceLock::new();

pub fn acquire_browser() -> anyhow::Result<MutexGuard<'static, ()>> {
    BROWSER_LEASE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("X 浏览器操作锁已中毒，拒绝并发操作"))
}

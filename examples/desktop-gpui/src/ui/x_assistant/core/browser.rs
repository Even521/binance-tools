//! 浏览器操作互斥锁。所有会截图、点击、输入或滚动的任务应持有同一把锁。
use std::{
    sync::{Mutex, MutexGuard, OnceLock, TryLockError},
    thread,
    time::{Duration, Instant},
};

static BROWSER_LEASE: OnceLock<Mutex<()>> = OnceLock::new();
const BROWSER_LEASE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn acquire_browser() -> anyhow::Result<MutexGuard<'static, ()>> {
    let lock = BROWSER_LEASE.get_or_init(|| Mutex::new(()));
    let started = Instant::now();

    loop {
        match lock.try_lock() {
            Ok(guard) => {
                let waited = started.elapsed();
                if waited >= Duration::from_millis(250) {
                    super::super::automation::log_event(&format!(
                        "浏览器操作锁：等待 {:.2} 秒后获取成功。",
                        waited.as_secs_f64()
                    ));
                }
                return Ok(guard);
            }
            Err(TryLockError::WouldBlock) if started.elapsed() < BROWSER_LEASE_TIMEOUT => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(TryLockError::WouldBlock) => {
                let waited = started.elapsed();
                super::super::automation::log_event(&format!(
                    "浏览器操作锁：等待 {:.2} 秒仍未释放，终止本轮，避免页面无限卡住。",
                    waited.as_secs_f64()
                ));
                anyhow::bail!(
                    "等待 X 浏览器操作锁超过 {} 秒，可能有其他自动化任务未释放浏览器",
                    BROWSER_LEASE_TIMEOUT.as_secs()
                );
            }
            Err(TryLockError::Poisoned(_)) => {
                anyhow::bail!("X 浏览器操作锁已中毒，拒绝并发操作");
            }
        }
    }
}

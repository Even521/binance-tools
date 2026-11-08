//! 统一管理桌面应用的后台任务。
//!
//! 新增后台任务时优先放在本模块下，并通过 spawn_unique 调度，
//! 避免在业务 UI/自动化模块中散落线程创建逻辑。

use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Mutex, OnceLock},
    thread,
};

mod screenshot_cleanup;

static RUNNING_TASKS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

fn running_tasks() -> &'static Mutex<HashSet<&'static str>> {
    RUNNING_TASKS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 启动一个具名后台任务；相同名称的任务正在运行时拒绝重复启动。
/// 任务 panic 或线程创建失败时都会释放运行标记。
pub(crate) fn spawn_unique(
    name: &'static str,
    task: impl FnOnce() + Send + 'static,
) -> bool {
    {
        let mut running = running_tasks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !running.insert(name) {
            return false;
        }
    }

    let spawn_result = thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(task));
            if result.is_err() {
                app_log::error!("后台任务异常退出：{}", name);
            }
            let mut running = running_tasks()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            running.remove(name);
        });

    if let Err(error) = spawn_result {
        let mut running = running_tasks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        running.remove(name);
        app_log::error!("创建后台任务线程失败：{}，{}", name, error);
        return false;
    }

    true
}

/// 请求后台清理截图存储；调用方不会等待磁盘扫描或文件删除完成。
pub(crate) fn request_screenshot_cleanup() {
    screenshot_cleanup::request();
}

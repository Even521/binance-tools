//! X 助手截图存储清理任务。

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, SystemTime},
};

use super::spawn_unique;

const TASK_NAME: &str = "x-screenshot-cleanup";
const MAX_IMAGES: usize = 300;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const SCREENSHOT_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
// 新文件很可能仍在被 OCR 读取或刚刚写入，给它们留出保护窗口。
const ACTIVE_FILE_GRACE: Duration = Duration::from_secs(10 * 60);
// 自动发帖归档截图至少保留 1 小时，便于根据日志排查发帖问题。
const AUTO_POST_RETENTION: Duration = Duration::from_secs(60 * 60);

/// 每 25 次截图请求检查一次；首次请求会立即在后台启动清理。
pub(super) fn request() {
    static REQUEST_COUNT: AtomicUsize = AtomicUsize::new(0);
    if REQUEST_COUNT.fetch_add(1, Ordering::Relaxed) % 25 != 0 {
        return;
    }

    let _ = spawn_unique(TASK_NAME, cleanup);
}

fn cleanup() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data")
        .join("x_assistant");
    let screenshot_dir = root.join("screenshots");
    let auto_post_dir = root.join("auto_post");
    let now = SystemTime::now();
    // 每张图片分别记录最短保留时间和最长保存时间。
    let mut images = Vec::new();
    let mut removed_intermediates = 0usize;
    let mut directories = Vec::new();
    let mut failed_deletions = 0usize;

    fn collect_files(
        dir: &Path,
        now: SystemTime,
        min_retention: Duration,
        max_age: Duration,
        images: &mut Vec<(PathBuf, u64, SystemTime, Duration, Duration)>,
        removed_intermediates: &mut usize,
        directories: &mut Vec<PathBuf>,
    ) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path.clone());
                collect_files(
                    &path, now, min_retention, max_age, images, removed_intermediates, directories,
                );
                continue;
            }

            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let age = now.duration_since(modified).unwrap_or_default();

            if extension == "bmp" {
                // 只清理明显遗留的临时 BMP，避免碰到正在写入的截图。
                if age > ACTIVE_FILE_GRACE && fs::remove_file(&path).is_ok() {
                    *removed_intermediates += 1;
                }
            } else if matches!(extension.as_str(), "png" | "jpg" | "jpeg") {
                images.push((path, metadata.len(), modified, min_retention, max_age));
            }
        }
    }

    // 普通 OCR 截图保留 7 天；自动发帖过程截图保留 1 小时用于排错。
    collect_files(
        &screenshot_dir,
        now,
        ACTIVE_FILE_GRACE,
        SCREENSHOT_MAX_AGE,
        &mut images,
        &mut removed_intermediates,
        &mut directories,
    );
    collect_files(
        &auto_post_dir,
        now,
        AUTO_POST_RETENTION,
        AUTO_POST_RETENTION,
        &mut images,
        &mut removed_intermediates,
        &mut directories,
    );

    images.sort_by_key(|(_, _, modified, _, _)| *modified);
    let mut total_bytes: u64 = images.iter().map(|(_, size, _, _, _)| *size).sum();
    let mut remaining = images.len();
    let mut removed_images = 0usize;

    for (path, size, modified, min_retention, max_age) in images {
        let age = now.duration_since(modified).unwrap_or_default();
        // 保留窗口内的文件既不计入可删除候选，也不因超限被强制删除。
        if age <= min_retention {
            continue;
        }

        let expired = age > max_age;
        if !expired && remaining <= MAX_IMAGES && total_bytes <= MAX_BYTES {
            break;
        }

        match fs::remove_file(&path) {
            Ok(()) => {
                remaining = remaining.saturating_sub(1);
                total_bytes = total_bytes.saturating_sub(size);
                removed_images += 1;
                if (removed_images + removed_intermediates) % 10 == 0 {
                    thread::sleep(Duration::from_millis(10));
                }
            }
            Err(error) => {
                failed_deletions += 1;
                app_log::warn!("[截图清理] 删除图片失败：{}，原因：{}", path.display(), error);
            }
        }
    }

    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let mut removed_directories = 0usize;
    for dir in directories {
        // 新建轮次目录可能暂时为空；保护最近 10 分钟的目录，避免与截图写入竞争。
        let directory_age = fs::metadata(&dir)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or_default();
        if directory_age <= ACTIVE_FILE_GRACE {
            continue;
        }
        match fs::remove_dir(&dir) {
            Ok(()) => removed_directories += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                // 目录仍有受保护文件时属于正常情况，不视为清理失败。
                let still_non_empty = fs::read_dir(&dir)
                    .map(|mut entries| entries.next().is_some())
                    .unwrap_or(false);
                if !still_non_empty {
                    app_log::error!(
                        "[截图清理] 删除空目录失败：{}，原因：{}",
                        dir.display(),
                        error
                    );
                }
            }
        }
    }

    if removed_intermediates > 0 || removed_images > 0 || removed_directories > 0 || failed_deletions > 0 {
        app_log::info!(
            "[截图清理] 删除遗留 BMP {} 个、过期/超限图片 {} 个、空目录 {} 个；删除失败 {} 个；当前图片数约 {}，占用约 {:.1} MB（上限 {} 张 / {} MB；普通 OCR 截图保留 7 天，自动发帖截图至少保留 1 小时）",
            removed_intermediates,
            removed_images,
            removed_directories,
            failed_deletions,
            remaining,
            total_bytes as f64 / (1024.0 * 1024.0),
            MAX_IMAGES,
            MAX_BYTES / (1024 * 1024)
        );
    }
}

use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 北京时间固定时区（UTC+8），不依赖运行设备的系统时区。
pub fn beijing_timezone() -> FixedOffset {
    FixedOffset::east_opt(8 * 60 * 60).expect("UTC+8 is a valid fixed offset")
}

/// 获取当前北京时间。
pub fn beijing_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&beijing_timezone())
}

/// 将数据库中已保存的北京时间格式化显示。
/// 无时区的 SQLite 时间按北京时间原样显示；RFC 3339 时间按其时区转换为北京时间。
pub fn format_beijing_timestamp(value: &str) -> String {
    if let Ok(time) = DateTime::parse_from_rfc3339(value) {
        return time
            .with_timezone(&beijing_timezone())
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
    }

    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| value.to_string())
}

/// 将数据库常见的 UTC 时间字符串转换为北京时间显示。
/// 支持 RFC 3339 和 SQLite CURRENT_TIMESTAMP 格式；无法解析时原样返回。
pub fn format_utc_timestamp(value: &str) -> String {
    let parsed = DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
                .ok()
                .map(|time| time.and_utc())
        });

    parsed
        .map(|time| {
            time.with_timezone(&beijing_timezone())
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| value.to_string())
}

#[derive(Clone, Copy, Debug)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// 初始化日志目录。默认写入当前工作目录下的 logs/。
pub fn init() -> io::Result<PathBuf> {
    if let Some(path) = LOG_DIR.get() {
        return Ok(path.clone());
    }

    let path = std::env::current_dir()?.join("logs");
    fs::create_dir_all(&path)?;

    // 多线程同时初始化时，保留第一个成功设置的目录。
    let _ = LOG_DIR.set(path.clone());
    Ok(LOG_DIR.get().cloned().unwrap_or(path))
}

/// 返回当前日志目录。
pub fn log_dir() -> io::Result<PathBuf> {
    if let Some(path) = LOG_DIR.get() {
        return Ok(path.clone());
    }
    init()
}

/// 将日志追加到 logs/ 下指定文件。文件名只允许是单层文件名。
pub fn write_to(
    file_name: &str,
    level: Level,
    message: impl AsRef<str>,
) -> io::Result<()> {
    let file_path = Path::new(file_name);
    if file_name.is_empty()
        || file_path.file_name().and_then(|name| name.to_str()) != Some(file_name)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log file name must be a plain file name",
        ));
    }

    let directory = log_dir()?;
    fs::create_dir_all(&directory)?;

    let _guard = WRITE_LOCK
        .lock()
        .map_err(|_| io::Error::other("log writer lock poisoned"))?;

    let path = directory.join(file_name);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let timestamp = beijing_now().format("%Y-%m-%d %H:%M:%S%.3f");
    writeln!(file, "[{timestamp} +08:00] [{}] {}", level.as_str(), message.as_ref())?;
    file.flush()
}

/// 安装 panic hook，将 panic 信息和 backtrace 追加到 logs/crash.log。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let message = format!("{panic_info}\nBacktrace:\n{backtrace}");
        // 日志系统故障不能让 panic hook 再次 panic。
        let _ = write_to("crash.log", Level::Error, message);
    }));
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("app.log", $crate::Level::Debug, format!($($arg)*));
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("app.log", $crate::Level::Info, format!($($arg)*));
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("app.log", $crate::Level::Warn, format!($($arg)*));
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("app.log", $crate::Level::Error, format!($($arg)*));
    };
}

#[macro_export]
macro_rules! log_to {
    ($file:expr, $level:expr, $($arg:tt)*) => {
        let _ = $crate::write_to($file, $level, format!($($arg)*));
    };
}


/// X 助手专用日志入口，统一写入 logs/x-assistant.log。
#[macro_export]
macro_rules! x_debug {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("x-assistant.log", $crate::Level::Debug, format!("[X助手] {}", format!($($arg)*)));
    };
}

#[macro_export]
macro_rules! x_info {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("x-assistant.log", $crate::Level::Info, format!("[X助手] {}", format!($($arg)*)));
    };
}

#[macro_export]
macro_rules! x_warn {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("x-assistant.log", $crate::Level::Warn, format!("[X助手] {}", format!($($arg)*)));
    };
}

#[macro_export]
macro_rules! x_error {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("x-assistant.log", $crate::Level::Error, format!("[X助手] {}", format!($($arg)*)));
    };
}

/// 自动发帖日志统一写入 logs/auto-post.log。
#[macro_export]
macro_rules! post_debug {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("auto-post.log", $crate::Level::Debug, format!("[自动发帖] {}", format!($($arg)*)));
    };
}
#[macro_export]
macro_rules! post_info {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("auto-post.log", $crate::Level::Info, format!("[自动发帖] {}", format!($($arg)*)));
    };
}
#[macro_export]
macro_rules! post_warn {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("auto-post.log", $crate::Level::Warn, format!("[自动发帖] {}", format!($($arg)*)));
    };
}
#[macro_export]
macro_rules! post_error {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("auto-post.log", $crate::Level::Error, format!("[自动发帖] {}", format!($($arg)*)));
    };
}

/// 加密新闻与娱乐新闻 RSS 同步日志统一写入 logs/news-sync.log。
#[macro_export]
macro_rules! news_info {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("news-sync.log", $crate::Level::Info, format!("[新闻同步] {}", format!($($arg)*)));
    };
}
#[macro_export]
macro_rules! news_warn {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("news-sync.log", $crate::Level::Warn, format!("[新闻同步] {}", format!($($arg)*)));
    };
}
#[macro_export]
macro_rules! news_error {
    ($($arg:tt)*) => {
        let _ = $crate::write_to("news-sync.log", $crate::Level::Error, format!("[新闻同步] {}", format!($($arg)*)));
    };
}

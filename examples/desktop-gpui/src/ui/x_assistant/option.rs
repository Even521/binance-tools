use anyhow::{Context, Result};
use std::process::Command;

pub fn open_chrome() -> Result<()> {
    open_chrome_at("https://x.com/compose/post")
}

/// 自动回复启动时必须进入 X 首页，不能复用发帖页面启动地址。
pub fn open_chrome_home() -> Result<()> {
    open_chrome_at("https://x.com/home")
}

fn open_chrome_at(url: &str) -> Result<()> {
    Command::new("cmd")
        .args([
            "/C",
            "start",
            "",
            "chrome",
            "--new-window",
            url,
        ])
        .spawn()
        .context("启动 Chrome 失败")?;

    Ok(())
}

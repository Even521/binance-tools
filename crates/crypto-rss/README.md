# crypto-rss

一个只负责 RSS/Atom 采集的 Rust crate，适合放进 `binance-tools/crates/crypto-rss`。

## 功能

- 从 `feeds.json` 读取 RSS/Atom 源
- HTTP 请求和超时控制
- 解析标题、原文链接、摘要、发布时间
- 规范化 URL，移除常见追踪参数
- 单轮采集内按 URL 去重；没有 URL 时按标题哈希去重
- 将结果输出为 JSON Lines（每行一条 JSON）
- 单个源失败不会阻止其他源继续采集

## 运行

在 crate 目录执行：

```powershell
cargo run --release
```

指定配置和输出文件：

```powershell
cd D:\rust\binance-tools

cargo run -p crypto-rss -- .\crates\crypto-rss\feeds.json .\crates\crypto-rss\news.jsonl
```

## 放入现有工作区

把本目录复制到：

```text
D:\rust\binance-tools\crates\crypto-rss
```

在仓库根目录 `Cargo.toml` 的 `[workspace] members` 中加入：

```toml
"crates/crypto-rss",
```

如果根 `Cargo.toml` 使用 `members = ["crates/*"]`，则通常不需要额外添加。

从仓库根目录运行：

```powershell
cargo run -p crypto-rss
```

## 边界

本 crate 只做 RSS/Atom 采集与单轮去重，不负责 SQLite、推送、新闻评分、爬取网页正文、交易所公告 API 或链上监控。`feeds.json` 中的源地址应在运行环境逐个验证，网站可能更改订阅地址或限制请求。

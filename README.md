# Binance Tools

Binance Tools 是一个基于 **Rust + GPUI** 的桌面工具，包含市场榜单、日均线/K线图、币安广场任务和 AI 分析能力。

## 技术优势

- **Rust**：内存安全、性能稳定，适合长期运行的桌面工具和本地数据处理。
- **GPUI**：原生桌面 UI，启动快、交互流畅，界面体验接近现代编辑器。
- **本地优先**：AI 配置、主题配置和 SQLite 缓存都保存在本机，不随安装包分发用户数据。

## 本地启动

```bash
cargo run -p desktop-gpui
```

开发检查：

```bash
cargo check -p desktop-gpui
```

Release 构建：

```bash
cargo build -p desktop-gpui --release
```

## 文档

- [安装说明](./docs/安装/README.md)
- [Windows 安装与打包](./docs/安装/windows.md)

## 本地数据

程序首次启动会自动初始化本地配置和数据库：

- `config/ai.json`：AI 配置
- `config/setings.json`：主题配置
- `db/`：SQLite 本地缓存

安装包不会打包本机已有的 `config/` 和 `db/` 数据。

## 免责声明

AI 分析和市场数据仅用于学习与辅助观察，不构成投资建议。

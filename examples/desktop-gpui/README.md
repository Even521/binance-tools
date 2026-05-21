# GPUI 桌面示例

本示例是 workspace 成员，通过路径依赖引用仓库根 crate `binance_tools`，并固定使用 `gpui-component` / `gpui-component-assets` 的 `v0.5.1` tag 构建桌面入口。

## 目录

```text
src/
├── main.rs          # 薄入口，只调用 app::run()
├── app.rs           # GPUI Application、1024x768 启动窗口和 Root 初始化
├── theme.rs         # ThemeRegistry 加载主题目录与主题切换
└── ui/
    ├── mod.rs
    ├── dashboard.rs # 主界面内容区
    └── title_bar.rs # 窗体栏与 Theme 点击下拉菜单
themes/
├── ayu.json
├── catppuccin.json
└── ...
assets/
└── app.ico          # Windows exe / 安装包图标
build.rs            # Windows 资源嵌入脚本
```

主题参考 gpui-component 的 Theme 文档实现：`themes/` 下放置 gpui-component 官方主题 JSON，启动时由 `ThemeRegistry::watch_dir` 加载。窗体栏的 `Theme` 按钮点击后才弹出菜单，菜单项从 `ThemeRegistry::sorted_themes()` 生成，选中后调用 `Theme::global_mut(cx).apply_config(...)` 切换主题并刷新窗口。

## 运行

```powershell
cargo run -p desktop-gpui
```

## 构建和图标

Release 构建：

```powershell
cargo build -p desktop-gpui --release
```

Windows 下 `build.rs` 使用 `winresource` 把 `assets/app.ico` 嵌入 exe。需要替换应用图标时，保持文件名 `assets/app.ico` 不变并重新构建。

`Cargo.toml` 也包含 `[package.metadata.packager]`，后续使用 `cargo-packager` 打包安装文件时会复用同一图标。

当前界面包含市场榜单、现货币种、日均线信号、K 线图、币安广场任务/日志和右侧 AI Agent。业务逻辑优先沉淀到根 crate 的 `src/` 下，再由本桌面工程调用。

# Windows 安装与打包

本文说明如何在 Windows 上运行、构建和打包 `desktop-gpui` 桌面应用。

## 环境要求

| 工具 | 说明 |
| --- | --- |
| Windows 10/11 | 当前桌面应用主要在 Windows 环境验证 |
| Rust stable | 项目使用 Rust 2024 edition |
| MSVC Build Tools | Rust Windows MSVC 工具链需要 C++ build tools |
| PowerShell | 示例命令使用 PowerShell |

检查 Rust：

```powershell
rustc --version
cargo --version
```

## 本地运行

在项目根目录执行：

```powershell
cargo run -p desktop-gpui
```

桌面应用启动窗口默认尺寸为 `1024 x 768`，默认主题读取 `config/setings.json`，如果文件不存在会使用 `Ayu Light` 并自动创建。

## Release 构建

```powershell
cargo build -p desktop-gpui --release
```

生成文件：

```text
target\release\desktop-gpui.exe
```

Windows exe 图标来自：

```text
examples\desktop-gpui\assets\app.ico
```

该图标通过 `examples/desktop-gpui/build.rs` 使用 `winresource` 嵌入 exe。替换图标后需要重新执行 Release 构建。

## 配置和数据目录

桌面程序启动时会把工作目录切换到 `desktop-gpui.exe` 所在目录。因此传统 installer 选择安装到哪个目录，运行时配置和数据库就会写到哪个目录下：

| 路径 | 说明 |
| --- | --- |
| `config/ai.json` | AI Provider、模型和 Agent 配置 |
| `config/setings.json` | 用户主题选择 |
| `db/binance_tools.sqlite` | SQLite 数据库、缓存和发送日志 |

例如用户安装到：

```text
D:\Apps\Binance Tools
```

则运行时文件会在首次启动或用户保存配置时自动生成：

```text
D:\Apps\Binance Tools\config\ai.json
D:\Apps\Binance Tools\config\setings.json
D:\Apps\Binance Tools\db\binance_tools.sqlite
```

注意：如果用户选择 `C:\Program Files\Binance Tools`，普通用户可能没有写入权限，导致主题、AI 配置或 SQLite 数据保存失败。希望所有文件都留在安装目录时，建议默认安装到用户可写目录，例如 `{localappdata}\Binance Tools`，同时允许用户手动选择其它目录。

## 使用 cargo-packager

项目已在 `examples/desktop-gpui/Cargo.toml` 中配置：

```toml
[package.metadata.packager]
product-name = "Binance Tools"
identifier = "com.binance-tools.desktop"
icons = ["assets/app.ico"]
```

安装工具：

```powershell
cargo install cargo-packager
```

打包：

```powershell
cargo packager -p desktop-gpui --release
```

输出目录通常在：

```text
target\release\bundle\
```

## 使用 Inno Setup

如果需要传统 `.exe` 安装向导，可使用 Inno Setup。

项目提供安装脚本模板：

```text
installer/inno/binance-tools.iss
```

这个安装脚本只打包 `desktop-gpui.exe`，不会打包开发机本地的 `config/`、`db/` 或真实 API Key。首次启动时程序会自动初始化所需目录和文件。

示例核心配置：

```ini
[Setup]
AppName=Binance Tools
AppVersion=0.1.0
DefaultDirName={localappdata}\Binance Tools
DefaultGroupName=Binance Tools
OutputBaseFilename=BinanceToolsSetup
SetupIconFile=examples\desktop-gpui\assets\app.ico
Compression=lzma
SolidCompression=yes
DisableDirPage=no
DisableProgramGroupPage=no

[Files]
Source: "target\release\desktop-gpui.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Binance Tools"; Filename: "{app}\desktop-gpui.exe"; WorkingDir: "{app}"
Name: "{commondesktop}\Binance Tools"; Filename: "{app}\desktop-gpui.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\desktop-gpui.exe"; WorkingDir: "{app}"; Description: "启动 Binance Tools"; Flags: nowait postinstall skipifsilent
```

构建流程：

1. 执行 `cargo build -p desktop-gpui --release`。
2. 用 Inno Setup 打开 `installer/inno/binance-tools.iss`。
3. 点击 Compile 生成安装包。

生成的安装包默认输出到：

```text
target\installer\BinanceToolsSetup.exe
```

## 首次启动初始化

安装包不携带开发机本地配置和数据库。首次启动后按需生成：

| 文件 | 生成时机 |
| --- | --- |
| `config/setings.json` | 启动时如果不存在，会按默认主题 `Ayu Light` 创建 |
| `config/ai.json` | 用户在 AI Settings 中新增或修改 Provider 后保存 |
| `db/binance_tools.sqlite` | 页面首次访问市场、现货或币安广场数据时创建并自动迁移表结构 |

不要把开发机上的真实 `config/ai.json` 或 `db/binance_tools.sqlite` 打进安装包，避免泄露 API Key、历史消息和缓存数据。

## 常见问题

### 图标没有变化

Windows 会缓存 exe 图标。可以尝试：

- 重新执行 `cargo clean -p desktop-gpui` 后再构建。
- 改变 exe 文件名或安装目录。
- 重启资源管理器。

### 安装后配置无法保存

如果应用安装在 `Program Files`，普通用户可能没有写入权限。当前程序会把 `config/` 和 `db/` 放在 exe 所在目录，所以短期建议安装到用户可写目录，例如 `{localappdata}\Binance Tools` 或用户自定义的数据盘目录。

### 首次构建很慢

第一次构建会下载 `gpui`、`gpui-component`、`winresource` 等依赖。网络慢时可能出现 crates.io 超时警告，重试通常可以继续。

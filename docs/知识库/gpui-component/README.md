# GPUI Component 知识库

本目录整理 [**GPUI Component**](https://longbridge.github.io/gpui-component/docs/)（Longbridge 开源的 **Rust + GPUI** 桌面 UI 组件库）的本地资料，便于离线检索与和本项目对照阅读。

## 官方入口

| 说明 | 链接 |
| --- | --- |
| 文档站（英文） | [longbridge.github.io/gpui-component/docs/](https://longbridge.github.io/gpui-component/docs/) |
| 简体中文 | 站点语言切换为「简体中文」，或见本目录 [`官方文档链接索引.md`](./官方文档链接索引.md) 中的「简体」列 |
| 源码仓库 | [github.com/longbridge/gpui-component](https://github.com/longbridge/gpui-component) |
| 底层 UI 框架 GPUI | [gpui.rs](https://gpui.rs/) |
| 许可证 | Apache-2.0 |

## 本目录文件说明

| 文件 | 用途 |
| --- | --- |
| [`README.md`](./README.md) | 本说明（索引） |
| [`00-导读-安装与入门.md`](./00-导读-安装与入门.md) | 简介、环境、依赖、`init`、Root、无状态/有状态组件等**中文导读** |
| [`官方文档链接索引.md`](./官方文档链接索引.md) | 各文档页的 **English / 简体中文** 直达链接表 |
| [`llms-full.txt`](./llms-full.txt) | 从官网下载的**完整文档聚合**（约 3.6 万行，含各组件中英文 Markdown 片段与 `url` 元数据） |

## 如何使用 `llms-full.txt`

- 每个文档块以 YAML 头开始，例如：

```yaml
---
url: /gpui-component/docs/components/button.md
description: ...
---
```

- 正文为 Markdown，可在编辑器中全文搜索组件名（如 `Button`、`DataTable`）。
- 与线上一致时，可重新下载覆盖：  
  [https://longbridge.github.io/gpui-component/llms-full.txt](https://longbridge.github.io/gpui-component/llms-full.txt)

## 组件规模（官方描述摘要）

60+ 跨平台桌面组件；设计风格参考 macOS / Windows 与 [shadcn/ui](https://ui.shadcn.com/)；内置主题与尺寸档（如 `xs` / `sm` / `md` / `lg`）；支持 Dock / Tiles 等布局；虚拟化 Table / List；Markdown 与简单 HTML；图表与带 LSP 的编辑器等（详见官网与 `llms-full.txt`）。

## 社区

- [Issues](https://github.com/longbridge/gpui-component/issues)
- [Contributing](https://github.com/longbridge/gpui-component/blob/main/CONTRIBUTING.md)

上级目录见 [`../README.md`](../README.md)。

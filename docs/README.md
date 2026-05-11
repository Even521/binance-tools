# 文档（`docs`）

本目录存放与**源码解耦**的说明与知识库类 Markdown，便于单独浏览、检索与版本管理。仓库根目录下的 Rust 工程与 `docs/` 互不依赖，可按需只克隆或分享文档树。

## 子目录一览

| 路径 | 说明 |
| --- | --- |
| [`知识库/`](./知识库/README.md) | 知识库总索引（币安 API / SDK、GPUI Component 等） |
| [`项目执行/`](./项目执行/README.md) | 项目执行过程文档（里程碑、计划、发布与运维 Checklist、决策记录等） |

新增其他一级目录时，请在本表追加一行。

## 知识库内 README 与主入口

以下路径均在 `docs/知识库/` 之下（完整表格见 [`知识库/README.md`](./知识库/README.md)）。

| 主题 | 入口文件 |
| --- | --- |
| 知识库总览 | [`知识库/README.md`](./知识库/README.md) |
| 币安 API 资料区 | [`知识库/币安api/README.md`](./知识库/币安api/README.md) |
| 现货 REST 中文分卷 | [`知识库/币安api/spot-rest-api-CN/README.md`](./知识库/币安api/spot-rest-api-CN/README.md) |
| 币安官方 Rust SDK | [`知识库/币安sdk/README.md`](./知识库/币安sdk/README.md) |
| GPUI Component 文档库 | [`知识库/gpui-component/README.md`](./知识库/gpui-component/README.md) |
| 币安现货 API 外链总索引 | [`知识库/币安现货API.md`](./知识库/币安现货API.md) |
| REST 单文件占位说明 | [`知识库/rest-api_CN.md`](./知识库/rest-api_CN.md) |

## 浏览方式

- **Rust 工程布局**（`src` 公共库、`examples/` 独立示例子工程规划）：见 [`项目执行/01-项目框架.md`](./项目执行/01-项目框架.md) 与仓库根 [`../examples/README.md`](../examples/README.md)。
- 从本文件进入：打开 [`知识库/README.md`](./知识库/README.md) 或 [`项目执行/README.md`](./项目执行/README.md)，再按需进入子目录。
- 全文检索：GPUI 聚合文档为 [`知识库/gpui-component/llms-full.txt`](./知识库/gpui-component/llms-full.txt)；现货 REST 分卷为 `知识库/币安api/spot-rest-api-CN/*.md`。

从仓库根目录进入时，路径为 `docs/`。

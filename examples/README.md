# `examples/` — 示例 Rust 工程目录

本目录用于存放**独立的 Rust 子项目**（每个示例子目录内自有 `Cargo.toml`），用于演示如何依赖上级 crate `binance_tools`，而不是 Cargo 包自带的「单文件示例」（即不使用根目录 `cargo run --example <name>` 那种 `examples/*.rs` 形态）。

## 约定

- 每个示例：`examples/<名称>/`，内含完整 Cargo 工程。
- 子工程通过 **路径依赖** 引用库 crate，例如：

```toml
[dependencies]
binance_tools = { path = "../.." }
```

（具体路径随子目录深度调整。）

## 当前状态

**尚未初始化任何示例子工程**；需要时用 `cargo new examples/<名称>`（或等价方式）在本目录下创建即可。

上级说明见 [`../docs/项目执行/01-项目框架.md`](../docs/项目执行/01-项目框架.md)。

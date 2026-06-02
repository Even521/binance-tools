# 07 - Binance Alpha

> 本文记录 Binance Alpha 文档入库和本地 API 实现范围。

## 功能定位

Binance Alpha 模块用于查询 Alpha 公共市场数据。该业务线和现货 REST API 分开维护，知识库位于 `docs/知识库/币安api/alpha/`，代码入口位于 `src/binance/alpha.rs`。

## 当前能力

| 状态 | 功能 | 说明 |
| --- | --- | --- |
| ✓ | Alpha 知识库 | 已创建 `docs/知识库/币安api/alpha/` |
| ✓ | Alpha REST 文档 | 已整理 Token 列表、交易对信息、聚合交易、K 线、24hr ticker、完整深度 |
| ✓ | Alpha WebSocket 文档 | 已整理订阅协议和官方 stream 清单 |
| ✓ | Alpha REST 客户端 | `AlphaClient` 已实现 6 个公开市场数据 REST 接口 |
| ✓ | Alpha WebSocket 辅助 | 已实现订阅/取消订阅/查询订阅消息和 stream 名称构造 |
| ✓ | 单元测试 | 已覆盖 URL 构造、K 线数组解析和 WebSocket stream 名称 |
| ✓ | Alpha 菜单入口 | 顶部菜单栏新增 `Alpha`，包含 `Token列表`、`交易对信息`、`热力图` 和 `日均线信号` |
| ✓ | Alpha Token 页面 | 可查询并展示 Alpha Token 列表，支持 `积分+`（仅 `mulPoint` 为 2 或 4）、`证券代币`、链分类和关键字搜索，表头使用中文，代币名称后展示 `mulPoint` 倍率，并可点击 K 线图标进入 120 日 Alpha K 线图 |
| ✓ | Alpha 热力图 | 新增 Alpha 热力图页面，复用现货热力图排版；市值模式使用 `marketCap`，成交额模式使用 `volume24h`，支持链筛选、搜索和点击进入 Alpha K 线图 |
| ✓ | Alpha 交易对页面 | 支持搜索 Symbol/Base/Quote/状态、按计价资产切换，并按当前筛选结果统计上涨、下跌、持平数量 |
| ✓ | Alpha SQLite 缓存 | 新增 `alpha_tokens`、`alpha_assets`、`alpha_symbols`，页面优先读库，手动刷新才请求 Binance |
| ✓ | Alpha 日均线信号 | 新增 `alpha_klines` 缓存表和 Alpha 日均线页面，查询逻辑与现货一致：先读库、缺失时补日 K |
| ✓ | Alpha K 线图 | 复用现货 K 线图交互能力，Alpha 数据源读取 `alpha_klines`，Token 列表点击时按单个 Alpha USDT 交易对补齐 120 日日线；若本地缓存最新日线不是当天，会自动重新拉取补齐 |
| ✓ | 证券代币分类 | 同时满足 `stockState = true` 且符号/名称/CEX 名称带 `on` 后缀时归类为证券代币 |
| ✓ | Alpha 交易对页面 | 可查询并展示 Alpha exchange info 交易对 |

## 代码入口

| 层级 | 文件 | 职责 |
| --- | --- | --- |
| API | `src/binance/alpha.rs` | Alpha REST 调用、响应结构、WebSocket 消息和 stream 名称 |
| 模块导出 | `src/binance/mod.rs` | 暴露 `binance::alpha` |
| UI 菜单 | `examples/desktop-gpui/src/ui/title_bar.rs` | 顶部 `Alpha` 菜单 |
| UI 页面 | `examples/desktop-gpui/src/ui/alpha.rs` | Alpha Token 列表和交易对信息页面 |
| UI 页面 | `examples/desktop-gpui/src/ui/alpha_heatmap.rs` | Alpha 热力图页面，按市值或 24h 成交额占比展示涨跌色块 |
| UI 页面 | `examples/desktop-gpui/src/ui/alpha_ma_signal.rs` | Alpha 日均线信号页面 |
| UI 页面 | `examples/desktop-gpui/src/ui/kline.rs` | Spot/Alpha 1 日 K 线图，支持 MA7/MA25/MA99、缩放、拖拽和悬浮十字线 |
| 知识库 | `docs/知识库/币安api/alpha/` | 官方文档本地整理 |

## 接口清单

| 状态 | 接口 | 本地函数 |
| --- | --- | --- |
| ✓ | Token 列表 | `AlphaClient::token_list` |
| ✓ | 交易对信息 | `AlphaClient::exchange_info` |
| ✓ | 聚合交易 | `AlphaClient::aggregate_trades` |
| ✓ | K 线 | `AlphaClient::klines` |
| ✓ | 24hr 价格变动 | `AlphaClient::ticker_24hr` |
| ✓ | 完整深度 | `AlphaClient::full_depth` |

## 维护约定

- Alpha symbol 使用 `ALPHA_175USDT` 这类格式，不按普通现货 symbol 推断。
- 需要构造 Alpha symbol 时，先调用 Token 列表读取 `alphaId`。
- UI 层不要散落写死 Alpha URL，统一调用 `src/binance/alpha.rs`。

## Alpha 热力图页面

- 页面入口位于 `Alpha` 下拉菜单，菜单项为 `热力图`。
- 页面读取 `alpha_tokens` 本地缓存，空库时请求 Binance Alpha Token List 并落库。
- 支持按链筛选和关键字搜索；搜索范围包含 Alpha ID、Symbol、名称、链和合约地址。
- 方块颜色表示 `percentChange24h`：上涨绿色，下跌红色，持平灰色。
- 方块大小按当前筛选结果中的权重占比分级，权重可在 `市值` 和 `成交额` 间切换。
- `市值` 模式使用 `marketCap`，`成交额` 模式使用 `volume24h`。
- 方块显示使用 Token `symbol` 和 `name`，不把 `alphaId` 当作页面展示名称；`alphaId` 仅用于构造 Alpha K 线交易对。
- 默认最多展示前 100 个 Token，并隐藏当前筛选权重占比低于 0.25% 的小权重 Token。
- 热力图块使用显式分行布局，每行自动铺满可用宽度，避免留白。
- 点击任意方块会按 `alphaId + USDT` 打开 Alpha 1 日 K 线图，并自动补齐最新 120 日日线。

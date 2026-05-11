# 币安现货 API 知识库

> 整理日期：2026-05-11  
> **入口页面（你提供的官网链接）**：[币安 API 介绍（简体中文）](https://www.binance.com/zh-CN/binance-api)  
> 说明：官网该页多为导航与产品介绍，完整技术规格以 **开发者中心** 与 **官方开源文档仓库** 为准。

---

## 一、官方文档入口（建议收藏）

| 说明 | 链接 |
| --- | --- |
| 币安开放平台（中文现货文档总览） | [developers.binance.com · 现货 API 文档 README（zh-CN）](https://developers.binance.com/docs/zh-CN/binance-spot-api-docs/README) |
| 官方 GitHub 文档仓库（Markdown 源文件，与开放平台同步） | [github.com/binance/binance-spot-api-docs](https://github.com/binance/binance-spot-api-docs) |
| API 与数据流变更、停机等公告 | [Telegram: binance_api_announcements](https://t.me/binance_api_announcements) |

### 关于中文翻译版（官方声明）

- 中文文档由英文文档翻译而来，**当中文与英文冲突时，以英文文档为准**。
- 文档中已定义的接口、数据流、参数、响应等视为官方支持内容；**未在文档中出现的内容不承诺支持**。

---

## 二、本目录中的文件

| 文件 / 目录 | 内容 |
| --- | --- |
| **本文件**（`币安现货API.md`） | 索引、入口链接、配套文档导航 |
| **`rest-api_CN.md`** | 占位说明，指向下方分卷目录 |
| **`币安api/spot-rest-api-CN/`** | 现货 **REST** 中文说明（`/api`）按章节拆成 5 个文件，入口见 [`REST-API-CN-分卷索引.md`](./币安api/spot-rest-api-CN/REST-API-CN-分卷索引.md) |

分卷正文内的相对链接（如 `./errors_CN.md`、`faqs/...`）仍指向 GitHub 仓库中其他 Markdown。若本地未下载配套文件，请在线上对照：

- 浏览器：[开放平台 REST 文档目录](https://developers.binance.com/docs/zh-CN/binance-spot-api-docs/rest-api)
- 或仓库：[binance-spot-api-docs 根目录](https://github.com/binance/binance-spot-api-docs/tree/master)

---

## 三、现货 API 文档索引（中文）

以下表格与官方 `README_CN.md` 一致，链接指向 **GitHub 仓库内对应文件**（便于对照本地 `rest-api_CN.md`）。

### 3.1 核心文档

| 文档 | 描述 |
| --- | --- |
| [enums_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/enums_CN.md) | 适用于 REST 与 WebSocket API 的枚举定义 |
| [errors_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/errors_CN.md) | 现货 API 错误代码及含义 |
| [filters_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/filters_CN.md) | 现货 API 过滤器说明 |
| [spot-rest-api-CN 分卷](./币安api/spot-rest-api-CN/REST-API-CN-分卷索引.md) | 现货 REST API（`/api`），本地分卷 |
| [rest-api_CN.md（官方单文件）](https://github.com/binance/binance-spot-api-docs/blob/master/rest-api_CN.md) | 同上，GitHub 单文件版 |
| [fix-api_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/fix-api_CN.md) | 现货 FIX API |
| [web-socket-api_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/web-socket-api_CN.md) | 现货 WebSocket API |
| [web-socket-streams_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/web-socket-streams_CN.md) | 现货行情 WebSocket 数据流 |
| [sbe-market-data-streams_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/sbe-market-data-streams_CN.md) | SBE 市场数据流 |
| [user-data-stream_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/user-data-stream_CN.md) | 现货用户数据流 |
| [sbe/schemas](https://github.com/binance/binance-spot-api-docs/tree/master/sbe/schemas) | 现货 SBE Schema |
| [testnet](https://github.com/binance/binance-spot-api-docs/tree/master/testnet) | 仅现货测试网可用能力 |
| [demo-mode](https://github.com/binance/binance-spot-api-docs/tree/master/demo-mode) | 模拟交易说明 |

### 3.2 其他产品线（开放平台中文，非纯现货 `/api`）

| 文档 | 描述 |
| --- | --- |
| [杠杆 Margin](https://developers.binance.com/docs/zh-CN/margin_trading/Introduction) | 杠杆交易 |
| [U 本位合约 UM](https://developers.binance.com/docs/zh-CN/derivatives/usds-margined-futures/general-info) | `/fapi` |
| [币本位合约 CM](https://developers.binance.com/docs/zh-CN/derivatives/coin-margined-futures/general-info) | `/dapi` |
| [期权](https://developers.binance.com/docs/zh-CN/derivatives/option/general-info) | `/eapi` |
| [统一账户](https://developers.binance.com/docs/zh-CN/derivatives/portfolio-margin/general-info) | `/papi` |
| [钱包 Wallet](https://developers.binance.com/docs/zh-CN/wallet/Introduction) | `/sapi` |
| [子账户](https://developers.binance.com/docs/zh-CN/sub_account/Introduction) | `/sapi` |

（赚币、双币投资、定投、质押、矿池、策略、跟单、法币、C2C、借币、Pay、闪兑、返佣、NFT、礼品卡等见 [开放平台中文 README](https://developers.binance.com/docs/zh-CN/binance-spot-api-docs/README) 完整表格。）

### 3.3 常见问题（FAQ）

| 文档 | 描述 |
| --- | --- |
| [api_key_types_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/api_key_types_CN.md) | API 密钥类型 |
| [spot_glossary_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/spot_glossary_CN.md) | 现货 API 术语表 |
| [commission_faq_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/commission_faq_CN.md) | 佣金计算 |
| [trailing-stop-faq_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/trailing-stop-faq_CN.md) | 追踪止损 |
| [stp_faq_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/stp_faq_CN.md) | 自成交预防 STP |
| [market_orders_faq_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/market_orders_faq_CN.md) | 市价单行为 |
| [market_data_only_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/market_data_only_CN.md) | 仅市场数据 API / 流 |
| [sor_faq_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/sor_faq_CN.md) | 智能指令路由 SOR |
| [order_count_decrement_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/order_count_decrement_CN.md) | 下单次数限制更新 |
| [order_amend_keep_priority_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/order_amend_keep_priority_CN.md) | 改单保留优先级 |
| [pegged_orders_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/pegged_orders_CN.md) | 挂钩单 |
| [sbe_faq_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/sbe_faq_CN.md) | SBE 编码 |
| [price_range_execution_rules_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/price_range_execution_rules_CN.md) | 价格区间执行规则 |

### 3.4 更新日志

- [CHANGELOG_CN.md](https://github.com/binance/binance-spot-api-docs/blob/master/CHANGELOG_CN.md)

---

## 四、工具与连接器（官方推荐）

| 资源 | 链接 |
| --- | --- |
| Postman Collections | [binance-api-postman](https://github.com/binance/binance-api-postman) |
| OpenAPI / Swagger | [binance-api-swagger](https://github.com/binance/binance-api-swagger) |
| 现货测试网 | [testnet.binance.vision](https://testnet.binance.vision/)（仅 `/api/*`，不支持 `/sapi/*`） |
| Rust 连接器（官方） | [binance-connector-rust](https://github.com/binance/binance-connector-rust)（crate：`binance-sdk`）· [本地说明](./币安sdk/README.md) |

---

## 五、联系与支持

- [Binance API 中文电报群](https://t.me/binance_api_chinese) / [英文电报群](https://t.me/binance_api_english)
- [Binance 开发者论坛](https://dev.binance.vision/)
- [币安客服中心](https://www.binance.com/zh-CN/support-center)（账户、资金、2FA 等）

---

*本文索引依据 Binance 官方开源仓库 `binance-spot-api-docs` 的 `README_CN.md` 结构整理；`rest-api_CN.md` 为同期从该仓库 `master` 分支拉取的副本，若与线上最新版不一致，请以 GitHub 或开放平台为准。*

# btcc_wallets

BTCC 多钱包列表表，用于保存钱包管理页面的本地钱包元数据。

## 状态

- [✓] 【完成】 已在 `src/db/mod.rs` 创建表结构。
- [✓] 【完成】 已在 `src/db/btcc_wallet.rs` 提供列表、创建、更新、删除、余额更新方法。
- [✓] 【完成】 已接入 `BTCC -> 钱包列表` 页面。

## 表字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `INTEGER PRIMARY KEY AUTOINCREMENT` | 本地钱包记录 ID |
| `name` | `TEXT NOT NULL` | 钱包名称 |
| `address` | `TEXT NOT NULL UNIQUE` | BTCC 地址，当前校验 `cc1` 开头 |
| `network` | `TEXT NOT NULL` | 网络名称，默认 `Bitcoin-Classic (BTCC)` |
| `derivation_path` | `TEXT NOT NULL DEFAULT ''` | 派生路径，例如 `m/84'/0'/0'/0/0` |
| `source_type` | `TEXT NOT NULL` | 来源：`generated`、`mnemonic`、`wif`、`watch` |
| `public_key` | `TEXT NOT NULL DEFAULT ''` | 公钥，当前可为空 |
| `encrypted_mnemonic` | `BLOB` | 预留：加密后的助记词 |
| `encrypted_wif` | `BLOB` | 预留：加密后的 WIF 私钥 |
| `balance_sats` | `INTEGER NOT NULL DEFAULT 0` | 确认余额，单位 sats |
| `unconfirmed_sats` | `INTEGER NOT NULL DEFAULT 0` | 未确认余额，单位 sats |
| `utxo_count` | `INTEGER NOT NULL DEFAULT 0` | UTXO 数量 |
| `last_synced_at` | `TEXT` | 最近余额同步时间 |
| `note` | `TEXT NOT NULL DEFAULT ''` | 备注 |
| `is_active` | `INTEGER NOT NULL DEFAULT 1` | 软删除标记 |
| `created_at` | `TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP` | 创建时间 |
| `updated_at` | `TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP` | 更新时间 |

## 索引

| 索引 | 字段 | 说明 |
| --- | --- | --- |
| `idx_btcc_wallets_active_updated` | `is_active, updated_at DESC` | 钱包列表按有效状态和更新时间读取 |
| `idx_btcc_wallets_address` | `address` | 地址查询和去重 |

## 安全约定

- [✓] 【完成】 钱包列表页面默认只保存公开地址和元数据。
- [✓] 【完成】 助记词和私钥字段只预留加密存储列，未加密前不写入数据库。
- [✓] 【完成】 删除钱包使用 `is_active = 0` 软删除，避免误删历史元数据。
## 2026-06-11 单表加密约定

- [✓] 【完成】 BTCC 钱包只使用 `btcc_wallets` 一张表，不再新增钱包密钥表。
- [✓] 【完成】 数据库以后不保存明文助记词和明文 WIF 私钥。
- [✓] 【完成】 普通钱包记录的 `encrypted_mnemonic` 保存 `src/wallet/pbe.rs` 加密后的助记词，`encrypted_wif` 保存 `src/wallet/pbe.rs` 加密后的 WIF 私钥。
- [✓] 【完成】 钱包密码不少于 6 位，用 Argon2 校验；密码本身不保存。
- [✓] 【完成】 钱包密码校验值使用 `btcc_wallets` 内部记录保存，内部记录地址固定为 `__btcc_wallet_vault__`，列表查询会过滤该记录。
- [✓] 【完成】 创建钱包时必须输入钱包密码，验证助记词通过后才会加密写入数据库。
- [✓] 【完成】 导出钱包时必须再次输入钱包密码，解密结果只在当前页面临时显示。

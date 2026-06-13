//! BTCC wallet list persistence.

use crate::wallet::pbe::PBEWithHmacSha512AndAes256;
use anyhow::{Context, anyhow};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use rusqlite::{Connection, params};

const VAULT_ADDRESS: &str = "__btcc_wallet_vault__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtccWalletRecord {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub network: String,
    pub derivation_path: String,
    pub source_type: String,
    pub public_key: String,
    pub balance_sats: i64,
    pub unconfirmed_sats: i64,
    pub utxo_count: i64,
    pub last_synced_at: Option<String>,
    pub note: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtccWalletSecrets {
    pub mnemonic: String,
    pub private_key_wif: String,
}

pub fn list_btcc_wallets_blocking() -> anyhow::Result<Vec<BtccWalletRecord>> {
    let connection = super::open_default_connection()?;
    list_btcc_wallets(&connection)
}

pub fn btcc_wallet_password_exists_blocking() -> anyhow::Result<bool> {
    let connection = super::open_default_connection()?;
    btcc_wallet_password_exists(&connection)
}

pub fn create_btcc_wallet_password_blocking(password: String) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    create_btcc_wallet_password(&connection, &password)
}

pub fn verify_btcc_wallet_password_blocking(password: String) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    verify_btcc_wallet_password(&connection, &password)
}

pub fn create_btcc_wallet_blocking(
    name: String,
    address: String,
    derivation_path: String,
    source_type: String,
    public_key: String,
    note: String,
) -> anyhow::Result<i64> {
    let connection = super::open_default_connection()?;
    create_btcc_wallet(
        &connection,
        &name,
        &address,
        &derivation_path,
        &source_type,
        &public_key,
        &note,
    )
}

pub fn create_encrypted_btcc_wallet_blocking(
    name: String,
    address: String,
    derivation_path: String,
    source_type: String,
    public_key: String,
    note: String,
    mnemonic: String,
    private_key_wif: String,
    password: String,
) -> anyhow::Result<i64> {
    let connection = super::open_default_connection()?;
    create_encrypted_btcc_wallet(
        &connection,
        &name,
        &address,
        &derivation_path,
        &source_type,
        &public_key,
        &note,
        &mnemonic,
        &private_key_wif,
        &password,
    )
}

pub fn decrypt_btcc_wallet_secrets_blocking(
    wallet_id: i64,
    password: String,
) -> anyhow::Result<BtccWalletSecrets> {
    let connection = super::open_default_connection()?;
    decrypt_btcc_wallet_secrets(&connection, wallet_id, &password)
}

pub fn update_btcc_wallet_blocking(id: i64, name: String, note: String) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    update_btcc_wallet(&connection, id, &name, &note)
}

pub fn delete_btcc_wallet_blocking(id: i64) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    delete_btcc_wallet(&connection, id)
}

pub fn update_btcc_wallet_balance_blocking(
    id: i64,
    balance_sats: i64,
    unconfirmed_sats: i64,
    utxo_count: i64,
) -> anyhow::Result<()> {
    let connection = super::open_default_connection()?;
    update_btcc_wallet_balance(&connection, id, balance_sats, unconfirmed_sats, utxo_count)
}

pub fn list_btcc_wallets(connection: &Connection) -> anyhow::Result<Vec<BtccWalletRecord>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                id,
                name,
                address,
                network,
                derivation_path,
                source_type,
                public_key,
                balance_sats,
                unconfirmed_sats,
                utxo_count,
                last_synced_at,
                note,
                is_active,
                created_at,
                updated_at
            FROM btcc_wallets
            WHERE is_active = 1
              AND address <> ?1
            ORDER BY updated_at DESC, id DESC
            "#,
        )
        .context("prepare list btcc wallets failed")?;

    let records = statement
        .query_map(params![VAULT_ADDRESS], |row| {
            Ok(BtccWalletRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                address: row.get(2)?,
                network: row.get(3)?,
                derivation_path: row.get(4)?,
                source_type: row.get(5)?,
                public_key: row.get(6)?,
                balance_sats: row.get(7)?,
                unconfirmed_sats: row.get(8)?,
                utxo_count: row.get(9)?,
                last_synced_at: row.get(10)?,
                note: row.get(11)?,
                is_active: row.get::<_, i64>(12)? != 0,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
            })
        })
        .context("query btcc wallets failed")?
        .collect::<Result<Vec<_>, _>>()
        .context("read btcc wallets failed")?;

    Ok(records)
}

pub fn btcc_wallet_password_exists(connection: &Connection) -> anyhow::Result<bool> {
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(1) FROM btcc_wallets WHERE address = ?1",
            params![VAULT_ADDRESS],
            |row| row.get(0),
        )
        .context("query BTCC wallet password state failed")?;
    Ok(count > 0)
}

pub fn create_btcc_wallet_password(connection: &Connection, password: &str) -> anyhow::Result<()> {
    validate_password(password)?;
    anyhow::ensure!(
        !btcc_wallet_password_exists(connection)?,
        "BTCC wallet password already exists"
    );

    let password_hash = hash_password(password)?;

    connection
        .execute(
            r#"
            INSERT INTO btcc_wallets (
                name,
                address,
                network,
                derivation_path,
                source_type,
                public_key,
                note,
                is_active,
                updated_at
            )
            VALUES (
                '__vault__',
                ?1,
                'Bitcoin-Classic (BTCC)',
                '',
                'watch',
                ?2,
                'BTCC wallet password verifier and encryption salt',
                0,
                CURRENT_TIMESTAMP
            )
            "#,
            params![VAULT_ADDRESS, password_hash],
        )
        .context("create BTCC wallet password failed")?;

    Ok(())
}

pub fn verify_btcc_wallet_password(connection: &Connection, password: &str) -> anyhow::Result<()> {
    let password_hash: String = connection
        .query_row(
            r#"
            SELECT public_key
            FROM btcc_wallets
            WHERE address = ?1
            LIMIT 1
            "#,
            params![VAULT_ADDRESS],
            |row| row.get(0),
        )
        .context("BTCC wallet password is not initialized")?;

    let parsed_hash = PasswordHash::new(&password_hash)
        .map_err(|err| anyhow!("parse password hash failed: {err}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| anyhow!("BTCC wallet password is incorrect"))?;

    Ok(())
}

pub fn create_btcc_wallet(
    connection: &Connection,
    name: &str,
    address: &str,
    derivation_path: &str,
    source_type: &str,
    public_key: &str,
    note: &str,
) -> anyhow::Result<i64> {
    let name = name.trim();
    let address = address.trim();
    let derivation_path = derivation_path.trim();
    let source_type = source_type.trim();
    let public_key = public_key.trim();
    let note = note.trim();

    anyhow::ensure!(!name.is_empty(), "wallet name cannot be empty");
    anyhow::ensure!(!address.is_empty(), "wallet address cannot be empty");
    anyhow::ensure!(
        address.starts_with("cc1"),
        "BTCC address should start with cc1"
    );
    anyhow::ensure!(
        matches!(source_type, "generated" | "mnemonic" | "wif" | "watch"),
        "unsupported wallet source type"
    );

    connection
        .execute(
            r#"
            INSERT INTO btcc_wallets (
                name,
                address,
                derivation_path,
                source_type,
                public_key,
                note,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
            ON CONFLICT(address) DO UPDATE SET
                name = excluded.name,
                derivation_path = excluded.derivation_path,
                source_type = excluded.source_type,
                public_key = excluded.public_key,
                note = excluded.note,
                is_active = 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                name,
                address,
                derivation_path,
                source_type,
                public_key,
                note
            ],
        )
        .context("create btcc wallet failed")?;

    Ok(connection.last_insert_rowid())
}

pub fn create_encrypted_btcc_wallet(
    connection: &Connection,
    name: &str,
    address: &str,
    derivation_path: &str,
    source_type: &str,
    public_key: &str,
    note: &str,
    mnemonic: &str,
    private_key_wif: &str,
    password: &str,
) -> anyhow::Result<i64> {
    verify_btcc_wallet_password(connection, password)?;
    let pbe = PBEWithHmacSha512AndAes256::new(password);
    let encrypted_mnemonic = pbe.encrypt_str(mnemonic).into_bytes();
    let encrypted_wif = pbe.encrypt_str(private_key_wif).into_bytes();

    create_btcc_wallet(
        connection,
        name,
        address,
        derivation_path,
        source_type,
        public_key,
        note,
    )?;

    connection
        .execute(
            r#"
            UPDATE btcc_wallets
            SET encrypted_mnemonic = ?2,
                encrypted_wif = ?3,
                updated_at = CURRENT_TIMESTAMP
            WHERE address = ?1
            "#,
            params![address.trim(), encrypted_mnemonic, encrypted_wif],
        )
        .context("store encrypted BTCC wallet secrets failed")?;

    connection
        .query_row(
            "SELECT id FROM btcc_wallets WHERE address = ?1",
            params![address.trim()],
            |row| row.get(0),
        )
        .context("read encrypted BTCC wallet id failed")
}

pub fn decrypt_btcc_wallet_secrets(
    connection: &Connection,
    wallet_id: i64,
    password: &str,
) -> anyhow::Result<BtccWalletSecrets> {
    verify_btcc_wallet_password(connection, password)?;
    let (encrypted_mnemonic, encrypted_wif): (Option<Vec<u8>>, Option<Vec<u8>>) = connection
        .query_row(
            r#"
            SELECT encrypted_mnemonic, encrypted_wif
            FROM btcc_wallets
            WHERE id = ?1
              AND is_active = 1
              AND address <> ?2
            "#,
            params![wallet_id, VAULT_ADDRESS],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("read encrypted BTCC wallet secrets failed")?;

    let encrypted_mnemonic = encrypted_mnemonic.context("wallet mnemonic is not encrypted")?;
    let encrypted_wif = encrypted_wif.context("wallet private key is not encrypted")?;
    let encrypted_mnemonic =
        String::from_utf8(encrypted_mnemonic).context("wallet mnemonic ciphertext is not UTF-8")?;
    let encrypted_wif =
        String::from_utf8(encrypted_wif).context("wallet private key ciphertext is not UTF-8")?;
    let pbe = PBEWithHmacSha512AndAes256::new(password);

    Ok(BtccWalletSecrets {
        mnemonic: pbe.decrypt_to_string_result(&encrypted_mnemonic)?,
        private_key_wif: pbe.decrypt_to_string_result(&encrypted_wif)?,
    })
}

pub fn update_btcc_wallet(
    connection: &Connection,
    id: i64,
    name: &str,
    note: &str,
) -> anyhow::Result<()> {
    let name = name.trim();
    let note = note.trim();

    anyhow::ensure!(!name.is_empty(), "wallet name cannot be empty");

    connection
        .execute(
            r#"
            UPDATE btcc_wallets
            SET name = ?2,
                note = ?3,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![id, name, note],
        )
        .with_context(|| format!("update btcc wallet failed: {id}"))?;

    Ok(())
}

pub fn delete_btcc_wallet(connection: &Connection, id: i64) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            UPDATE btcc_wallets
            SET is_active = 0,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![id],
        )
        .with_context(|| format!("delete btcc wallet failed: {id}"))?;

    Ok(())
}

pub fn update_btcc_wallet_balance(
    connection: &Connection,
    id: i64,
    balance_sats: i64,
    unconfirmed_sats: i64,
    utxo_count: i64,
) -> anyhow::Result<()> {
    connection
        .execute(
            r#"
            UPDATE btcc_wallets
            SET balance_sats = ?2,
                unconfirmed_sats = ?3,
                utxo_count = ?4,
                last_synced_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![id, balance_sats, unconfirmed_sats, utxo_count],
        )
        .with_context(|| format!("update btcc wallet balance failed: {id}"))?;

    Ok(())
}

fn validate_password(password: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        password.chars().count() >= 6,
        "BTCC wallet password must be at least 6 characters"
    );
    Ok(())
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| anyhow!("hash BTCC wallet password failed: {err}"))
}

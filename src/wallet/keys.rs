use anyhow::{Context, anyhow};
use bech32::Hrp;
use bip39::{Language, Mnemonic};
use bitcoin::{
    CompressedPublicKey, Network, PrivateKey, PublicKey, WPubkeyHash,
    bip32::{DerivationPath, Xpriv},
    hashes::Hash,
    secp256k1::{Secp256k1, SecretKey},
};
use rand::{RngCore, rngs::OsRng};
use std::str::FromStr;

pub const BTCC_NATIVE_SEGWIT_PATH: &str = "m/84'/0'/0'/0/0";
const BTCC_BECH32_HRP: &str = "cc";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtccWallet {
    pub network: String,
    pub mnemonic: String,
    pub derivation_path: String,
    pub address: String,
    pub private_key_wif: String,
    pub public_key: PublicKey,
    pub secret_key: SecretKey,
}

pub type BitcoinWallet = BtccWallet;

pub fn generate_btcc_wallet() -> anyhow::Result<BtccWallet> {
    let mut entropy = [0u8; 16];
    OsRng.fill_bytes(&mut entropy);
    wallet_from_entropy(&entropy)
}

pub fn generate_bitcoin_wallet() -> anyhow::Result<BtccWallet> {
    generate_btcc_wallet()
}

pub fn wallet_from_mnemonic(mnemonic_str: &str) -> anyhow::Result<BtccWallet> {
    let mnemonic = Mnemonic::from_str(mnemonic_str).context("Invalid mnemonic phrase")?;

    if mnemonic.word_count() != 12 && mnemonic.word_count() != 24 {
        return Err(anyhow!("Mnemonic must be 12 or 24 words"));
    }

    wallet_from_mnemonic_value(&mnemonic)
}

pub fn wallet_from_private_key_wif(wif: &str) -> anyhow::Result<BtccWallet> {
    let private_key = PrivateKey::from_wif(wif).context("Invalid WIF private key")?;
    let secp = Secp256k1::new();
    let public_key = PublicKey::new(private_key.inner.public_key(&secp));
    let address = encode_btcc_address(&public_key)?;

    Ok(BtccWallet {
        network: "Bitcoin-Classic (BTCC)".to_string(),
        mnemonic: String::new(),
        derivation_path: "imported-wif".to_string(),
        address,
        private_key_wif: wif.trim().to_string(),
        public_key,
        secret_key: private_key.inner,
    })
}

fn wallet_from_entropy(entropy: &[u8]) -> anyhow::Result<BtccWallet> {
    let mnemonic =
        Mnemonic::from_entropy_in(Language::English, entropy).context("create mnemonic failed")?;
    wallet_from_mnemonic_value(&mnemonic)
}

fn wallet_from_mnemonic_value(mnemonic: &Mnemonic) -> anyhow::Result<BtccWallet> {
    let seed = mnemonic.to_seed("");
    let secp = Secp256k1::new();
    let master =
        Xpriv::new_master(Network::Bitcoin, &seed).context("create master xpriv failed")?;
    let path = DerivationPath::from_str(BTCC_NATIVE_SEGWIT_PATH)
        .context("parse derivation path failed")?;
    let child = master
        .derive_priv(&secp, &path)
        .context("derive private key failed")?;
    let private_key = PrivateKey::new(child.private_key, Network::Bitcoin);
    let public_key = PublicKey::new(child.private_key.public_key(&secp));
    let address = encode_btcc_address(&public_key)?;

    Ok(BtccWallet {
        network: "Bitcoin-Classic (BTCC)".to_string(),
        mnemonic: mnemonic.to_string(),
        derivation_path: BTCC_NATIVE_SEGWIT_PATH.to_string(),
        address,
        private_key_wif: private_key.to_wif(),
        public_key,
        secret_key: child.private_key,
    })
}

fn encode_btcc_address(public_key: &PublicKey) -> anyhow::Result<String> {
    let compressed = CompressedPublicKey::try_from(*public_key)
        .context("BTCC wallet requires compressed key")?;
    let hash = WPubkeyHash::hash(&compressed.to_bytes());

    bech32::segwit::encode(
        Hrp::parse(BTCC_BECH32_HRP)?,
        bech32::segwit::VERSION_0,
        hash.as_byte_array(),
    )
    .context("encode BTCC address failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_repeatable_btcc_wallet_from_mnemonic() {
        let wallet = wallet_from_entropy(&[7u8; 16]).unwrap();
        let imported = wallet_from_mnemonic(&wallet.mnemonic).unwrap();

        assert_eq!(wallet.address, imported.address);
        assert_eq!(wallet.private_key_wif, imported.private_key_wif);
        assert!(wallet.address.starts_with("cc1q"));
    }

    #[test]
    fn imports_wif_wallet() {
        let wallet = wallet_from_entropy(&[9u8; 16]).unwrap();
        let imported = wallet_from_private_key_wif(&wallet.private_key_wif).unwrap();

        assert_eq!(wallet.address, imported.address);
        assert_eq!("imported-wif", imported.derivation_path);
    }
}

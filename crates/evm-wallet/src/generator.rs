use crate::{
    VanityPattern,
    cuda::{self, CudaError},
};
use bip39::Mnemonic;
use bitcoin::{
    NetworkKind,
    bip32::{ChildNumber, DerivationPath, Xpriv, Xpub},
    secp256k1::{PublicKey, Secp256k1, SecretKey},
};
use sha3::{Digest, Keccak256};
use std::{error::Error, fmt, fmt::Write as _, ops::Deref, str::FromStr};
use zeroize::{Zeroize, Zeroizing};

const DERIVATIONS_PER_MNEMONIC: u32 = 1024;

pub struct WalletMaterial {
    pub address: String,
    pub mnemonic: String,
    pub private_key: String,
    pub derivation_path: String,
}

impl Drop for WalletMaterial {
    fn drop(&mut self) {
        self.mnemonic.zeroize();
        self.private_key.zeroize();
    }
}

pub enum SearchOutcome {
    Found(WalletMaterial),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchBackend {
    Cpu,
    Cuda,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchError {
    MnemonicGeneration,
    WalletDerivation,
    CudaUnavailable,
    CudaFailure(i32),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MnemonicGeneration => f.write_str("无法生成助记词"),
            Self::WalletDerivation => f.write_str("无法派生 Ethereum 钱包"),
            Self::CudaUnavailable => f.write_str("CUDA 后端不可用"),
            Self::CudaFailure(code) => write!(f, "CUDA 搜索失败（错误码 {code}）"),
        }
    }
}

impl Error for SearchError {}

/// Ensures a derived secp256k1 secret is erased when the BIP-32 node leaves scope.
struct ErasedXpriv(Xpriv);

impl Deref for ErasedXpriv {
    type Target = Xpriv;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for ErasedXpriv {
    fn drop(&mut self) {
        self.0.private_key.non_secure_erase();
    }
}

fn ethereum_address(secret_key: &SecretKey, secp: &Secp256k1<bitcoin::secp256k1::All>) -> String {
    let public_key = PublicKey::from_secret_key(secp, secret_key).serialize_uncompressed();
    let digest = Keccak256::digest(&public_key[1..]);
    let mut lowercase = String::with_capacity(40);
    for byte in &digest[12..] {
        let _ = write!(&mut lowercase, "{byte:02x}");
    }

    let checksum = Keccak256::digest(lowercase.as_bytes());
    let mut address = String::with_capacity(42);
    address.push_str("0x");
    for (index, character) in lowercase.bytes().enumerate() {
        let hash_nibble = if index % 2 == 0 {
            checksum[index / 2] >> 4
        } else {
            checksum[index / 2] & 0x0f
        };
        address.push(if character.is_ascii_alphabetic() && hash_nibble >= 8 {
            (character as char).to_ascii_uppercase()
        } else {
            character as char
        });
    }
    address
}

/// Searches derivation indexes under fresh BIP39 phrases until a match is found or cancelled.
/// Progress reports the number of candidate addresses checked so far.
pub fn search_vanity(
    pattern: VanityPattern,
    is_cancelled: impl FnMut() -> bool,
    on_progress: impl FnMut(u64),
) -> Result<SearchOutcome, SearchError> {
    search_vanity_with_backend(pattern, SearchBackend::Cpu, is_cancelled, on_progress)
}

pub fn search_vanity_with_backend(
    pattern: VanityPattern,
    backend: SearchBackend,
    mut is_cancelled: impl FnMut() -> bool,
    mut on_progress: impl FnMut(u64),
) -> Result<SearchOutcome, SearchError> {
    if backend == SearchBackend::Cuda && !cuda::is_cuda_available() {
        return Err(SearchError::CudaUnavailable);
    }

    let mut attempts = 0_u64;
    let secp = Secp256k1::new();
    let account_path =
        DerivationPath::from_str("m/44'/60'/0'/0").map_err(|_| SearchError::WalletDerivation)?;

    loop {
        if is_cancelled() {
            return Ok(SearchOutcome::Cancelled);
        }

        let mnemonic = Mnemonic::generate(12).map_err(|_| SearchError::MnemonicGeneration)?;
        let phrase = Zeroizing::new(mnemonic.to_string());
        let seed = Zeroizing::new(mnemonic.to_seed_normalized(""));
        let master = ErasedXpriv(
            Xpriv::new_master(NetworkKind::Main, seed.as_ref())
                .map_err(|_| SearchError::WalletDerivation)?,
        );
        let account = ErasedXpriv(
            master
                .derive_priv(&secp, &account_path)
                .map_err(|_| SearchError::WalletDerivation)?,
        );

        if backend == SearchBackend::Cuda {
            let xpub = Xpub::from_priv(&secp, &account.0);
            let serialized_parent = xpub.public_key.serialize_uncompressed();
            let parent_pubkey: [u8; 64] = serialized_parent[1..]
                .try_into()
                .map_err(|_| SearchError::WalletDerivation)?;
            let chain_code = *xpub.chain_code.as_bytes();
            let mut start_index = 0_u32;
            while start_index < 0x8000_0000 {
                if is_cancelled() {
                    return Ok(SearchOutcome::Cancelled);
                }
                let count = cuda::max_batch().min(0x8000_0000 - start_index);
                let found = cuda::search_xpub_batch(
                    &parent_pubkey,
                    &chain_code,
                    &pattern,
                    start_index,
                    count,
                )
                .map_err(|error| match error {
                    CudaError::Unavailable => SearchError::CudaUnavailable,
                    CudaError::Runtime(code) => SearchError::CudaFailure(code),
                })?;

                if let Some(index) = found {
                    if index < start_index || index >= start_index + count {
                        return Err(SearchError::CudaFailure(-4));
                    }
                    attempts = attempts.saturating_add((index - start_index + 1) as u64);
                    on_progress(attempts);
                    let child_number = ChildNumber::from_normal_idx(index)
                        .map_err(|_| SearchError::WalletDerivation)?;
                    let child = ErasedXpriv(
                        account
                            .derive_priv(&secp, &[child_number])
                            .map_err(|_| SearchError::WalletDerivation)?,
                    );
                    let address = ethereum_address(&child.private_key, &secp);
                    if !pattern.matches(&address) {
                        return Err(SearchError::CudaFailure(-5));
                    }
                    return Ok(SearchOutcome::Found(wallet_material(
                        address, &phrase, child, index,
                    )));
                }
                attempts = attempts.saturating_add(count as u64);
                on_progress(attempts);
                start_index += count;
            }
            continue;
        }

        for index in 0..DERIVATIONS_PER_MNEMONIC {
            if is_cancelled() {
                return Ok(SearchOutcome::Cancelled);
            }

            let child_number =
                ChildNumber::from_normal_idx(index).map_err(|_| SearchError::WalletDerivation)?;
            let child = ErasedXpriv(
                account
                    .derive_priv(&secp, &[child_number])
                    .map_err(|_| SearchError::WalletDerivation)?,
            );
            attempts = attempts.saturating_add(1);
            on_progress(attempts);

            let address = ethereum_address(&child.private_key, &secp);
            if pattern.matches(&address) {
                return Ok(SearchOutcome::Found(wallet_material(
                    address, &phrase, child, index,
                )));
            }
        }
    }
}

fn wallet_material(
    address: String,
    phrase: &str,
    child: ErasedXpriv,
    index: u32,
) -> WalletMaterial {
    let derivation_path = format!("m/44'/60'/0'/0/{index}");
    let private_key_bytes = Zeroizing::new(child.private_key.secret_bytes());
    let mut private_key = String::with_capacity(64);
    for byte in private_key_bytes.iter() {
        let _ = write!(&mut private_key, "{byte:02x}");
    }
    WalletMaterial {
        address,
        mnemonic: phrase.to_string(),
        private_key,
        derivation_path,
    }
}

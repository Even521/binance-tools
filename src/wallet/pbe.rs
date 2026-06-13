//! PBKDF2-HMAC-SHA512 派生密钥 + AES-256-CBC（与 `poly_market::encryptor::standard_pbe` 一致）。

use aes::Aes256;
use anyhow;
use base64::Engine as _;
use base64::engine::general_purpose;
use cbc::{Decryptor, Encryptor};
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha512;

/// 口令基于 PBE 的 AES-256-CBC 加解密（迭代次数固定为 **1000**，与 poly_market 相同）。
pub struct PBEWithHmacSha512AndAes256 {
    password: Vec<u8>,
}

impl PBEWithHmacSha512AndAes256 {
    const ITERATIONS: u32 = 1000;

    #[must_use]
    pub fn new(password: &str) -> Self {
        Self {
            password: password.as_bytes().to_vec(),
        }
    }

    /// AES-256-CBC + PKCS7，返回 Base64(**salt ∥ iv ∥ ciphertext**)。
    pub fn encrypt_str(&self, plaintext: &str) -> String {
        self.encrypt(plaintext.as_bytes())
    }

    pub fn decrypt_to_string(&self, encrypted_base64: &str) -> String {
        let decrypted_bytes = self.decrypt(encrypted_base64);
        String::from_utf8(decrypted_bytes).expect("UTF-8 decode error")
    }

    pub fn decrypt_to_string_result(
        &self,
        encrypted_base64: &str,
    ) -> std::result::Result<String, anyhow::Error> {
        let decrypted_bytes = self.decrypt_result(encrypted_base64)?;
        String::from_utf8(decrypted_bytes).map_err(|e| anyhow::anyhow!("私钥解密后非 UTF-8: {}", e))
    }

    pub fn decrypt_result(
        &self,
        encrypted_base64: &str,
    ) -> std::result::Result<Vec<u8>, anyhow::Error> {
        let bytes = general_purpose::STANDARD
            .decode(encrypted_base64.trim())
            .map_err(|e| anyhow::anyhow!("Base64 解码失败: {}", e))?;
        if bytes.len() < 32 {
            anyhow::bail!("密文过短（需包含 salt+iv）");
        }
        let (salt, rest) = bytes.split_at(16);
        let (iv, ciphertext) = rest.split_at(16);
        let mut key = [0u8; 32];
        pbkdf2_hmac::<Sha512>(&self.password, salt, Self::ITERATIONS, &mut key);
        let cipher = Decryptor::<Aes256>::new_from_slices(&key, iv)
            .map_err(|e| anyhow::anyhow!("解密器创建失败: {}", e))?;
        let mut buffer = ciphertext.to_vec();
        let decrypted_slice = cipher
            .decrypt_padded_mut::<Pkcs7>(&mut buffer)
            .map_err(|e| anyhow::anyhow!("解密或去填充失败: {}", e))?;
        Ok(decrypted_slice.to_vec())
    }

    fn encrypt(&self, plaintext: &[u8]) -> String {
        let mut salt = [0u8; 16];
        let mut iv = [0u8; 16];
        let mut rng = rand::rngs::OsRng;
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut iv);

        let mut key = [0u8; 32];
        pbkdf2_hmac::<Sha512>(&self.password, &salt, Self::ITERATIONS, &mut key);

        let cipher =
            Encryptor::<Aes256>::new_from_slices(&key, &iv).expect("invalid key or iv length");

        let block_size = 16;
        let padded_len = ((plaintext.len() / block_size) + 1) * block_size;
        let mut buffer = vec![0u8; padded_len];
        buffer[..plaintext.len()].copy_from_slice(plaintext);

        let ciphertext_slice = cipher
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
            .expect("encryption failed");

        let mut result = Vec::with_capacity(16 + 16 + ciphertext_slice.len());
        result.extend_from_slice(&salt);
        result.extend_from_slice(&iv);
        result.extend_from_slice(ciphertext_slice);

        general_purpose::STANDARD.encode(result)
    }

    fn decrypt(&self, encrypted_base64: &str) -> Vec<u8> {
        let bytes = general_purpose::STANDARD
            .decode(encrypted_base64)
            .expect("base64 decode failed");

        let (salt, rest) = bytes.split_at(16);
        let (iv, ciphertext) = rest.split_at(16);

        let mut key = [0u8; 32];
        pbkdf2_hmac::<Sha512>(&self.password, salt, Self::ITERATIONS, &mut key);

        let cipher =
            Decryptor::<Aes256>::new_from_slices(&key, iv).expect("invalid key or iv length");

        let mut buffer = ciphertext.to_vec();
        let decrypted_slice = cipher
            .decrypt_padded_mut::<Pkcs7>(&mut buffer)
            .expect("decryption/unpad failed");

        decrypted_slice.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::PBEWithHmacSha512AndAes256;
    use crate::common::client::local_signer_from_private_key_hex;

    const TEST_PASSWORD: &str = "poly-market-v2-test-password";
    const TEST_PRIVATE_KEY: &str =
        "0x1111111111111111111111111111111111111111111111111111111111111111";

    /// 功能：验证 PBE 加密后可解密回原始测试私钥，并打印密文、解密结果和派生地址。
    /// 参数：无，使用固定公开测试私钥，不读取 `.env` 或真实账户配置。
    /// 返回：断言加解密结果与地址派生成功。
    /// 边界：`encrypt_str` 使用随机 salt/iv，每次运行打印的密文都会不同。
    #[test]
    fn encrypt_decrypt_private_key_and_print_address() {
        let pbe = PBEWithHmacSha512AndAes256::new(TEST_PASSWORD);

        let encrypted = pbe.encrypt_str(TEST_PRIVATE_KEY);
        let decrypted = pbe.decrypt_to_string_result(&encrypted).unwrap();
        let signer = local_signer_from_private_key_hex(&decrypted).unwrap();
        let address = signer.address();

        println!("test encrypted private key: {encrypted}");
        println!("test decrypted private key: {decrypted}");
        println!("test private key address: {address:?}");

        assert_eq!(decrypted, TEST_PRIVATE_KEY);
        assert_eq!(
            format!("{address:?}"),
            "0x19e7e376e7c213b7e7e7e46cc70a5dd086daff2a"
        );
    }
}

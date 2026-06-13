use crate::wallet::{BtccUtxo, BtccWallet};
use anyhow::{Context, anyhow};
use bitcoin::{
    Amount, OutPoint, PublicKey, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    absolute,
    consensus::encode::serialize_hex,
    hashes::Hash,
    secp256k1::{Message, Secp256k1},
    sighash::{EcdsaSighashType, SighashCache},
    transaction,
};
use std::str::FromStr;

const BTCC_BECH32_HRP: &str = "cc";
const DUST_SATS: u64 = 546;

#[derive(Clone, Debug)]
pub struct BtccSendRequest {
    pub to_address: String,
    pub amount_sats: u64,
    pub fee_rate_sat_vb: u64,
}

#[derive(Clone, Debug)]
pub struct BtccSignedTransaction {
    pub rawtx: String,
    pub total_input_sats: u64,
    pub send_sats: u64,
    pub change_sats: u64,
    pub fee_sats: u64,
    pub input_count: usize,
}

pub fn build_signed_transaction(
    wallet: &BtccWallet,
    utxos: &[BtccUtxo],
    request: &BtccSendRequest,
) -> anyhow::Result<BtccSignedTransaction> {
    if request.amount_sats == 0 {
        return Err(anyhow!("send amount must be greater than 0"));
    }
    if request.fee_rate_sat_vb == 0 {
        return Err(anyhow!("fee rate must be greater than 0"));
    }

    let recipient_script = btcc_address_script(&request.to_address)?;
    let change_script = p2wpkh_script(&wallet.public_key)?;
    let mut selected = Vec::new();
    let mut selected_total = 0u64;
    let mut fee_sats = 0u64;

    for utxo in utxos {
        selected_total = selected_total.saturating_add(utxo.value);
        selected.push(utxo.clone());
        fee_sats = estimate_fee_sats(selected.len(), 2, request.fee_rate_sat_vb);
        if selected_total >= request.amount_sats.saturating_add(fee_sats) {
            break;
        }
    }

    if selected_total < request.amount_sats.saturating_add(fee_sats) {
        return Err(anyhow!("insufficient balance for amount plus fee"));
    }

    let mut change_sats = selected_total - request.amount_sats - fee_sats;
    let output_count = if change_sats >= DUST_SATS { 2 } else { 1 };
    fee_sats = estimate_fee_sats(selected.len(), output_count, request.fee_rate_sat_vb);

    if selected_total < request.amount_sats.saturating_add(fee_sats) {
        return Err(anyhow!("insufficient balance after fee recalculation"));
    }

    change_sats = selected_total - request.amount_sats - fee_sats;
    if change_sats < DUST_SATS {
        fee_sats = fee_sats.saturating_add(change_sats);
        change_sats = 0;
    }

    let inputs = selected
        .iter()
        .map(|utxo| {
            let txid = Txid::from_str(&utxo.tx_hash).context("invalid UTXO txid")?;
            Ok(TxIn {
                previous_output: OutPoint {
                    txid,
                    vout: utxo.tx_pos,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut outputs = vec![TxOut {
        value: Amount::from_sat(request.amount_sats),
        script_pubkey: recipient_script,
    }];

    if change_sats > 0 {
        outputs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: change_script.clone(),
        });
    }

    let mut tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let secp = Secp256k1::new();
    let sighash_type = EcdsaSighashType::All;
    let mut sighasher = SighashCache::new(&mut tx);

    for (input_index, utxo) in selected.iter().enumerate() {
        let sighash = sighasher
            .p2wpkh_signature_hash(
                input_index,
                &change_script,
                Amount::from_sat(utxo.value),
                sighash_type,
            )
            .context("create p2wpkh sighash failed")?;
        let message = Message::from(sighash);
        let signature = secp.sign_ecdsa(&message, &wallet.secret_key);
        let signature = bitcoin::ecdsa::Signature {
            signature,
            sighash_type,
        };
        *sighasher
            .witness_mut(input_index)
            .context("missing transaction witness")? =
            Witness::p2wpkh(&signature, &wallet.secret_key.public_key(&secp));
    }

    let signed_tx = sighasher.into_transaction();

    Ok(BtccSignedTransaction {
        rawtx: serialize_hex(&signed_tx),
        total_input_sats: selected_total,
        send_sats: request.amount_sats,
        change_sats,
        fee_sats,
        input_count: selected.len(),
    })
}

pub fn btcc_to_sats(value: &str) -> anyhow::Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("amount is empty"));
    }

    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let whole_sats = whole
        .parse::<u64>()
        .context("invalid amount integer part")?
        .checked_mul(100_000_000)
        .ok_or_else(|| anyhow!("amount is too large"))?;

    let mut fraction = fraction.to_string();
    if fraction.len() > 8 {
        return Err(anyhow!("BTCC amount supports at most 8 decimals"));
    }
    while fraction.len() < 8 {
        fraction.push('0');
    }
    let fraction_sats = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()
            .context("invalid amount decimal part")?
    };

    whole_sats
        .checked_add(fraction_sats)
        .ok_or_else(|| anyhow!("amount is too large"))
}

fn estimate_fee_sats(input_count: usize, output_count: usize, fee_rate_sat_vb: u64) -> u64 {
    let vbytes = 10 + input_count as u64 * 68 + output_count as u64 * 31;
    vbytes.saturating_mul(fee_rate_sat_vb)
}

fn p2wpkh_script(public_key: &PublicKey) -> anyhow::Result<ScriptBuf> {
    let wpkh = public_key
        .wpubkey_hash()
        .context("compressed public key required")?;
    Ok(ScriptBuf::new_p2wpkh(&wpkh))
}

fn btcc_address_script(address: &str) -> anyhow::Result<ScriptBuf> {
    let (hrp, version, program) =
        bech32::segwit::decode(address).context("invalid BTCC bech32 address")?;
    if hrp.as_str() != BTCC_BECH32_HRP {
        return Err(anyhow!("recipient address must use cc bech32 prefix"));
    }
    if version != bech32::segwit::VERSION_0 || program.len() != 20 {
        return Err(anyhow!("only BTCC p2wpkh cc1 addresses are supported"));
    }

    let hash = bitcoin::WPubkeyHash::from_slice(&program).context("invalid witness program")?;
    Ok(ScriptBuf::new_p2wpkh(&hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::generate_btcc_wallet;

    #[test]
    fn parses_btcc_amount_to_sats() {
        assert_eq!(btcc_to_sats("1").unwrap(), 100_000_000);
        assert_eq!(btcc_to_sats("0.00000001").unwrap(), 1);
        assert_eq!(btcc_to_sats("1.25").unwrap(), 125_000_000);
    }

    #[test]
    fn builds_signed_transaction_hex() {
        let wallet = generate_btcc_wallet().unwrap();
        let utxo = BtccUtxo {
            tx_hash: "0000000000000000000000000000000000000000000000000000000000000001".to_string(),
            tx_pos: 0,
            height: 1,
            value: 100_000,
        };
        let request = BtccSendRequest {
            to_address: wallet.address.clone(),
            amount_sats: 50_000,
            fee_rate_sat_vb: 2,
        };

        let signed = build_signed_transaction(&wallet, &[utxo], &request).unwrap();

        assert!(signed.rawtx.starts_with("02000000"));
        assert_eq!(signed.input_count, 1);
        assert!(signed.fee_sats > 0);
    }
}

use anyhow::Context;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_BTCC_EXPLORER_API: &str = "https://explorer.btc-classic.org/api/v1";

#[derive(Clone, Debug)]
pub struct BtccExplorerClient {
    base_url: String,
    http: Client,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BtccAddressInfo {
    pub address: String,
    #[serde(rename = "type")]
    pub address_type: Option<String>,
    pub confirmed_sats: u64,
    pub confirmed_btcc: f64,
    pub unconfirmed_sats: i64,
    pub unconfirmed_btcc: f64,
    pub total_sats: i64,
    pub total_btcc: f64,
    pub utxos: Vec<BtccUtxo>,
    pub utxo_total: usize,
    pub utxo_has_more: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BtccUtxo {
    pub tx_hash: String,
    pub tx_pos: u32,
    pub height: i64,
    pub value: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BtccBroadcastResult {
    pub txid: String,
}

#[derive(Serialize)]
struct BroadcastBody<'a> {
    rawtx: &'a str,
}

impl Default for BtccExplorerClient {
    fn default() -> Self {
        Self::new(DEFAULT_BTCC_EXPLORER_API)
    }
}

impl BtccExplorerClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("create HTTP client");

        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn address_info(&self, address: &str) -> anyhow::Result<BtccAddressInfo> {
        let url = format!(
            "{}/explorer/address/{}?include_history=false&utxo_limit=100&utxo_offset=0",
            self.base_url,
            url_escape(address)
        );
        self.http
            .get(url)
            .send()
            .context("BTCC explorer address request failed")?
            .error_for_status()
            .context("BTCC explorer address response failed")?
            .json()
            .context("parse BTCC address response failed")
    }

    pub fn broadcast_raw_transaction(&self, rawtx: &str) -> anyhow::Result<BtccBroadcastResult> {
        let url = format!("{}/tx/broadcast", self.base_url);
        self.http
            .post(url)
            .json(&BroadcastBody {
                rawtx: rawtx.trim(),
            })
            .send()
            .context("BTCC broadcast request failed")?
            .error_for_status()
            .context("BTCC broadcast response failed")?
            .json()
            .context("parse BTCC broadcast response failed")
    }
}

fn url_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![ch],
            _ => format!("%{:02X}", ch as u32).chars().collect(),
        })
        .collect()
}

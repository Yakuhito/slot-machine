use chia::protocol::Bytes32;
use reqwest::Client;
use serde::Deserialize;
use std::error::Error;

use super::utils::{hex_string_to_bytes32, hex_string_to_bytes32_maybe};

#[derive(Debug)]
pub struct ChiaRpcClient {
    base_url: String,
    client: Client,
}

#[derive(Deserialize, Debug)]
pub struct BlockchainStateResponse {
    pub blockchain_state: Option<BlockchainState>,
    pub success: bool,
}

#[derive(Deserialize, Debug)]
pub struct BlockchainState {
    pub average_block_time: u64,
    pub block_max_cost: u64,
    pub difficulty: u64,
    pub genesis_challenge_initialized: bool,
    pub mempool_cost: u64,
    pub mempool_fees: u64,
    pub mempool_max_total_cost: u64,
    pub mempool_min_fees: MempoolMinFees,
    pub mempool_size: u32,
    #[serde(with = "hex_string_to_bytes32")]
    pub node_id: Bytes32,
    pub peak: DeserializableBlockRecord,
    pub space: u64,
    pub sub_slot_iters: u64,
    pub sync: Sync,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MempoolMinFees {
    pub cost_5000000: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DeserializableBlockRecord {
    #[serde(with = "hex_string_to_bytes32")]
    pub header_hash: Bytes32,
    #[serde(with = "hex_string_to_bytes32")]
    pub prev_hash: Bytes32,
    pub height: u32,
    pub weight: u128,
    pub total_iters: u128,
    pub signage_point_index: u8,
    #[serde(with = "hex_string_to_bytes32")]
    pub farmer_puzzle_hash: Bytes32,
    pub required_iters: u64,
    pub deficit: u8,
    pub overflow: bool,
    pub prev_transaction_block_height: u32,
    pub timestamp: Option<u64>,
    #[serde(with = "hex_string_to_bytes32_maybe")]
    pub prev_transaction_block_hash: Option<Bytes32>,
    pub fees: Option<u64>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Sync {
    pub sync_mode: bool,
    pub sync_progress_height: u32,
    pub sync_tip_height: u32,
    pub synced: bool,
}

impl ChiaRpcClient {
    pub fn new(base_url: &str) -> Self {
        ChiaRpcClient {
            base_url: base_url.to_string(),
            client: Client::new(),
        }
    }

    pub fn coinset_testnet11() -> Self {
        Self::new("https://testnet11.api.coinset.org")
    }

    pub fn coinset_mainnet() -> Self {
        Self::new("https://api.coinset.org")
    }

    pub async fn get_blockchain_state(&self) -> Result<BlockchainStateResponse, Box<dyn Error>> {
        let url = format!("{}/get_blockchain_state", self.base_url);
        let res = self
            .client
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await?;

        let response = res.json::<BlockchainStateResponse>().await?;
        Ok(response)
    }
}

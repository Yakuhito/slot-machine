use chia::protocol::Bytes32;
use serde::Deserialize;

use super::utils::{hex_string_to_bytes32, hex_string_to_bytes32_maybe};

#[derive(Deserialize, Debug)]
pub struct BlockchainStateResponse {
    pub blockchain_state: Option<BlockchainState>,
    pub error: Option<String>,
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
    pub space: u128,
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

#[derive(Deserialize, Debug)]
pub struct AdditionsAndRemovalsResponse {
    pub additions: Option<Vec<DeserializableCoinRecord>>,
    pub removals: Option<Vec<DeserializableCoinRecord>>,
    pub error: Option<String>,
    pub success: bool,
}

#[derive(Deserialize, Debug)]
pub struct DeserializableCoinRecord {
    pub coin: DeserializableCoin,
    pub coinbase: bool,
    pub confirmed_block_index: u32,
    pub spent: bool,
    pub spent_block_index: u32,
    pub timestamp: u64,
}

#[derive(Deserialize, Debug)]
pub struct DeserializableCoin {
    pub amount: u64,
    #[serde(with = "hex_string_to_bytes32")]
    pub parent_coin_info: Bytes32,
    #[serde(with = "hex_string_to_bytes32")]
    pub puzzle_hash: Bytes32,
}

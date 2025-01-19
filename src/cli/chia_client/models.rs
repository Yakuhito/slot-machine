use chia::protocol::{BlockRecord, Bytes32, Coin, FullBlock};
use serde::Deserialize;

use super::de::{
    deserialize_block_record, deserialize_block_record_maybe, deserialize_coin,
    deserialize_full_block_maybe, hex_string_to_bytes32,
};

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
    #[serde(with = "deserialize_block_record")]
    pub peak: BlockRecord,
    pub space: u128,
    pub sub_slot_iters: u64,
    pub sync: Sync,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MempoolMinFees {
    pub cost_5000000: u64,
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
    pub additions: Option<Vec<CoinRecord>>,
    pub removals: Option<Vec<CoinRecord>>,
    pub error: Option<String>,
    pub success: bool,
}

#[derive(Deserialize, Debug)]
pub struct CoinRecord {
    #[serde(with = "deserialize_coin")]
    pub coin: Coin,
    pub coinbase: bool,
    pub confirmed_block_index: u32,
    pub spent: bool,
    pub spent_block_index: u32,
    pub timestamp: u64,
}

#[derive(Deserialize, Debug)]
pub struct GetBlockResponse {
    #[serde(with = "deserialize_full_block_maybe")]
    pub block: Option<FullBlock>,
    pub error: Option<String>,
    pub success: bool,
}

#[derive(Deserialize, Debug)]
pub struct GetBlockRecordResponse {
    #[serde(with = "deserialize_block_record_maybe")]
    pub block_record: Option<BlockRecord>,
    pub error: Option<String>,
    pub success: bool,
}

use chia_protocol::Bytes32;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Public NFT details embedded in the common singleton shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingletonNftDetails {
    pub metadata_treehash: String,
    pub metadata_updater_puzzle_hash: String,
    pub current_owner: Option<String>,
    pub royalty_puzzle_hash: String,
    pub royalty_basis_points: u16,
    pub p2_puzzle_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

/// Common public singleton shape shared by `GET /singletons/{launcher_id}` and later proof reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingletonResponse {
    pub launcher_id: String,
    pub parent_coin_id: String,
    pub amount: u64,
    pub inner_puzzle_hash: String,
    pub confirmation_height: u32,
    pub melted: bool,
    pub melt_height: Option<u32>,
    pub nft: Option<SingletonNftDetails>,
    pub indexed_peak_height: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SingletonQuery {
    #[serde(default)]
    pub include_metadata: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

pub fn hex32(id: Bytes32) -> String {
    hex::encode(id.to_bytes())
}

pub fn parse_launcher_id(raw: &str) -> Result<Bytes32, ()> {
    let raw = raw.strip_prefix("0x").unwrap_or(raw);
    if raw.len() != 64 {
        return Err(());
    }
    let bytes = hex::decode(raw).map_err(|_| ())?;
    if bytes.len() != 32 {
        return Err(());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(Bytes32::new(arr))
}

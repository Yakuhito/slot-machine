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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HandleQuery {
    #[serde(default)]
    pub include_metadata: bool,
    /// Explicit registry selection; omission selects the first configured registry.
    pub launcher_id: Option<String>,
    #[serde(default)]
    pub bypass_expiration_safety_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotNeighborsJson {
    pub left_value: String,
    pub right_value: String,
}

/// Complete latest Handle-slot value (protocol-fixed amount 0 remains implicit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandleSlotJson {
    pub counter: u64,
    pub handle_hash: String,
    pub neighbors: SlotNeighborsJson,
    pub expiration: u64,
    pub owner_launcher_id: String,
    pub resolved_launcher_id: String,
}

/// Indivisible unified Handle proof returned by `GET /handle/{handle}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandleProofResponse {
    pub registry_launcher_id: String,
    pub handle: String,
    pub slot: HandleSlotJson,
    pub slot_parent_coin_id: String,
    pub slot_confirmation_height: u32,
    pub resolved_singleton: SingletonResponse,
    pub indexed_peak_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistrationQuery {
    /// Explicit registry selection; omission selects the first configured registry.
    pub launcher_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecentRegistrationsQuery {
    pub launcher_id: Option<String>,
    /// Newest-first page size; capped at 50. Omitted defaults to 50.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationResponse {
    pub handle: String,
    pub registration_secret: String,
    pub action_kind: String,
    pub protocol_fee: u64,
    pub confirmation_height: u32,
    pub indexed_peak_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRegistrationItem {
    pub handle: String,
    pub action_kind: String,
    pub confirmation_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRegistrationsResponse {
    pub items: Vec<RecentRegistrationItem>,
    pub total_registered: u64,
    pub indexed_peak_height: u32,
}

/// Canonical Handle grammar: 3–63 lowercase ASCII alphanumeric, no normalization.
pub fn is_canonical_handle(handle: &str) -> bool {
    let len = handle.len();
    (3..=63).contains(&len)
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
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

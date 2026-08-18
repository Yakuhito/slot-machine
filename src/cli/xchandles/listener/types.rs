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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PendingTransferQuery {
    /// Explicit registry selection; omission selects the first configured registry.
    pub launcher_id: Option<String>,
}

/// Performable pending transfer returned by `GET /handle/{handle}/pending-transfer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingTransferResponse {
    pub handle_hash: String,
    pub new_owner_launcher_id: String,
    pub new_resolved_launcher_id: String,
    pub update_confirmation_height: u32,
    pub minimum_execution_height: u32,
    pub update_initiator_coin_id: String,
    pub current_executor_coin_id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExpiringQuery {
    pub view: Option<String>,
    pub cursor: Option<String>,
    /// Page size; capped at 50. Omitted defaults to 50.
    pub limit: Option<u32>,
    /// Explicit registry selection; omission selects the first configured registry.
    pub launcher_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiringView {
    Active,
    Soon,
}

impl ExpiringView {
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw {
            Some("active") => Some(Self::Active),
            Some("soon") => Some(Self::Soon),
            _ => None,
        }
    }
}

/// Active expiration-auction row for `GET /expiring?view=active`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiringActiveItem {
    pub handle: String,
    pub expiration: u64,
    pub projected_pricing_timestamp: u64,
    pub current_premium: u64,
    pub total_registration_fee: u64,
    pub base_registration_fee: u64,
    pub reaches_base_at: u64,
}

/// Expiring-soon row for `GET /expiring?view=soon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiringSoonItem {
    pub handle: String,
    pub expiration: u64,
    pub base_registration_fee: u64,
}

/// Cursor-paginated active-auction directory response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiringActiveResponse {
    pub items: Vec<ExpiringActiveItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub indexed_peak_height: u32,
    /// Latest confirmed transaction-block timestamp used for pricing.
    pub confirmed_timestamp: u64,
}

/// Cursor-paginated expiring-soon directory response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiringSoonResponse {
    pub items: Vec<ExpiringSoonItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub indexed_peak_height: u32,
    pub confirmed_timestamp: u64,
}

/// Canonical Handle grammar: 3-63 lowercase ASCII alphanumeric, no normalization.
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

pub fn parse_launcher_id(raw: &str) -> Option<Bytes32> {
    let raw = raw.strip_prefix("0x").unwrap_or(raw);
    if raw.len() != 64 {
        return None;
    }
    let bytes = hex::decode(raw).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(Bytes32::new(arr))
}

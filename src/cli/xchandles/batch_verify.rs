//! Premine batch set-equality verification, finality gate, and identical-retry.
//!
//! Public seams:
//! - [`compare_premine_set`] — full-set equality of security-relevant fields
//! - [`finality_reached`] / [`reorganization_invalidates_finality`] — 32-block gate
//! - [`decide_batch_retry`] — reuse identical spend while inputs remain unspent
//! - [`expected_observations_from_bundle_rows`] — bundle → expected observations
//! - [`emit_pre_broadcast_plan`] — machine-readable plan before irreversible spends
//! - [`reject_legacy_premine_bytes`] — refuse legacy media-column Premine shapes

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use chia_protocol::{Bytes32, CoinSpend, SpendBundle};
use serde::{Deserialize, Serialize};

use crate::{
    build_pre_broadcast_plan, CliError, PremineLaunchBundle, PremineLaunchBundleRow,
    OWNER_RESOLVED_RELATIONSHIP,
};

/// Canonical blocks that must be built on top of a batch confirmation before finality.
pub const PREMINE_FINALITY_DEPTH: u32 = 32;

pub const PENDING_BATCH_SPEND_FORMAT: &str = "xchandles-pending-batch-spend";
pub const PENDING_BATCH_SPEND_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationPhase {
    Canonical,
    Final,
}

/// One expected Premine Handle after mint+register, as committed by the bundle
/// plus the discovered NFT launcher identity for that Handle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedPremineObservation {
    pub handle: String,
    pub recipient_puzzle_hash: String,
    pub expiration: u64,
    pub owner_resolved_relationship: String,
    pub owner_launcher_id: String,
    pub resolved_launcher_id: String,
    pub nft_launcher_id: String,
    pub display_name: String,
    pub image_uri: String,
    pub image_hash: String,
    pub metadata_uri: String,
    pub metadata_hash: String,
    pub license_uri: String,
    pub license_hash: String,
    pub handle_nft_metadata_clvm_hex: String,
    pub updater_hash: String,
    pub royalty_puzzle_hash: String,
    pub royalty_basis_points: u16,
    pub batch_id: u32,
    pub row_index: u32,
}

/// Observed on-chain Premine Handle state reconstructed from registry + NFT.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservedPremineHandle {
    pub handle: String,
    pub recipient_puzzle_hash: String,
    pub expiration: u64,
    pub owner_resolved_relationship: String,
    pub owner_launcher_id: String,
    pub resolved_launcher_id: String,
    pub nft_launcher_id: String,
    pub display_name: String,
    pub image_uri: String,
    pub image_hash: String,
    pub metadata_uri: String,
    pub metadata_hash: String,
    pub license_uri: String,
    pub license_hash: String,
    pub handle_nft_metadata_clvm_hex: String,
    pub updater_hash: String,
    pub royalty_puzzle_hash: String,
    pub royalty_basis_points: u16,
    pub batch_id: u32,
    pub row_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PremineFieldDiff {
    pub handle: String,
    pub field: String,
    pub expected: String,
    pub observed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PremineVerificationReport {
    pub ok: bool,
    pub phase: VerificationPhase,
    pub expected_count: usize,
    pub observed_count: usize,
    pub missing_handles: Vec<String>,
    pub extra_handles: Vec<String>,
    /// Handles that appeared more than once in `expected` (map collapse risk).
    pub duplicate_expected_handles: Vec<String>,
    /// Handles that appeared more than once in `observed` (map collapse risk).
    pub duplicate_observed_handles: Vec<String>,
    pub field_diffs: Vec<PremineFieldDiff>,
}

impl PremineVerificationReport {
    pub fn to_machine_readable_json(&self) -> Result<String, CliError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| CliError::Custom(format!("verification report serialize failed: {e}")))
    }

    /// Fail closed: any mismatch prevents later batches.
    pub fn gate_later_batches(&self) -> Result<(), CliError> {
        if self.ok {
            return Ok(());
        }
        let json = self.to_machine_readable_json()?;
        Err(CliError::Custom(format!(
            "premine batch verification failed ({:?}); later batches blocked:\n{json}",
            self.phase
        )))
    }
}

/// Build expected observations for the given bundle rows, attaching NFT launcher
/// IDs discovered on-chain (keyed by normalized Handle). Missing launcher IDs
/// become empty strings so set comparison can report missing Handles.
pub fn expected_observations_from_bundle_rows(
    rows: &[PremineLaunchBundleRow],
    nft_launcher_ids_by_handle: &BTreeMap<String, String>,
) -> Result<Vec<ExpectedPremineObservation>, CliError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let nft_launcher_id = nft_launcher_ids_by_handle
            .get(&row.handle)
            .cloned()
            .unwrap_or_default();
        out.push(ExpectedPremineObservation {
            handle: row.handle.clone(),
            recipient_puzzle_hash: row.recipient_puzzle_hash.clone(),
            expiration: row.expiration,
            owner_resolved_relationship: row.owner_resolved_relationship.clone(),
            owner_launcher_id: nft_launcher_id.clone(),
            resolved_launcher_id: nft_launcher_id.clone(),
            nft_launcher_id,
            display_name: row.display_name.clone(),
            image_uri: row.image_uri.clone(),
            image_hash: row.image_hash.clone(),
            metadata_uri: row.metadata_uri.clone(),
            metadata_hash: row.metadata_hash.clone(),
            license_uri: row.license_uri.clone(),
            license_hash: row.license_hash.clone(),
            handle_nft_metadata_clvm_hex: row.handle_nft_metadata_clvm_hex.clone(),
            updater_hash: row.updater_hash.clone(),
            royalty_puzzle_hash: row.royalty_puzzle_hash.clone(),
            royalty_basis_points: row.royalty_basis_points,
            batch_id: row.batch_id,
            row_index: row.row_index,
        });
    }
    Ok(out)
}

/// Index observations by Handle while recording duplicate keys that would
/// otherwise be silently collapsed by a map.
fn index_unique_by_handle<'a, T, F>(
    rows: &'a [T],
    handle_of: F,
) -> (BTreeMap<&'a str, &'a T>, Vec<String>)
where
    F: Fn(&T) -> &str,
{
    let mut by_handle = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for row in rows {
        let handle = handle_of(row);
        if by_handle.insert(handle, row).is_some() {
            duplicates.insert(handle.to_string());
        }
    }
    (by_handle, duplicates.into_iter().collect())
}

/// Compare complete set equality of security-relevant Premine fields.
///
/// Enforces cardinality: duplicate Handles in either side fail closed, so a
/// later matching row cannot hide an earlier modified observation.
pub fn compare_premine_set(
    expected: &[ExpectedPremineObservation],
    observed: &[ObservedPremineHandle],
    phase: VerificationPhase,
) -> PremineVerificationReport {
    let (expected_by_handle, duplicate_expected_handles) =
        index_unique_by_handle(expected, |e| e.handle.as_str());
    let (observed_by_handle, duplicate_observed_handles) =
        index_unique_by_handle(observed, |o| o.handle.as_str());

    let expected_handles: BTreeSet<&str> = expected_by_handle.keys().copied().collect();
    let observed_handles: BTreeSet<&str> = observed_by_handle.keys().copied().collect();

    let missing_handles: Vec<String> = expected_handles
        .difference(&observed_handles)
        .map(|h| (*h).to_string())
        .collect();
    let extra_handles: Vec<String> = observed_handles
        .difference(&expected_handles)
        .map(|h| (*h).to_string())
        .collect();

    let mut field_diffs = Vec::new();
    for handle in expected_handles.intersection(&observed_handles) {
        let exp = expected_by_handle[handle];
        let obs = observed_by_handle[handle];
        push_diff(
            &mut field_diffs,
            handle,
            "recipient_puzzle_hash",
            &exp.recipient_puzzle_hash,
            &obs.recipient_puzzle_hash,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "expiration",
            &exp.expiration.to_string(),
            &obs.expiration.to_string(),
        );
        push_diff(
            &mut field_diffs,
            handle,
            "owner_resolved_relationship",
            &exp.owner_resolved_relationship,
            &obs.owner_resolved_relationship,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "owner_launcher_id",
            &exp.owner_launcher_id,
            &obs.owner_launcher_id,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "resolved_launcher_id",
            &exp.resolved_launcher_id,
            &obs.resolved_launcher_id,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "nft_launcher_id",
            &exp.nft_launcher_id,
            &obs.nft_launcher_id,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "display_name",
            &exp.display_name,
            &obs.display_name,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "image_uri",
            &exp.image_uri,
            &obs.image_uri,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "image_hash",
            &exp.image_hash,
            &obs.image_hash,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "metadata_uri",
            &exp.metadata_uri,
            &obs.metadata_uri,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "metadata_hash",
            &exp.metadata_hash,
            &obs.metadata_hash,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "license_uri",
            &exp.license_uri,
            &obs.license_uri,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "license_hash",
            &exp.license_hash,
            &obs.license_hash,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "handle_nft_metadata_clvm_hex",
            &exp.handle_nft_metadata_clvm_hex,
            &obs.handle_nft_metadata_clvm_hex,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "updater_hash",
            &exp.updater_hash,
            &obs.updater_hash,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "royalty_puzzle_hash",
            &exp.royalty_puzzle_hash,
            &obs.royalty_puzzle_hash,
        );
        push_diff(
            &mut field_diffs,
            handle,
            "royalty_basis_points",
            &exp.royalty_basis_points.to_string(),
            &obs.royalty_basis_points.to_string(),
        );
        push_diff(
            &mut field_diffs,
            handle,
            "batch_id",
            &exp.batch_id.to_string(),
            &obs.batch_id.to_string(),
        );
        push_diff(
            &mut field_diffs,
            handle,
            "row_index",
            &exp.row_index.to_string(),
            &obs.row_index.to_string(),
        );

        // Owner/Resolved must be the same dedicated Handle NFT.
        if obs.owner_launcher_id != obs.resolved_launcher_id
            || obs.owner_launcher_id != obs.nft_launcher_id
            || obs.owner_resolved_relationship != OWNER_RESOLVED_RELATIONSHIP
        {
            push_diff(
                &mut field_diffs,
                handle,
                "launcher_relationship",
                &format!(
                    "owner=resolved=nft ({OWNER_RESOLVED_RELATIONSHIP})"
                ),
                &format!(
                    "owner={} resolved={} nft={} relationship={}",
                    obs.owner_launcher_id,
                    obs.resolved_launcher_id,
                    obs.nft_launcher_id,
                    obs.owner_resolved_relationship
                ),
            );
        }
    }

    // Deduplicate launcher_relationship diffs that also hit exact field compares.
    field_diffs.sort_by(|a, b| (&a.handle, &a.field).cmp(&(&b.handle, &b.field)));
    field_diffs.dedup();

    let cardinality_ok = duplicate_expected_handles.is_empty()
        && duplicate_observed_handles.is_empty()
        && expected.len() == observed.len()
        && expected.len() == expected_by_handle.len()
        && observed.len() == observed_by_handle.len();

    let ok = cardinality_ok
        && missing_handles.is_empty()
        && extra_handles.is_empty()
        && field_diffs.is_empty();
    PremineVerificationReport {
        ok,
        phase,
        expected_count: expected.len(),
        observed_count: observed.len(),
        missing_handles,
        extra_handles,
        duplicate_expected_handles,
        duplicate_observed_handles,
        field_diffs,
    }
}

fn push_diff(
    diffs: &mut Vec<PremineFieldDiff>,
    handle: &str,
    field: &str,
    expected: &str,
    observed: &str,
) {
    if expected != observed {
        diffs.push(PremineFieldDiff {
            handle: handle.to_string(),
            field: field.to_string(),
            expected: expected.to_string(),
            observed: observed.to_string(),
        });
    }
}

/// True when `peak_height` has at least [`PREMINE_FINALITY_DEPTH`] canonical
/// blocks built on top of `confirmation_height`.
pub fn finality_reached(confirmation_height: u32, peak_height: u32) -> bool {
    peak_height.saturating_sub(confirmation_height) >= PREMINE_FINALITY_DEPTH
}

/// A reorganization that drops below the confirmation height (or reassigns it)
/// invalidates finality until the batch is re-confirmed and re-finalized.
pub fn reorganization_invalidates_finality(
    original_confirmation_height: u32,
    current_confirmation_height: Option<u32>,
    peak_height: u32,
) -> bool {
    match current_confirmation_height {
        None => true,
        Some(h) if h != original_confirmation_height => true,
        Some(h) => !finality_reached(h, peak_height),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputCoinState {
    Unspent,
    /// Spent by the pending spend we intend to reuse.
    SpentByPending,
    /// Spent by a conflicting or unknown spend.
    SpentConflicting,
}

/// Classify one pending-spend input from its on-chain spent flag and the
/// optional on-chain [`CoinSpend`]. A spent coin is [`SpentByPending`] only when
/// the on-chain puzzle+solution equal the pending bundle's spend for that coin.
pub fn classify_pending_input_state(
    record_present: bool,
    spent: bool,
    pending_coin_spend: Option<&CoinSpend>,
    on_chain_coin_spend: Option<&CoinSpend>,
) -> InputCoinState {
    if !record_present {
        return InputCoinState::SpentConflicting;
    }
    if !spent {
        return InputCoinState::Unspent;
    }
    match (pending_coin_spend, on_chain_coin_spend) {
        (Some(pending), Some(on_chain)) if pending == on_chain => InputCoinState::SpentByPending,
        _ => InputCoinState::SpentConflicting,
    }
}

/// Every coin spent by `sb` — the complete identical-retry input set.
pub fn spend_bundle_input_coin_ids(sb: &SpendBundle) -> Vec<Bytes32> {
    sb.coin_spends.iter().map(|cs| cs.coin.coin_id()).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingBatchSpendRecord {
    pub format: String,
    pub version: u32,
    pub registry_launcher_id: String,
    pub batch_id: u32,
    pub phase: String,
    pub handles: Vec<String>,
    pub input_coin_ids: Vec<String>,
    pub spend_bundle: SpendBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchRetryConflictReport {
    pub batch_id: u32,
    pub phase: String,
    pub handles: Vec<String>,
    pub conflicting_input_coin_ids: Vec<String>,
    pub observed_states: Vec<(String, InputCoinState)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchRetryDecision {
    /// No pending record — construct a fresh spend for this batch.
    ConstructFresh,
    /// All relevant inputs remain unspent — resubmit the identical spend.
    ReuseIdentical(PendingBatchSpendRecord),
    /// Pending spend already landed (inputs spent by it) — treat as done.
    AlreadyApplied(PendingBatchSpendRecord),
    /// A spent/conflicting input stops blind retry.
    Conflict(BatchRetryConflictReport),
}

/// Decide whether a retry must reuse the identical spend, report a conflict,
/// or construct fresh when no pending record exists.
pub fn decide_batch_retry(
    pending: Option<&PendingBatchSpendRecord>,
    input_states: &BTreeMap<String, InputCoinState>,
) -> BatchRetryDecision {
    let Some(pending) = pending else {
        return BatchRetryDecision::ConstructFresh;
    };

    let mut conflicting = Vec::new();
    let mut any_unspent = false;
    let mut any_pending_spent = false;
    let mut observed_states = Vec::new();

    for coin_id in &pending.input_coin_ids {
        let state = input_states
            .get(coin_id)
            .copied()
            .unwrap_or(InputCoinState::SpentConflicting);
        observed_states.push((coin_id.clone(), state));
        match state {
            InputCoinState::Unspent => any_unspent = true,
            InputCoinState::SpentByPending => any_pending_spent = true,
            InputCoinState::SpentConflicting => conflicting.push(coin_id.clone()),
        }
    }

    if !conflicting.is_empty() || (any_unspent && any_pending_spent) {
        return BatchRetryDecision::Conflict(BatchRetryConflictReport {
            batch_id: pending.batch_id,
            phase: pending.phase.clone(),
            handles: pending.handles.clone(),
            conflicting_input_coin_ids: conflicting,
            observed_states,
        });
    }

    if any_unspent {
        return BatchRetryDecision::ReuseIdentical(pending.clone());
    }

    BatchRetryDecision::AlreadyApplied(pending.clone())
}

pub fn default_pending_batch_spend_path(batch_id: u32, phase: &str) -> String {
    format!("xchandles_pending_batch_{batch_id}_{phase}.json")
}

pub fn write_pending_batch_spend<P: AsRef<Path>>(
    path: P,
    record: &PendingBatchSpendRecord,
) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|e| CliError::Custom(format!("pending batch spend serialize failed: {e}")))?;
    fs::write(path, bytes)?;
    Ok(())
}

pub fn load_pending_batch_spend<P: AsRef<Path>>(
    path: P,
) -> Result<Option<PendingBatchSpendRecord>, CliError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let record: PendingBatchSpendRecord = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Custom(format!("failed to parse pending batch spend: {e}")))?;
    if record.format != PENDING_BATCH_SPEND_FORMAT {
        return Err(CliError::Custom(format!(
            "unexpected pending batch spend format {}",
            record.format
        )));
    }
    if record.version != PENDING_BATCH_SPEND_VERSION {
        return Err(CliError::Custom(format!(
            "unsupported pending batch spend version {}",
            record.version
        )));
    }
    Ok(Some(record))
}

pub fn clear_pending_batch_spend<P: AsRef<Path>>(path: P) -> Result<(), CliError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn new_pending_batch_spend(
    registry_launcher_id: Bytes32,
    batch_id: u32,
    phase: &str,
    handles: Vec<String>,
    input_coin_ids: Vec<Bytes32>,
    spend_bundle: SpendBundle,
) -> PendingBatchSpendRecord {
    PendingBatchSpendRecord {
        format: PENDING_BATCH_SPEND_FORMAT.to_string(),
        version: PENDING_BATCH_SPEND_VERSION,
        registry_launcher_id: hex::encode(registry_launcher_id),
        batch_id,
        phase: phase.to_string(),
        handles,
        input_coin_ids: input_coin_ids.into_iter().map(|id| hex::encode(id)).collect(),
        spend_bundle,
    }
}

/// Emit the complete pre-broadcast plan (stdout + optional path) before any
/// irreversible batch construction/submission.
pub fn emit_pre_broadcast_plan(
    bundle: &PremineLaunchBundle,
    plan_path: Option<&Path>,
) -> Result<crate::PreBroadcastPlan, CliError> {
    let plan = build_pre_broadcast_plan(bundle);
    let json = serde_json::to_string_pretty(&plan)
        .map_err(|e| CliError::Custom(format!("pre-broadcast plan serialize failed: {e}")))?;
    println!("Pre-broadcast plan (machine-readable):\n{json}");
    if let Some(path) = plan_path {
        fs::write(path, json.as_bytes())?;
        println!("Wrote pre-broadcast plan to {}", path.display());
    }
    Ok(plan)
}

/// Reject legacy Premine media-column CSV / non-bundle shapes. Bundle JSON is accepted.
pub fn reject_legacy_premine_bytes(bytes: &[u8]) -> Result<(), CliError> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        let format = value.get("format").and_then(|v| v.as_str());
        if format == Some(crate::BUNDLE_FORMAT) {
            return Ok(());
        }
        return Err(CliError::Custom(
            "legacy Premine record shape rejected; provide a versioned Premine Launch Bundle"
                .to_string(),
        ));
    }

    // Heuristic for legacy media-column CSV headers.
    let text = String::from_utf8_lossy(bytes);
    let header = text.lines().next().unwrap_or("").to_ascii_lowercase();
    if header.contains("image_uris")
        || header.contains("image_hash")
        || header.contains("metadata_uris")
        || (header.contains("handle")
            && header.contains("recipient")
            && !header.contains("allocation_type"))
    {
        return Err(CliError::Custom(
            "legacy Premine media-column CSV rejected; generate a Premine Launch Bundle first"
                .to_string(),
        ));
    }

    Err(CliError::Custom(
        "input is neither a Premine Launch Bundle nor recognized; launch accepts only the versioned bundle"
            .to_string(),
    ))
}

/// Bundle rows belonging to `batch_id`, in committed order.
pub fn bundle_rows_for_batch(
    bundle: &PremineLaunchBundle,
    batch_id: u32,
) -> Vec<&PremineLaunchBundleRow> {
    bundle
        .rows
        .iter()
        .filter(|r| r.batch_id == batch_id)
        .collect()
}

/// All bundle rows with `batch_id <= through_batch_id`, in committed order.
pub fn bundle_rows_through_batch(
    bundle: &PremineLaunchBundle,
    through_batch_id: u32,
) -> Vec<&PremineLaunchBundleRow> {
    bundle
        .rows
        .iter()
        .filter(|r| r.batch_id <= through_batch_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        generate_premine_launch_bundle, launch_handles_from_bundle, load_premine_launch_bundle,
        BUNDLE_FORMAT,
    };
    use chia_bls::Signature;
    use std::path::PathBuf;

    const ALICE_RECIPIENT: &str =
        "xch1z4lpey8mwe7f246f2y6a8hkfm6797g0q9rrx437cvy27v7anfcmsxj0evh";
    const LONG_HANDLE: &str = "ashorttermmindgetsinthewayofalongtermgrind";
    const LONG_RECIPIENT: &str =
        "xch1446hepskdwgn2nfunq0qhwweyjvwfn4kfcll6pznjkkfdptvrasqaxkxz5";

    fn fixture_csv() -> String {
        format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             {LONG_HANDLE},{LONG_RECIPIENT},1797757200,namesdao,https://mintgarden.io/nfts/nft13nj86c69u5mepktgf5m8akvzr2wj7ykyzh2eps9enfu2f9ne3pfq9gccs4\n\
             alice,{ALICE_RECIPIENT},1818752400,contributor,https://example.com/alice\n\
             bob,{ALICE_RECIPIENT},1797757200,cns,https://mintgarden.io/nfts/nft1bob\n"
        )
    }

    fn sample_observation_from_row(
        row: &PremineLaunchBundleRow,
        launcher: &str,
    ) -> (ExpectedPremineObservation, ObservedPremineHandle) {
        let expected = ExpectedPremineObservation {
            handle: row.handle.clone(),
            recipient_puzzle_hash: row.recipient_puzzle_hash.clone(),
            expiration: row.expiration,
            owner_resolved_relationship: row.owner_resolved_relationship.clone(),
            owner_launcher_id: launcher.to_string(),
            resolved_launcher_id: launcher.to_string(),
            nft_launcher_id: launcher.to_string(),
            display_name: row.display_name.clone(),
            image_uri: row.image_uri.clone(),
            image_hash: row.image_hash.clone(),
            metadata_uri: row.metadata_uri.clone(),
            metadata_hash: row.metadata_hash.clone(),
            license_uri: row.license_uri.clone(),
            license_hash: row.license_hash.clone(),
            handle_nft_metadata_clvm_hex: row.handle_nft_metadata_clvm_hex.clone(),
            updater_hash: row.updater_hash.clone(),
            royalty_puzzle_hash: row.royalty_puzzle_hash.clone(),
            royalty_basis_points: row.royalty_basis_points,
            batch_id: row.batch_id,
            row_index: row.row_index,
        };
        let observed = ObservedPremineHandle {
            handle: expected.handle.clone(),
            recipient_puzzle_hash: expected.recipient_puzzle_hash.clone(),
            expiration: expected.expiration,
            owner_resolved_relationship: expected.owner_resolved_relationship.clone(),
            owner_launcher_id: expected.owner_launcher_id.clone(),
            resolved_launcher_id: expected.resolved_launcher_id.clone(),
            nft_launcher_id: expected.nft_launcher_id.clone(),
            display_name: expected.display_name.clone(),
            image_uri: expected.image_uri.clone(),
            image_hash: expected.image_hash.clone(),
            metadata_uri: expected.metadata_uri.clone(),
            metadata_hash: expected.metadata_hash.clone(),
            license_uri: expected.license_uri.clone(),
            license_hash: expected.license_hash.clone(),
            handle_nft_metadata_clvm_hex: expected.handle_nft_metadata_clvm_hex.clone(),
            updater_hash: expected.updater_hash.clone(),
            royalty_puzzle_hash: expected.royalty_puzzle_hash.clone(),
            royalty_basis_points: expected.royalty_basis_points,
            batch_id: expected.batch_id,
            row_index: expected.row_index,
        };
        (expected, observed)
    }

    fn full_set_fixture() -> (Vec<ExpectedPremineObservation>, Vec<ObservedPremineHandle>) {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let mut expected = Vec::new();
        let mut observed = Vec::new();
        for (i, row) in bundle.rows.iter().enumerate() {
            let launcher = format!("{:064x}", i + 1);
            let (e, o) = sample_observation_from_row(row, &launcher);
            expected.push(e);
            observed.push(o);
        }
        (expected, observed)
    }

    #[test]
    fn successful_full_set_verification() {
        let (expected, observed) = full_set_fixture();
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(report.ok);
        assert!(report.missing_handles.is_empty());
        assert!(report.extra_handles.is_empty());
        assert!(report.field_diffs.is_empty());
        assert!(report.gate_later_batches().is_ok());
        assert!(expected.iter().any(|e| e.handle == LONG_HANDLE));
    }

    #[test]
    fn missing_row_fails_with_machine_readable_diff() {
        let (expected, mut observed) = full_set_fixture();
        observed.retain(|o| o.handle != "bob");
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(!report.ok);
        assert_eq!(report.missing_handles, vec!["bob".to_string()]);
        let err = report.gate_later_batches().unwrap_err().to_string();
        assert!(err.contains("missing_handles"));
        assert!(err.contains("bob"));
    }

    #[test]
    fn extra_row_fails_with_machine_readable_diff() {
        let (expected, mut observed) = full_set_fixture();
        let mut extra = observed[0].clone();
        extra.handle = "intruder".to_string();
        observed.push(extra);
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(!report.ok);
        assert_eq!(report.extra_handles, vec!["intruder".to_string()]);
    }

    #[test]
    fn duplicate_modified_observation_cannot_pass_via_map_collapse() {
        let (expected, mut observed) = full_set_fixture();
        let mut modified = observed[0].clone();
        modified.recipient_puzzle_hash = "00".repeat(32);
        // Earlier modified duplicate + later matching row: map would keep the match.
        observed.insert(0, modified);
        assert_eq!(observed.iter().filter(|o| o.handle == observed[1].handle).count(), 2);
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(!report.ok, "duplicate observations must fail closed");
        assert!(
            report
                .duplicate_observed_handles
                .contains(&observed[1].handle),
            "report={report:?}"
        );
        assert!(report.observed_count > report.expected_count);
        assert!(report.gate_later_batches().is_err());
    }

    #[test]
    fn duplicate_expected_handles_fail_closed() {
        let (mut expected, observed) = full_set_fixture();
        expected.push(expected[0].clone());
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Final);
        assert!(!report.ok);
        assert!(report
            .duplicate_expected_handles
            .contains(&expected[0].handle));
    }

    #[test]
    fn wrong_recipient_fails() {
        let (expected, mut observed) = full_set_fixture();
        observed[0].recipient_puzzle_hash = "00".repeat(32);
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Final);
        assert!(!report.ok);
        assert!(report
            .field_diffs
            .iter()
            .any(|d| d.field == "recipient_puzzle_hash"));
        assert_eq!(report.phase, VerificationPhase::Final);
    }

    #[test]
    fn wrong_expiration_fails() {
        let (expected, mut observed) = full_set_fixture();
        observed[1].expiration = 1;
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(report
            .field_diffs
            .iter()
            .any(|d| d.field == "expiration" && d.observed == "1"));
    }

    #[test]
    fn wrong_metadata_fails() {
        let (expected, mut observed) = full_set_fixture();
        observed[0].metadata_hash = "11".repeat(32);
        observed[0].image_uri = "https://evil.example/x.png".to_string();
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(report
            .field_diffs
            .iter()
            .any(|d| d.field == "metadata_hash"));
        assert!(report.field_diffs.iter().any(|d| d.field == "image_uri"));
    }

    #[test]
    fn wrong_royalty_fails() {
        let (expected, mut observed) = full_set_fixture();
        observed[0].royalty_basis_points = 999;
        observed[0].royalty_puzzle_hash = "22".repeat(32);
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(report
            .field_diffs
            .iter()
            .any(|d| d.field == "royalty_basis_points"));
        assert!(report
            .field_diffs
            .iter()
            .any(|d| d.field == "royalty_puzzle_hash"));
    }

    #[test]
    fn wrong_launcher_relationship_fails() {
        let (expected, mut observed) = full_set_fixture();
        observed[0].resolved_launcher_id = "33".repeat(32);
        observed[0].owner_resolved_relationship = "distinct_singletons".to_string();
        let report = compare_premine_set(&expected, &observed, VerificationPhase::Canonical);
        assert!(report.field_diffs.iter().any(|d| {
            d.field == "resolved_launcher_id"
                || d.field == "launcher_relationship"
                || d.field == "owner_resolved_relationship"
        }));
    }

    #[test]
    fn finality_requires_thirty_two_blocks() {
        assert!(!finality_reached(100, 131));
        assert!(finality_reached(100, 132));
        assert!(finality_reached(100, 200));
    }

    #[test]
    fn reorganization_before_finality_invalidates() {
        assert!(reorganization_invalidates_finality(100, None, 200));
        assert!(reorganization_invalidates_finality(100, Some(90), 200));
        assert!(reorganization_invalidates_finality(100, Some(100), 120));
        assert!(!reorganization_invalidates_finality(100, Some(100), 132));
    }

    #[test]
    fn reassigned_confirmation_is_not_final_even_if_peak_deep_over_old_anchor() {
        // Peak advanced 32+ over the original confirmation, but a reorg reassigned
        // the spend to a recent height — finality must not be claimed yet.
        let original = 100u32;
        let reassigned = 180u32;
        let peak = 200u32;
        assert!(finality_reached(original, peak));
        assert!(!finality_reached(reassigned, peak));
        assert!(reorganization_invalidates_finality(
            original,
            Some(reassigned),
            peak
        ));
    }

    fn empty_sb() -> SpendBundle {
        SpendBundle::new(vec![], Signature::default())
    }

    fn sample_pending(batch_id: u32) -> PendingBatchSpendRecord {
        PendingBatchSpendRecord {
            format: PENDING_BATCH_SPEND_FORMAT.to_string(),
            version: PENDING_BATCH_SPEND_VERSION,
            registry_launcher_id: "aa".repeat(32),
            batch_id,
            phase: "mint_precommit".to_string(),
            handles: vec!["alice".to_string(), "bob".to_string()],
            input_coin_ids: vec!["01".repeat(32), "02".repeat(32)],
            spend_bundle: empty_sb(),
        }
    }

    #[test]
    fn identical_retry_reuses_spend_while_inputs_unspent() {
        let pending = sample_pending(0);
        let mut states = BTreeMap::new();
        states.insert("01".repeat(32), InputCoinState::Unspent);
        states.insert("02".repeat(32), InputCoinState::Unspent);
        match decide_batch_retry(Some(&pending), &states) {
            BatchRetryDecision::ReuseIdentical(reused) => {
                assert_eq!(reused.batch_id, 0);
                assert_eq!(reused.handles, pending.handles);
                assert_eq!(reused.input_coin_ids, pending.input_coin_ids);
            }
            other => panic!("expected ReuseIdentical, got {other:?}"),
        }
    }

    #[test]
    fn spent_conflicting_input_stops_blind_retry() {
        let pending = sample_pending(1);
        let mut states = BTreeMap::new();
        states.insert("01".repeat(32), InputCoinState::Unspent);
        states.insert("02".repeat(32), InputCoinState::SpentConflicting);
        match decide_batch_retry(Some(&pending), &states) {
            BatchRetryDecision::Conflict(report) => {
                assert_eq!(report.batch_id, 1);
                assert_eq!(report.conflicting_input_coin_ids, vec!["02".repeat(32)]);
                let json = serde_json::to_string(&report).unwrap();
                assert!(json.contains("spent_conflicting") || json.contains("02"));
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn all_inputs_spent_by_conflict_are_not_already_applied() {
        let pending = sample_pending(2);
        let mut states = BTreeMap::new();
        states.insert("01".repeat(32), InputCoinState::SpentConflicting);
        states.insert("02".repeat(32), InputCoinState::SpentConflicting);
        match decide_batch_retry(Some(&pending), &states) {
            BatchRetryDecision::Conflict(report) => {
                assert_eq!(report.conflicting_input_coin_ids.len(), 2);
            }
            other => panic!("expected Conflict when all spent conflicting, got {other:?}"),
        }
    }

    #[test]
    fn classify_spent_requires_matching_pending_coin_spend() {
        use chia_protocol::{Bytes, Coin, Program};

        let coin = Coin::new(Bytes32::new([9u8; 32]), Bytes32::new([8u8; 32]), 1);
        let pending_cs = CoinSpend::new(
            coin,
            Program::new(Bytes::new(vec![1, 2, 3])),
            Program::new(Bytes::new(vec![4, 5, 6])),
        );
        let conflicting_cs = CoinSpend::new(
            coin,
            Program::new(Bytes::new(vec![1, 2, 3])),
            Program::new(Bytes::new(vec![9, 9, 9])),
        );

        assert_eq!(
            classify_pending_input_state(true, false, Some(&pending_cs), None),
            InputCoinState::Unspent
        );
        assert_eq!(
            classify_pending_input_state(true, true, Some(&pending_cs), Some(&pending_cs)),
            InputCoinState::SpentByPending
        );
        assert_eq!(
            classify_pending_input_state(true, true, Some(&pending_cs), Some(&conflicting_cs)),
            InputCoinState::SpentConflicting
        );
        assert_eq!(
            classify_pending_input_state(true, true, Some(&pending_cs), None),
            InputCoinState::SpentConflicting
        );
        assert_eq!(
            classify_pending_input_state(false, true, Some(&pending_cs), None),
            InputCoinState::SpentConflicting
        );
    }

    #[test]
    fn spend_bundle_input_coin_ids_lists_every_spend() {
        use chia_protocol::{Bytes, Coin, Program};

        let c1 = Coin::new(Bytes32::new([1u8; 32]), Bytes32::new([2u8; 32]), 1);
        let c2 = Coin::new(Bytes32::new([3u8; 32]), Bytes32::new([4u8; 32]), 1);
        let sb = SpendBundle::new(
            vec![
                CoinSpend::new(
                    c1,
                    Program::new(Bytes::new(vec![1])),
                    Program::new(Bytes::new(vec![2])),
                ),
                CoinSpend::new(
                    c2,
                    Program::new(Bytes::new(vec![3])),
                    Program::new(Bytes::new(vec![4])),
                ),
            ],
            Signature::default(),
        );
        let ids = spend_bundle_input_coin_ids(&sb);
        assert_eq!(ids, vec![c1.coin_id(), c2.coin_id()]);
    }

    #[test]
    fn no_pending_constructs_fresh() {
        let states = BTreeMap::new();
        assert_eq!(
            decide_batch_retry(None, &states),
            BatchRetryDecision::ConstructFresh
        );
    }

    #[test]
    fn launch_rejects_legacy_premine_media_csv() {
        let legacy = "handle,recipient,image_uris,image_hash,metadata_uris,metadata_hash,license_uris,license_hash\n\
                      alice,xch1z4lpey8mwe7f246f2y6a8hkfm6797g0q9rrx437cvy27v7anfcmsxj0evh,https://x/a.png,aa,https://x/a.json,bb,https://x/l.txt,cc\n";
        let err = reject_legacy_premine_bytes(legacy.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("legacy"));
    }

    #[test]
    fn launch_rejects_legacy_json_without_bundle_format() {
        let legacy = r#"[{"handle":"alice","recipient":"xch1...","image_uris":["u"]}]"#;
        let err = reject_legacy_premine_bytes(legacy.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("legacy") || err.to_string().contains("versioned"));
    }

    #[test]
    fn launch_accepts_versioned_bundle_bytes() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let bytes = serde_json::to_vec(&bundle).unwrap();
        reject_legacy_premine_bytes(&bytes).unwrap();
        assert_eq!(bundle.format, BUNDLE_FORMAT);
    }

    #[test]
    fn pre_broadcast_plan_emitted_before_irreversible() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "xchandles-plan-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let plan_path = dir.join("plan.json");
        let plan = emit_pre_broadcast_plan(&bundle, Some(&plan_path)).unwrap();
        assert_eq!(plan.total_rows, 3);
        // Lexicographic: alice < ashort... < bob
        assert_eq!(
            plan.batches[0].handles,
            vec![
                "alice".to_string(),
                LONG_HANDLE.to_string(),
                "bob".to_string()
            ]
        );
        let loaded = fs::read_to_string(&plan_path).unwrap();
        assert!(loaded.contains(LONG_HANDLE));
        assert!(loaded.contains("xchandles-premine-pre-broadcast-plan"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_rows_match_bundle_membership_order() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let rows = bundle_rows_for_batch(&bundle, 0);
        let handles: Vec<_> = rows.iter().map(|r| r.handle.as_str()).collect();
        assert_eq!(handles, vec!["alice", LONG_HANDLE, "bob"]);
        let launch = launch_handles_from_bundle(&bundle).unwrap();
        assert_eq!(
            launch.iter().map(|h| h.handle.as_str()).collect::<Vec<_>>(),
            handles
        );
    }

    #[test]
    fn pending_spend_roundtrip_preserves_identical_bundle() {
        let pending = sample_pending(0);
        let path = std::env::temp_dir().join(format!(
            "pending-sb-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_pending_batch_spend(&path, &pending).unwrap();
        let loaded = load_pending_batch_spend(&path).unwrap().unwrap();
        assert_eq!(loaded.batch_id, pending.batch_id);
        assert_eq!(loaded.handles, pending.handles);
        assert_eq!(loaded.input_coin_ids, pending.input_coin_ids);
        clear_pending_batch_spend(&path).unwrap();
        assert!(load_pending_batch_spend(&path).unwrap().is_none());
    }

    #[test]
    fn load_bundle_rejects_non_bundle_file() {
        let path = PathBuf::from(format!(
            "/tmp/legacy-premine-{}.csv",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(
            &path,
            "handle,recipient,image_uris,image_hash,metadata_uris,metadata_hash,license_uris,license_hash\n",
        )
        .unwrap();
        let err = load_premine_launch_bundle(&path).unwrap_err();
        assert!(
            err.to_string().contains("legacy")
                || err.to_string().contains("failed to parse")
                || err.to_string().contains("bundle")
        );
        let _ = fs::remove_file(&path);
    }
}

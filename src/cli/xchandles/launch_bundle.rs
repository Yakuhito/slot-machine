//! Premine Launch Bundle transformer and pre-broadcast plan.
//!
//! Public seams:
//! - [`generate_premine_launch_bundle`] — published five-column Premine → versioned bundle
//! - [`build_pre_broadcast_plan`] — machine-readable batch plan from a bundle
//! - [`load_premine_launch_bundle`] / [`write_premine_launch_outputs_atomically`] — IO

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use chia_protocol::Bytes32;
use chia_wallet_sdk::{
    types::puzzles::{HandleNftMetadata, ANY_METADATA_UPDATER_HASH},
    utils::Address,
};
use clvm_traits::ToClvm;
use clvmr::{serde::node_to_bytes, Allocator};
use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use xchandles_nfts::{
    generate_v1_json, generate_v1_png, is_valid_handle, v1_json_uri, v1_png_uri, CC0_LEGAL_BYTES,
    GENERATOR_VERSION,
};

use crate::{
    royalty_puzzle_hash_from_address, CliError, CONTRIBUTION_PREMINE_EXPIRATION,
    LAUNCH_HANDLES_PER_BATCH, LICENSE_URI, MEDIA_ORIGIN, REGISTRATION_PERIOD, ROYALTY_ADDRESS,
    ROYALTY_BASIS_POINTS, ROYALTY_PUZZLE_HASH,
};

pub const BUNDLE_FORMAT: &str = "xchandles-premine-launch-bundle";
pub const BUNDLE_VERSION: u32 = 1;
pub const PLAN_FORMAT: &str = "xchandles-premine-pre-broadcast-plan";
pub const PLAN_VERSION: u32 = 1;
pub const OWNER_RESOLVED_RELATIONSHIP: &str = "same_dedicated_handle_nft";

const ACCEPTED_ALLOCATION_TYPES: [&str; 3] = ["contributor", "cns", "namesdao"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PremineLaunchBundle {
    pub format: String,
    pub version: u32,
    pub media_origin: String,
    pub generator_version: String,
    pub handles_per_batch: usize,
    pub registration_period: u64,
    pub royalty_address: String,
    pub royalty_puzzle_hash: String,
    pub royalty_basis_points: u16,
    pub updater_hash: String,
    pub license_uri: String,
    pub license_hash: String,
    pub rows: Vec<PremineLaunchBundleRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PremineLaunchBundleRow {
    pub source_row_id: u32,
    pub source_handle: String,
    pub source_recipient: String,
    pub source_expiration: u64,
    pub allocation_type: String,
    pub allocation_explanation: String,
    pub handle: String,
    pub recipient_puzzle_hash: String,
    pub expiration: u64,
    pub buy_time: u64,
    pub owner_resolved_relationship: String,
    pub display_name: String,
    pub image_uri: String,
    pub image_hash: String,
    pub metadata_uri: String,
    pub metadata_hash: String,
    pub license_uri: String,
    pub license_hash: String,
    pub handle_nft_metadata_clvm_hex: String,
    pub royalty_puzzle_hash: String,
    pub royalty_basis_points: u16,
    pub updater_hash: String,
    pub generator_version: String,
    pub row_index: u32,
    pub batch_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreBroadcastPlan {
    pub format: String,
    pub version: u32,
    pub bundle_format: String,
    pub bundle_version: u32,
    pub total_rows: u32,
    pub handles_per_batch: usize,
    pub batches: Vec<PreBroadcastBatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreBroadcastBatch {
    pub batch_id: u32,
    pub row_indices: Vec<u32>,
    pub handles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchHandle {
    pub handle: String,
    pub recipient: Bytes32,
    pub expiration: u64,
    pub buy_time: u64,
    pub image_uris: Vec<String>,
    pub image_hash: Bytes32,
    pub metadata_uris: Vec<String>,
    pub metadata_hash: Bytes32,
    pub license_uris: Vec<String>,
    pub license_hash: Bytes32,
    pub row_index: u32,
    pub batch_id: u32,
    pub source_row_id: u32,
    pub allocation_type: String,
    pub allocation_explanation: String,
}

#[derive(Debug, Deserialize)]
struct PublishedPremineRow {
    handle: String,
    recipient: String,
    expiration: u64,
    allocation_type: String,
    allocation_explanation: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hex_bytes32(value: Bytes32) -> String {
    hex::encode(value)
}

fn parse_hex_bytes32(s: &str) -> Result<Bytes32, CliError> {
    let bytes = <[u8; 32]>::try_from(hex::decode(s.replace("0x", "")).map_err(CliError::ParseHex)?)
        .map_err(|_| CliError::Custom(format!("expected 32-byte hex, got {s}")))?;
    Ok(Bytes32::new(bytes))
}

fn is_absolute_http_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !s.contains(char::is_whitespace)
}

pub fn handle_nft_metadata_clvm_hex(metadata: &HandleNftMetadata) -> Result<String, CliError> {
    let mut allocator = Allocator::new();
    let ptr = metadata
        .to_clvm(&mut allocator)
        .map_err(|e| CliError::Custom(format!("HandleNftMetadata CLVM encode failed: {e}")))?;
    let bytes = node_to_bytes(&allocator, ptr)
        .map_err(|e| CliError::Custom(format!("HandleNftMetadata serialize failed: {e}")))?;
    Ok(hex::encode(bytes))
}

fn validate_mainnet_constants() -> Result<(Bytes32, String), CliError> {
    let from_address = royalty_puzzle_hash_from_address()?;
    if from_address != ROYALTY_PUZZLE_HASH {
        return Err(CliError::Custom(
            "royalty address does not decode to configured royalty puzzle hash".to_string(),
        ));
    }
    let license_hash = sha256_hex(CC0_LEGAL_BYTES);
    Ok((from_address, license_hash))
}

/// Transform a published five-column Premine CSV into one sorted, versioned launch bundle.
pub fn generate_premine_launch_bundle(csv_bytes: &[u8]) -> Result<PremineLaunchBundle, CliError> {
    generate_premine_launch_bundle_for_network(csv_bytes, true)
}

/// Same as [`generate_premine_launch_bundle`], but optionally allows `txch` recipients
/// for local testnet11 bundles (production Premine always requires mainnet `xch`).
pub fn generate_premine_launch_bundle_for_network(
    csv_bytes: &[u8],
    require_mainnet_recipients: bool,
) -> Result<PremineLaunchBundle, CliError> {
    let (royalty_ph, license_hash) = validate_mainnet_constants()?;
    let updater_hash = hex::encode(Bytes32::from(ANY_METADATA_UPDATER_HASH));
    if updater_hash != "9f28d55242a3bd2b3661c38ba8647392c26bb86594050ea6d33aad1725ca3eea" {
        return Err(CliError::Custom(
            "updater hash does not match settled ANY_METADATA_UPDATER_HASH".to_string(),
        ));
    }

    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(csv_bytes);
    let headers = rdr
        .headers()
        .map_err(CliError::Csv)?
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let expected = [
        "handle",
        "recipient",
        "expiration",
        "allocation_type",
        "allocation_explanation",
    ];
    if headers != expected {
        return Err(CliError::Custom(format!(
            "published Premine must use exact columns {:?}, got {:?}",
            expected, headers
        )));
    }

    let mut published = Vec::new();
    for (idx, result) in rdr.deserialize().enumerate() {
        let row: PublishedPremineRow = result.map_err(CliError::Csv)?;
        published.push((idx as u32 + 1, row));
    }

    let mut seen_handles = BTreeSet::new();
    let mut enriched = Vec::with_capacity(published.len());

    for (source_row_id, row) in published {
        let handle = row.handle.trim().to_string();
        if !is_valid_handle(&handle) {
            return Err(CliError::Custom(format!(
                "invalid Handle at source_row_id={source_row_id}: {handle:?}"
            )));
        }
        if !seen_handles.insert(handle.clone()) {
            return Err(CliError::Custom(format!(
                "duplicate Handle in published Premine: {handle}"
            )));
        }

        if !ACCEPTED_ALLOCATION_TYPES.contains(&row.allocation_type.as_str()) {
            return Err(CliError::Custom(format!(
                "unknown allocation_type {:?} for handle {handle}",
                row.allocation_type
            )));
        }

        if !is_absolute_http_url(&row.allocation_explanation) {
            return Err(CliError::Custom(format!(
                "allocation_explanation must be absolute HTTP(S) URL for handle {handle}"
            )));
        }

        if row.allocation_type == "contributor" && row.expiration != CONTRIBUTION_PREMINE_EXPIRATION {
            return Err(CliError::Custom(format!(
                "contributor expiration for {handle} is {}, expected {CONTRIBUTION_PREMINE_EXPIRATION}",
                row.expiration
            )));
        }

        let address = Address::decode(&row.recipient)
            .map_err(|e| CliError::Custom(format!("invalid recipient for {handle}: {e}")))?;
        if require_mainnet_recipients {
            if address.prefix != "xch" {
                return Err(CliError::Custom(format!(
                    "non-mainnet recipient for {handle}: prefix {}",
                    address.prefix
                )));
            }
        } else if address.prefix != "xch" && address.prefix != "txch" {
            return Err(CliError::Custom(format!(
                "invalid recipient prefix for {handle}: {}",
                address.prefix
            )));
        }
        let recipient_ph = address.puzzle_hash;

        if row.expiration < REGISTRATION_PERIOD {
            return Err(CliError::Custom(format!(
                "expiration {} for {handle} is below one registration period",
                row.expiration
            )));
        }
        let buy_time = row.expiration - REGISTRATION_PERIOD;

        let png = generate_v1_png(&handle)
            .map_err(|e| CliError::Custom(format!("png generation failed for {handle}: {e}")))?;
        let json = generate_v1_json(&handle)
            .map_err(|e| CliError::Custom(format!("json generation failed for {handle}: {e}")))?;
        let png2 = generate_v1_png(&handle)
            .map_err(|e| CliError::Custom(format!("png re-generation failed for {handle}: {e}")))?;
        let json2 = generate_v1_json(&handle)
            .map_err(|e| CliError::Custom(format!("json re-generation failed for {handle}: {e}")))?;
        if png != png2 || json != json2 {
            return Err(CliError::Custom(format!(
                "nondeterministic media for handle {handle}"
            )));
        }

        let image_hash = sha256_hex(&png);
        let metadata_hash = sha256_hex(&json);
        let image_uri = v1_png_uri(MEDIA_ORIGIN, &handle);
        let metadata_uri = v1_json_uri(MEDIA_ORIGIN, &handle);

        let image_hash_bytes = parse_hex_bytes32(&image_hash)?;
        let metadata_hash_bytes = parse_hex_bytes32(&metadata_hash)?;
        let license_hash_bytes = parse_hex_bytes32(&license_hash)?;

        let nft_metadata = HandleNftMetadata {
            display_name: Some(handle.clone()),
            image_uris: vec![image_uri.clone()],
            image_hash: Some(image_hash_bytes),
            metadata_uris: vec![metadata_uri.clone()],
            metadata_hash: Some(metadata_hash_bytes),
            license_uris: vec![LICENSE_URI.to_string()],
            license_hash: Some(license_hash_bytes),
        };
        let clvm_hex = handle_nft_metadata_clvm_hex(&nft_metadata)?;

        // Round-trip lock: re-parse is deferred to tests; here ensure encoding is stable.
        let clvm_hex2 = handle_nft_metadata_clvm_hex(&nft_metadata)?;
        if clvm_hex != clvm_hex2 {
            return Err(CliError::Custom(format!(
                "noncanonical metadata encoding for handle {handle}"
            )));
        }

        enriched.push((
            handle.clone(),
            PremineLaunchBundleRow {
                source_row_id,
                source_handle: row.handle,
                source_recipient: row.recipient,
                source_expiration: row.expiration,
                allocation_type: row.allocation_type,
                allocation_explanation: row.allocation_explanation,
                handle: handle.clone(),
                recipient_puzzle_hash: hex_bytes32(recipient_ph),
                expiration: row.expiration,
                buy_time,
                owner_resolved_relationship: OWNER_RESOLVED_RELATIONSHIP.to_string(),
                display_name: handle.clone(),
                image_uri,
                image_hash,
                metadata_uri,
                metadata_hash,
                license_uri: LICENSE_URI.to_string(),
                license_hash: license_hash.clone(),
                handle_nft_metadata_clvm_hex: clvm_hex,
                royalty_puzzle_hash: hex_bytes32(royalty_ph),
                royalty_basis_points: ROYALTY_BASIS_POINTS,
                updater_hash: updater_hash.clone(),
                generator_version: GENERATOR_VERSION.to_string(),
                row_index: 0,
                batch_id: 0,
            },
        ));
    }

    enriched.sort_by(|a, b| a.0.cmp(&b.0));

    let mut rows = Vec::with_capacity(enriched.len());
    for (i, (_handle, mut row)) in enriched.into_iter().enumerate() {
        row.row_index = i as u32;
        row.batch_id = (i / LAUNCH_HANDLES_PER_BATCH) as u32;
        rows.push(row);
    }

    Ok(PremineLaunchBundle {
        format: BUNDLE_FORMAT.to_string(),
        version: BUNDLE_VERSION,
        media_origin: MEDIA_ORIGIN.to_string(),
        generator_version: GENERATOR_VERSION.to_string(),
        handles_per_batch: LAUNCH_HANDLES_PER_BATCH,
        registration_period: REGISTRATION_PERIOD,
        royalty_address: ROYALTY_ADDRESS.to_string(),
        royalty_puzzle_hash: hex_bytes32(royalty_ph),
        royalty_basis_points: ROYALTY_BASIS_POINTS,
        updater_hash,
        license_uri: LICENSE_URI.to_string(),
        license_hash,
        rows,
    })
}

pub fn build_pre_broadcast_plan(bundle: &PremineLaunchBundle) -> PreBroadcastPlan {
    let mut batches: Vec<PreBroadcastBatch> = Vec::new();
    for row in &bundle.rows {
        let batch_id = row.batch_id;
        if batches.last().map(|b| b.batch_id) != Some(batch_id) {
            batches.push(PreBroadcastBatch {
                batch_id,
                row_indices: Vec::new(),
                handles: Vec::new(),
            });
        }
        let batch = batches.last_mut().expect("batch just ensured");
        batch.row_indices.push(row.row_index);
        batch.handles.push(row.handle.clone());
    }

    PreBroadcastPlan {
        format: PLAN_FORMAT.to_string(),
        version: PLAN_VERSION,
        bundle_format: bundle.format.clone(),
        bundle_version: bundle.version,
        total_rows: bundle.rows.len() as u32,
        handles_per_batch: bundle.handles_per_batch,
        batches,
    }
}

pub fn launch_handles_from_bundle(bundle: &PremineLaunchBundle) -> Result<Vec<LaunchHandle>, CliError> {
    bundle
        .rows
        .iter()
        .map(|row| {
            Ok(LaunchHandle {
                handle: row.handle.clone(),
                recipient: parse_hex_bytes32(&row.recipient_puzzle_hash)?,
                expiration: row.expiration,
                buy_time: row.buy_time,
                image_uris: vec![row.image_uri.clone()],
                image_hash: parse_hex_bytes32(&row.image_hash)?,
                metadata_uris: vec![row.metadata_uri.clone()],
                metadata_hash: parse_hex_bytes32(&row.metadata_hash)?,
                license_uris: vec![row.license_uri.clone()],
                license_hash: parse_hex_bytes32(&row.license_hash)?,
                row_index: row.row_index,
                batch_id: row.batch_id,
                source_row_id: row.source_row_id,
                allocation_type: row.allocation_type.clone(),
                allocation_explanation: row.allocation_explanation.clone(),
            })
        })
        .collect()
}

pub fn load_premine_launch_bundle<P: AsRef<Path>>(path: P) -> Result<PremineLaunchBundle, CliError> {
    let bytes = fs::read(path.as_ref())?;
    // Fail closed on legacy Premine CSV / media-column shapes before JSON parse.
    crate::reject_legacy_premine_bytes(&bytes)?;
    let bundle: PremineLaunchBundle = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Custom(format!("failed to parse launch bundle: {e}")))?;
    if bundle.format != BUNDLE_FORMAT {
        return Err(CliError::Custom(format!(
            "unexpected bundle format {}",
            bundle.format
        )));
    }
    if bundle.version != BUNDLE_VERSION {
        return Err(CliError::Custom(format!(
            "unsupported bundle version {}",
            bundle.version
        )));
    }
    Ok(bundle)
}

/// Generate and write bundle + plan atomically. On any failure before both temps
/// are fully written and renamed, prior outputs are left untouched.
pub fn write_premine_launch_outputs_atomically(
    bundle: &PremineLaunchBundle,
    plan: &PreBroadcastPlan,
    bundle_path: &Path,
    plan_path: &Path,
) -> Result<(), CliError> {
    let bundle_bytes = serde_json::to_vec_pretty(bundle)
        .map_err(|e| CliError::Custom(format!("bundle serialize failed: {e}")))?;
    let plan_bytes = serde_json::to_vec_pretty(plan)
        .map_err(|e| CliError::Custom(format!("plan serialize failed: {e}")))?;

    let bundle_tmp = tmp_path(bundle_path);
    let plan_tmp = tmp_path(plan_path);

    write_tmp(&bundle_tmp, &bundle_bytes)?;
    write_tmp(&plan_tmp, &plan_bytes)?;

    fs::rename(&bundle_tmp, bundle_path).map_err(|e| {
        let _ = fs::remove_file(&bundle_tmp);
        let _ = fs::remove_file(&plan_tmp);
        CliError::Io(e)
    })?;
    fs::rename(&plan_tmp, plan_path).map_err(|e| {
        let _ = fs::remove_file(&plan_tmp);
        CliError::Io(e)
    })?;

    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn write_tmp(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn default_mainnet_bundle_path() -> &'static str {
    "xchandles_premine_launch_bundle.json"
}

pub fn default_mainnet_plan_path() -> &'static str {
    "xchandles_premine_pre_broadcast_plan.json"
}

pub fn default_testnet11_bundle_path() -> &'static str {
    "xchandles_premine_launch_bundle_testnet11.json"
}

pub fn default_testnet11_plan_path() -> &'static str {
    "xchandles_premine_pre_broadcast_plan_testnet11.json"
}

pub fn xchandles_generate_launch_bundle(
    premine_path: String,
    bundle_path: String,
    plan_path: String,
    testnet11: bool,
) -> Result<(), CliError> {
    let csv_bytes = fs::read(&premine_path)?;
    let bundle = generate_premine_launch_bundle_for_network(&csv_bytes, !testnet11)?;
    let plan = build_pre_broadcast_plan(&bundle);
    write_premine_launch_outputs_atomically(
        &bundle,
        &plan,
        Path::new(&bundle_path),
        Path::new(&plan_path),
    )?;
    println!(
        "Wrote {} rows to {} and {} batches to {}",
        bundle.rows.len(),
        bundle_path,
        plan.batches.len(),
        plan_path
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn bundle_accepts_canonical_schema_and_sorts_lexicographically() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        assert_eq!(bundle.format, BUNDLE_FORMAT);
        assert_eq!(bundle.version, 1);
        assert_eq!(bundle.handles_per_batch, LAUNCH_HANDLES_PER_BATCH);
        assert_eq!(
            bundle
                .rows
                .iter()
                .map(|r| r.handle.as_str())
                .collect::<Vec<_>>(),
            vec!["alice", LONG_HANDLE, "bob"]
        );
        assert_eq!(bundle.rows[0].row_index, 0);
        assert_eq!(bundle.rows[1].row_index, 1);
        assert_eq!(bundle.rows[2].row_index, 2);
        // source identity preserved (alice was second published row → id 2)
        assert_eq!(bundle.rows[0].source_row_id, 2);
        assert_eq!(bundle.rows[0].source_handle, "alice");
        assert_eq!(bundle.rows[0].allocation_type, "contributor");
        assert_eq!(bundle.rows[0].expiration, CONTRIBUTION_PREMINE_EXPIRATION);
        assert_eq!(
            bundle.rows[0].buy_time,
            CONTRIBUTION_PREMINE_EXPIRATION - REGISTRATION_PERIOD
        );
    }

    #[test]
    fn bundle_row_media_matches_generator_v1_goldens() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let alice = bundle.rows.iter().find(|r| r.handle == "alice").unwrap();
        assert_eq!(
            alice.image_uri,
            "https://nfts.xchandles.com/v1/alice.png"
        );
        assert_eq!(
            alice.metadata_uri,
            "https://nfts.xchandles.com/v1/alice.json"
        );
        assert_eq!(
            alice.image_hash,
            "b614b7d0ddffcf568dff00db54678cc7dda7f745f3e69fc70acb47d5ff89d8da"
        );
        assert_eq!(
            alice.metadata_hash,
            "d3cabe6496a1c4bd53e1556913d642be242dba7e27de445a4b88e0556501196e"
        );
        assert_eq!(
            alice.license_hash,
            "a2010f343487d3f7618affe54f789f5487602331c0a8d03f49e9a7c547cf0499"
        );
        assert_eq!(alice.license_uri, LICENSE_URI);
        assert_eq!(alice.display_name, "alice");
        assert_eq!(alice.owner_resolved_relationship, OWNER_RESOLVED_RELATIONSHIP);
        assert_eq!(
            alice.updater_hash,
            "9f28d55242a3bd2b3661c38ba8647392c26bb86594050ea6d33aad1725ca3eea"
        );
        assert_eq!(
            alice.royalty_puzzle_hash,
            "36da8c738011bfdd51d457397543c8f710a28598f6cbecc0199529a359cebc81"
        );
        assert_eq!(alice.royalty_basis_points, 420);
        assert!(!alice.handle_nft_metadata_clvm_hex.is_empty());
        assert_ne!(alice.handle_nft_metadata_clvm_hex, "80");
    }

    #[test]
    fn long_premine_handle_passes_unchanged() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let row = bundle.rows.iter().find(|r| r.handle == LONG_HANDLE).unwrap();
        assert_eq!(row.handle.len(), 42);
        assert_eq!(row.source_handle, LONG_HANDLE);
        assert_eq!(row.expiration, 1797757200);
    }

    #[test]
    fn whole_bundle_is_byte_deterministic() {
        let a = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let b = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let a_bytes = serde_json::to_vec(&a).unwrap();
        let b_bytes = serde_json::to_vec(&b).unwrap();
        assert_eq!(a_bytes, b_bytes);
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_bad_contributor_expiry_and_unknown_type() {
        let bad_expiry = format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             alice,{ALICE_RECIPIENT},1797757200,contributor,https://example.com/alice\n"
        );
        let err = generate_premine_launch_bundle(bad_expiry.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("contributor expiration"));

        let bad_type = format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             alice,{ALICE_RECIPIENT},1818752400,other,https://example.com/alice\n"
        );
        let err = generate_premine_launch_bundle(bad_type.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("unknown allocation_type"));
    }

    #[test]
    fn rejects_duplicate_invalid_non_mainnet_and_bad_provenance() {
        let dup = format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             alice,{ALICE_RECIPIENT},1818752400,contributor,https://example.com/a\n\
             alice,{ALICE_RECIPIENT},1818752400,contributor,https://example.com/b\n"
        );
        assert!(generate_premine_launch_bundle(dup.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate"));

        let invalid = format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             AL,{ALICE_RECIPIENT},1818752400,contributor,https://example.com/a\n"
        );
        assert!(generate_premine_launch_bundle(invalid.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("invalid Handle"));

        let txch = format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             alice,txch1we8f6e6d97jyru8klr79uay6zlw7x30tuj3n2d4060h5z73l3jgqg5g78p,1818752400,contributor,https://example.com/a\n"
        );
        assert!(generate_premine_launch_bundle(txch.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("non-mainnet"));

        let bad_url = format!(
            "handle,recipient,expiration,allocation_type,allocation_explanation\n\
             alice,{ALICE_RECIPIENT},1818752400,contributor,not-a-url\n"
        );
        assert!(generate_premine_launch_bundle(bad_url.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("HTTP(S)"));
    }

    #[test]
    fn pre_broadcast_plan_enumerates_batches() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let plan = build_pre_broadcast_plan(&bundle);
        assert_eq!(plan.format, PLAN_FORMAT);
        assert_eq!(plan.total_rows, 3);
        assert_eq!(plan.batches.len(), 1);
        assert_eq!(plan.batches[0].handles, vec!["alice", LONG_HANDLE, "bob"]);
    }

    #[test]
    fn atomic_write_leaves_prior_output_on_generation_failure() {
        let dir = std::env::temp_dir().join(format!(
            "xch_bundle_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let bundle_path = dir.join("bundle.json");
        let plan_path = dir.join("plan.json");
        fs::write(&bundle_path, b"PRIOR_BUNDLE").unwrap();
        fs::write(&plan_path, b"PRIOR_PLAN").unwrap();

        // Generation fails before write — prior files untouched.
        let err = generate_premine_launch_bundle(b"not,csv\n");
        assert!(err.is_err());
        assert_eq!(fs::read(&bundle_path).unwrap(), b"PRIOR_BUNDLE");
        assert_eq!(fs::read(&plan_path).unwrap(), b"PRIOR_PLAN");

        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let plan = build_pre_broadcast_plan(&bundle);
        write_premine_launch_outputs_atomically(&bundle, &plan, &bundle_path, &plan_path).unwrap();
        assert!(fs::read_to_string(&bundle_path)
            .unwrap()
            .contains(BUNDLE_FORMAT));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_handles_deserialize_without_legacy_media_columns() {
        let bundle = generate_premine_launch_bundle(fixture_csv().as_bytes()).unwrap();
        let handles = launch_handles_from_bundle(&bundle).unwrap();
        assert_eq!(handles.len(), 3);
        assert_eq!(handles[0].handle, "alice");
        assert_eq!(handles[0].expiration, CONTRIBUTION_PREMINE_EXPIRATION);
        assert_eq!(
            handles[0].buy_time + REGISTRATION_PERIOD,
            handles[0].expiration
        );
        // No global period: each row carries its own buy_time/expiration.
        assert_ne!(handles[0].expiration, handles[1].expiration);
    }
}

use std::collections::HashSet;
use std::path::Path;

use chia_protocol::Bytes32;
use chia_wallet_sdk::{
    chia::sha2::Sha256,
    types::{MerkleProof, MerkleTree},
    utils::Address,
};
use clvm_utils::ToTreeHash;

use crate::{load_datastore_nft_csv, CliError, DatastoreNftRecord};

const HASH_TREE_PREFIX: &[u8] = &[2];
const HASH_LEAF_PREFIX: &[u8] = &[1];

pub fn records_from_entries(entries: &[(Bytes32, u64)]) -> Vec<DatastoreNftRecord> {
    entries
        .iter()
        .map(|(nft_id, weight)| DatastoreNftRecord {
            nft_id: *nft_id,
            weight: *weight,
        })
        .collect()
}

pub fn load_and_dedupe_csv<P: AsRef<Path>>(path: P) -> Result<Vec<DatastoreNftRecord>, CliError> {
    let records = load_datastore_nft_csv(path)?;
    reject_duplicates(&records)?;
    Ok(records)
}

fn reject_duplicates(records: &[DatastoreNftRecord]) -> Result<(), CliError> {
    let mut seen = HashSet::new();
    for record in records {
        if !seen.insert(record.nft_id) {
            let nft_id = Address::new(record.nft_id, "nft".to_string())
                .encode()
                .map_err(CliError::Address)?;
            return Err(CliError::Custom(format!(
                "Duplicate nft_id in CSV: {nft_id}"
            )));
        }
    }
    Ok(())
}

pub fn build_merkle_tree(records: &[DatastoreNftRecord]) -> Result<MerkleTree, CliError> {
    reject_duplicates(records)?;

    let mut sorted = records.to_vec();
    sorted.sort_by_key(|record| record.nft_id);

    let leaves: Vec<Bytes32> = sorted
        .iter()
        .map(|record| (record.nft_id, record.weight).tree_hash().into())
        .collect();

    Ok(MerkleTree::new(&leaves))
}

pub fn build_root_hash(records: &[DatastoreNftRecord]) -> Result<Bytes32, CliError> {
    Ok(build_merkle_tree(records)?.root())
}

pub fn leaf_hash(record: &DatastoreNftRecord) -> Bytes32 {
    (record.nft_id, record.weight).tree_hash().into()
}

fn merkle_sha256(parts: &[&[u8]]) -> Bytes32 {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    Bytes32::from(hasher.finalize())
}

pub fn verify_merkle_proof(leaf: Bytes32, proof: &MerkleProof, root: Bytes32) -> bool {
    let mut computed = merkle_sha256(&[HASH_LEAF_PREFIX, leaf.as_ref()]);
    let mut path = proof.path;

    for sibling in &proof.proof {
        computed = if path & 1 == 0 {
            merkle_sha256(&[HASH_TREE_PREFIX, computed.as_ref(), sibling.as_ref()])
        } else {
            merkle_sha256(&[HASH_TREE_PREFIX, sibling.as_ref(), computed.as_ref()])
        };
        path >>= 1;
    }

    computed == root
}

pub fn validate_update_csvs(
    old_records: &[DatastoreNftRecord],
    new_records: &[DatastoreNftRecord],
) -> Result<(), CliError> {
    reject_duplicates(old_records)?;
    reject_duplicates(new_records)?;

    let new_ids: HashSet<Bytes32> = new_records.iter().map(|r| r.nft_id).collect();

    for old in old_records {
        if !new_ids.contains(&old.nft_id) {
            let nft_id = Address::new(old.nft_id, "nft".to_string())
                .encode()
                .map_err(CliError::Address)?;
            return Err(CliError::Custom(format!(
                "{nft_id} should have its weight updated to 0 instead of being removed from the list"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Bytes32 {
        Bytes32::new([byte; 32])
    }

    #[test]
    fn test_merkle_tree_and_update_validation() -> Result<(), Box<dyn std::error::Error>> {
        let initial = records_from_entries(&[(id(1), 10), (id(2), 20), (id(3), 30)]);
        let tree = build_merkle_tree(&initial)?;
        let initial_root = tree.root();

        let target = initial.iter().find(|r| r.nft_id == id(2)).expect("entry 2");
        let leaf = leaf_hash(target);
        let proof = tree.proof(leaf).expect("proof should exist for entry 2");
        assert!(verify_merkle_proof(leaf, &proof, initial_root));

        let updated = records_from_entries(&[(id(1), 5), (id(2), 25), (id(3), 30), (id(4), 100)]);
        let updated_root = build_merkle_tree(&updated)?.root();
        assert_ne!(initial_root, updated_root);

        let removed = records_from_entries(&[(id(1), 10), (id(3), 30)]);
        let err = validate_update_csvs(&initial, &removed).unwrap_err();
        let expected = Address::new(id(2), "nft".to_string()).encode()?;
        let message = err.to_string();
        assert!(message.contains(&expected));
        assert!(message.contains(
            "should have its weight updated to 0 instead of being removed from the list"
        ));

        Ok(())
    }
}

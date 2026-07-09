use chia_bls::PublicKey;
use chia_protocol::{Bytes32, CoinSpend};
use chia_puzzle_types::standard::StandardArgs;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        DataStore, DataStoreMetadata, DelegatedPuzzle, Layer, Nft, OracleLayer, Puzzle,
        RewardDistributor, RewardDistributorStakeAction, RewardDistributorType, Spend,
        SpendContext,
    },
    types::{
        puzzles::{NonceWrapperArgs, DL_METADATA_UPDATER_PUZZLE_HASH, NONCE_WRAPPER_PUZZLE_HASH},
        MerkleProof, MerkleTree,
    },
    utils::Address,
};
use clvm_traits::clvm_tuple;
use clvm_utils::{CurriedProgram, ToTreeHash, TreeHash};

use crate::{
    build_merkle_tree, build_root_hash, hex_string_to_pubkey, leaf_hash, load_and_dedupe_csv,
    oracle_delegated_puzzles, CliError, DatastoreNftRecord, SageClient,
};

pub struct CustodyInfo {
    pub puzzle_hash: Bytes32,
    pub public_key: PublicKey,
    pub address: String,
}

pub struct CuratedDatastoreFields {
    pub dl_root_hash: Bytes32,
    pub dl_metadata_rest_hash: Option<Bytes32>,
    pub dl_metadata_updater_hash_hash: Bytes32,
    pub dl_inner_puzzle_hash: Bytes32,
}

pub struct NftMerkleEntry {
    pub weight: u64,
    pub proof: MerkleProof,
}

pub async fn resolve_custody(
    sage: &SageClient,
    custody_address: Option<String>,
) -> Result<CustodyInfo, CliError> {
    let (address, public_key) = if let Some(custody_address) = custody_address {
        if !sage.check_address(custody_address.clone()).await?.valid {
            return Err(CliError::Custom(
                "Custody address is not owned by the active wallet".to_string(),
            ));
        }
        let public_key = find_public_key_for_address(sage, &custody_address).await?;
        (custody_address, public_key)
    } else {
        let derivation = sage.get_derivations(false, 0, 1).await?.derivations[0].clone();
        (
            derivation.address.clone(),
            hex_string_to_pubkey(&derivation.public_key)?,
        )
    };

    let puzzle_hash = Address::decode(&address)?.puzzle_hash;
    if StandardArgs::curry_tree_hash(public_key) != puzzle_hash.into() {
        return Err(CliError::Custom(
            "Custody puzzle hash does not match the retrieved public key".to_string(),
        ));
    }

    Ok(CustodyInfo {
        puzzle_hash,
        public_key,
        address,
    })
}

async fn find_public_key_for_address(
    sage: &SageClient,
    address: &str,
) -> Result<PublicKey, CliError> {
    let mut offset = 0;
    let window_size = 100;
    loop {
        let derivations_resp = sage.get_derivations(false, offset, window_size).await?;
        if let Some(derivation) = derivations_resp
            .derivations
            .iter()
            .find(|d| d.address == address)
        {
            return hex_string_to_pubkey(&derivation.public_key);
        }

        if derivations_resp.total < window_size {
            break;
        }
        offset += window_size;
    }

    Err(CliError::Custom(
        "Could not find public key for custody address in wallet derivations".to_string(),
    ))
}

pub fn ensure_epoch_open(
    distributor: &RewardDistributor,
    latest_timestamp: u64,
) -> Result<(), CliError> {
    if latest_timestamp > distributor.info.state.round_time_info.epoch_end {
        return Err(CliError::Custom(
            "The current epoch has already ended - start a new epoch first".to_string(),
        ));
    }
    Ok(())
}

pub fn load_csv_matching_root(
    path: &str,
    expected_root: Bytes32,
) -> Result<Vec<DatastoreNftRecord>, CliError> {
    let records = load_and_dedupe_csv(path)?;
    let root = build_root_hash(&records)?;
    if root != expected_root {
        return Err(CliError::Custom(format!(
            "CSV root {} does not match on-chain whitelist root {} - run datastore update first or use the correct CSV",
            hex::encode(root),
            hex::encode(expected_root),
        )));
    }
    Ok(records)
}

pub fn merkle_proof_for_nft(
    records: &[DatastoreNftRecord],
    nft_launcher_id: Bytes32,
) -> Result<NftMerkleEntry, CliError> {
    let record = records
        .iter()
        .find(|r| r.nft_id == nft_launcher_id)
        .ok_or_else(|| {
            CliError::Custom(format!(
                "NFT {} is not in the whitelist CSV",
                Address::new(nft_launcher_id, "nft".to_string())
                    .encode()
                    .unwrap_or_else(|_| hex::encode(nft_launcher_id))
            ))
        })?;
    let tree = build_merkle_tree(records)?;
    let leaf = leaf_hash(record);
    let proof = tree
        .proof(leaf)
        .ok_or(CliError::Custom("Could not build merkle proof".to_string()))?;
    Ok(NftMerkleEntry {
        weight: record.weight,
        proof,
    })
}

pub fn curated_datastore_fields(
    datastore: &DataStore<DataStoreMetadata>,
    ctx: &mut SpendContext,
) -> Result<CuratedDatastoreFields, CliError> {
    let dl_metadata_rest_hash = datastore.info.metadata.label.as_ref().map(|label| {
        let description = datastore.info.metadata.description.as_deref().unwrap_or("");
        clvm_tuple!(("l", label.as_str()), ("d", description))
            .tree_hash()
            .into()
    });

    Ok(CuratedDatastoreFields {
        dl_root_hash: datastore.info.metadata.root_hash,
        dl_metadata_rest_hash,
        dl_metadata_updater_hash_hash: DL_METADATA_UPDATER_PUZZLE_HASH.tree_hash().into(),
        dl_inner_puzzle_hash: datastore
            .info
            .delegation_layer_puzzle_hash(ctx)
            .map_err(CliError::Driver)?
            .into(),
    })
}

pub fn oracle_datastore_inner_spend(
    ctx: &mut SpendContext,
    delegated_puzzles: &[DelegatedPuzzle],
) -> Result<Spend, CliError> {
    let oracle_layer = match delegated_puzzles.first() {
        Some(DelegatedPuzzle::Oracle(oracle_puzzle_hash, oracle_fee)) => {
            OracleLayer::new(*oracle_puzzle_hash, *oracle_fee)
                .ok_or(CliError::Custom("Invalid oracle fee".to_string()))?
        }
        _ => {
            return Err(CliError::Custom(
                "Datastore must have an oracle delegated puzzle".to_string(),
            ));
        }
    };
    oracle_layer
        .construct_spend(ctx, ())
        .map_err(CliError::Driver)
}

pub fn spend_datastore_oracle(
    ctx: &mut SpendContext,
    datastore: DataStore<DataStoreMetadata>,
    delegated_puzzles: &[DelegatedPuzzle],
) -> Result<CoinSpend, CliError> {
    let inner_spend = oracle_datastore_inner_spend(ctx, delegated_puzzles)?;
    let dl_spend = datastore
        .spend(ctx, inner_spend)
        .map_err(CliError::Driver)?;
    Ok(dl_spend)
}

pub fn format_cat_mojos(mojos: u64) -> String {
    format!("{:.3} CATs ({} mojos)", mojos as f64 / 1000.0, mojos)
}

pub fn format_precision_amount(amount: u128, precision: u64) -> String {
    let cat_mojos = amount / u128::from(precision);
    format!(
        "{:.3} CATs ({} CAT mojos)",
        cat_mojos as f64 / 1000.0,
        cat_mojos
    )
}

pub fn payout_amount_after_precision(amount: u64, precision: u64) -> f64 {
    (amount / precision) as f64 / 1000.0
}

pub fn locked_nft_p2_puzzle_hash(
    custody_puzzle_hash: Bytes32,
    launcher_id: Bytes32,
    shares: u64,
) -> Bytes32 {
    CurriedProgram {
        program: NONCE_WRAPPER_PUZZLE_HASH,
        args: NonceWrapperArgs::<(Bytes32, u64), TreeHash> {
            nonce: clvm_tuple!(custody_puzzle_hash, shares),
            inner_puzzle: RewardDistributorStakeAction::my_p2_puzzle_hash(launcher_id).into(),
        },
    }
    .tree_hash()
    .into()
}

pub async fn find_locked_nfts(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    launcher_id: Bytes32,
    custody_puzzle_hash: Bytes32,
    distributor_type: RewardDistributorType,
    entry_shares: u64,
) -> Result<Vec<(Nft, u64)>, CliError> {
    let search_shares = match distributor_type {
        RewardDistributorType::NftCollection { .. } => vec![1],
        RewardDistributorType::CuratedNft { .. } => (0..=entry_shares.max(1)).collect(),
        _ => return Ok(Vec::new()),
    };

    let mut locked_nfts = Vec::new();
    for shares in search_shares {
        let locked_nft_hint = locked_nft_p2_puzzle_hash(custody_puzzle_hash, launcher_id, shares);

        let possible_locked_nft_coins = client
            .get_coin_records_by_hint(locked_nft_hint, None, None, Some(false), None)
            .await?
            .coin_records
            .unwrap_or_default();

        for coin_record in possible_locked_nft_coins {
            let parent_coin_spend = client
                .get_puzzle_and_solution(
                    coin_record.coin.parent_coin_info,
                    Some(coin_record.confirmed_block_index),
                )
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotFound(coin_record.coin.parent_coin_info))?;

            let parent_puzzle = ctx.alloc(&parent_coin_spend.puzzle_reveal)?;
            let parent_puzzle = Puzzle::parse(ctx, parent_puzzle);
            let parent_solution = ctx.alloc(&parent_coin_spend.solution)?;

            if let Ok(Some(nft)) =
                Nft::parse_child(ctx, parent_coin_spend.coin, parent_puzzle, parent_solution)
            {
                if nft.info.p2_puzzle_hash == locked_nft_hint {
                    locked_nfts.push((nft, shares));
                }
            }
        }
    }

    Ok(locked_nfts)
}

pub async fn find_locked_cat(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    launcher_id: Bytes32,
    custody_puzzle_hash: Bytes32,
    asset_id: Bytes32,
    entry_shares: u64,
) -> Result<chia_wallet_sdk::driver::Cat, CliError> {
    use chia_wallet_sdk::driver::Cat;

    for shares in (1..=entry_shares.max(1)).rev() {
        let locked_cat_hint = locked_nft_p2_puzzle_hash(custody_puzzle_hash, launcher_id, shares);

        let possible_locked_cat_coins = client
            .get_coin_records_by_hint(locked_cat_hint, None, None, Some(false), None)
            .await?
            .coin_records
            .unwrap_or_default();

        for coin_record in possible_locked_cat_coins {
            if coin_record.coin.amount != shares {
                continue;
            }

            let parent_coin_spend = client
                .get_puzzle_and_solution(
                    coin_record.coin.parent_coin_info,
                    Some(coin_record.confirmed_block_index),
                )
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotFound(coin_record.coin.parent_coin_info))?;

            let parent_puzzle_ptr = ctx.alloc(&parent_coin_spend.puzzle_reveal)?;
            let parent_puzzle = Puzzle::parse(ctx, parent_puzzle_ptr);
            let parent_solution_ptr = ctx.alloc(&parent_coin_spend.solution)?;

            let Some(cats) = Cat::parse_children(
                ctx,
                parent_coin_spend.coin,
                parent_puzzle,
                parent_solution_ptr,
            )?
            else {
                continue;
            };
            if let Some(cat) = cats
                .into_iter()
                .find(|cat| cat.coin == coin_record.coin && cat.info.asset_id == asset_id)
            {
                return Ok(cat);
            }
        }
    }

    Err(CliError::Custom(
        "No locked CAT found - you may be using the wrong custody address/puzzle hash".to_string(),
    ))
}

pub fn delegated_puzzles() -> Vec<DelegatedPuzzle> {
    oracle_delegated_puzzles()
}

#[allow(clippy::result_large_err)]
pub fn build_merkle_tree_from_records(
    records: &[DatastoreNftRecord],
) -> Result<MerkleTree, CliError> {
    build_merkle_tree(records)
}

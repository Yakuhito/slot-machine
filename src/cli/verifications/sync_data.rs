use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
};
use chia_puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{Puzzle, SpendContext},
};
use clvmr::serde::node_from_bytes;

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CliError, UniquenessPrelauncher, VerifiedData,
};

pub async fn get_data_for_asset_id(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    asset_id: Bytes32,
    testnet11: bool,
) -> Result<VerifiedData, CliError> {
    let prelauncher_puzzle_hash: Bytes32 =
        UniquenessPrelauncher::puzzle_hash(asset_id.tree_hash().into()).into();

    let possible_prelaunchers = client
        .get_coin_records_by_puzzle_hash(prelauncher_puzzle_hash, None, None, Some(true))
        .await?
        .coin_records
        .ok_or(CliError::PuzzleHashRecordsNotFound(prelauncher_puzzle_hash))?;

    let prelauncher_coin_id: Option<Bytes32> = None;
    for possible_prelauncher_record in possible_prelaunchers {
        // if child exists, parent must be spent
        let catalog_spend_maybe = client
            .get_puzzle_and_solution(
                possible_prelauncher_record.coin.parent_coin_info,
                Some(possible_prelauncher_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(
                possible_prelauncher_record.coin.parent_coin_info,
            ))?;

        let mut temp_ctx = SpendContext::new();
        let puzzle_ptr = temp_ctx.alloc(&catalog_spend_maybe.puzzle_reveal)?;
        let puzzle = Puzzle::parse(&temp_ctx, puzzle_ptr);
        let solution_ptr = temp_ctx.alloc(&catalog_spend_maybe.puzzle_reveal)?;
        if let Ok(Some(_reg)) = CatalogRegistry::from_parent_spend(
            &mut temp_ctx,
            catalog_spend_maybe.coin,
            puzzle,
            solution_ptr,
            CatalogRegistryConstants::get(testnet11),
        ) {
            prelauncher_coin_id = Some(catalog_spend_maybe.coin.coin_id());
            break;
        }
    }

    let prelauncher_coin_id = prelauncher_coin_id.ok_or(CliError::Custom(format!(
        "Could not find prelauncher associated to asset id {}",
        hex::encode(asset_id)
    )))?;

    let nft_launcher_id =
        Coin::new(prelauncher_coin_id, SINGLETON_LAUNCHER_HASH.into(), 1).coin_id();

    todo!()
}

use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
};
use chia_puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{DriverError, Nft, Puzzle, SpendContext},
};

use crate::{
    CatNftMetadata, CatalogRegistry, CatalogRegistryConstants, CliError, UniquenessPrelauncher,
};

pub async fn get_latest_data_for_asset_id(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    asset_id: Bytes32,
    testnet11: bool,
) -> Result<CatNftMetadata, CliError> {
    let prelauncher_puzzle_hash: Bytes32 =
        UniquenessPrelauncher::<Bytes32>::puzzle_hash(asset_id.tree_hash()).into();

    let possible_prelaunchers = client
        .get_coin_records_by_puzzle_hash(prelauncher_puzzle_hash, None, None, Some(true))
        .await?
        .coin_records
        .ok_or(CliError::PuzzleHashRecordsNotFound(prelauncher_puzzle_hash))?;

    let mut prelauncher_coin_id: Option<Bytes32> = None;
    for possible_prelauncher_record in possible_prelaunchers {
        println!(
            "PROCESSING PRELAUNCHER {}",
            possible_prelauncher_record.confirmed_block_index
        ); // todo: debug
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

        let puzzle_ptr = ctx.alloc(&catalog_spend_maybe.puzzle_reveal)?;
        let puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = ctx.alloc(&catalog_spend_maybe.solution)?;
        if let Ok(Some(_reg)) = CatalogRegistry::from_parent_spend(
            ctx,
            catalog_spend_maybe.coin,
            puzzle,
            solution_ptr,
            CatalogRegistryConstants::get(testnet11),
        ) {
            println!("FOUND REGISTRY"); // todo: debug
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

    let possible_nft_coins = client
        .get_coin_records_by_hint(nft_launcher_id, None, None, Some(false))
        .await?
        .coin_records
        .ok_or(CliError::Driver(DriverError::MissingHint))?;

    for possible_nft_coin in possible_nft_coins {
        if possible_nft_coin.spent {
            continue;
        }

        let parent_spend = client
            .get_puzzle_and_solution(
                possible_nft_coin.coin.parent_coin_info,
                Some(possible_nft_coin.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(
                possible_nft_coin.coin.parent_coin_info,
            ))?;

        let puzzle_ptr = ctx.alloc(&parent_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = ctx.alloc(&parent_spend.puzzle_reveal)?;

        if let Ok(Some(nft)) =
            Nft::<CatNftMetadata>::parse_child(ctx, possible_nft_coin.coin, puzzle, solution_ptr)
        {
            if nft.info.launcher_id == nft_launcher_id {
                return Ok(nft.info.metadata);
            }
        }
    }

    Err(CliError::Custom(format!(
        "Could not find NFT associated to asset id {}",
        hex::encode(asset_id)
    )))
}

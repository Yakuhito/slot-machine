use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
};
use chia_puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{Nft, Puzzle, SpendContext},
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
            prelauncher_coin_id = Some(possible_prelauncher_record.coin.coin_id());
            break;
        }
    }

    let prelauncher_coin_id = prelauncher_coin_id.ok_or(CliError::Custom(format!(
        "Could not find prelauncher associated to asset id {}",
        hex::encode(asset_id)
    )))?;

    let nft_launcher_id =
        Coin::new(prelauncher_coin_id, SINGLETON_LAUNCHER_HASH.into(), 1).coin_id();
    println!("NFT launcher id: {}", hex::encode(nft_launcher_id));

    let mut next_nft_record = Some(
        client
            .get_coin_records_by_parent_ids(vec![nft_launcher_id], None, None, Some(true))
            .await?
            .coin_records
            .ok_or(CliError::CoinNotSpent(nft_launcher_id))?[0],
    );
    let mut latest_nft = None;

    while let Some(nft_record) = next_nft_record {
        if !nft_record.spent {
            break;
        }

        let coin_spend = client
            .get_puzzle_and_solution(
                nft_record.coin.coin_id(),
                Some(nft_record.spent_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(nft_record.coin.coin_id()))?;

        let puzzle_ptr = ctx.alloc(&coin_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = ctx.alloc(&coin_spend.solution)?;
        if let Ok(Some(nft)) =
            Nft::<CatNftMetadata>::parse_child(ctx, nft_record.coin, puzzle, solution_ptr)
        {
            next_nft_record = client
                .get_coin_record_by_name(nft.coin.coin_id())
                .await?
                .coin_record;
            latest_nft = Some(nft);
        } else {
            break;
        }
    }

    let latest_nft = latest_nft.ok_or(CliError::Custom(format!(
        "Could not find NFT associated to asset id {}",
        hex::encode(asset_id)
    )))?;

    Ok(latest_nft.info.metadata)
}

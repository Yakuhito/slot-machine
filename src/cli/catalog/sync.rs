use chia::{
    protocol::{Bytes32, Coin},
    puzzles::singleton::LauncherSolution,
};
use chia_wallet_sdk::{ChiaRpcClient, CoinsetClient, DriverError, Puzzle, SpendContext};
use clvm_traits::FromClvm;
use clvmr::NodePtr;

use crate::{CatalogRegistry, CatalogRegistryConstants, CliError, Db, CATALOG_LAST_UNSPENT_COIN};

pub async fn sync_catalog(
    client: &CoinsetClient,
    db: &Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    constants: CatalogRegistryConstants,
) -> Result<CatalogRegistry, CliError> {
    let last_unspent_coin_id_str = db.get_value_by_key(CATALOG_LAST_UNSPENT_COIN).await?;

    let mut last_coin_id: Bytes32 = if let Some(last_unspent_coin_id_str) = last_unspent_coin_id_str
    {
        let last_unspent_coin_id = Bytes32::new(
            hex::decode(last_unspent_coin_id_str)?
                .to_vec()
                .try_into()
                .unwrap(),
        );

        let coin_record_response = client.get_coin_record_by_name(last_unspent_coin_id).await?;
        if let Some(coin_record) = coin_record_response.coin_record {
            coin_record.coin.parent_coin_info
        } else {
            return Err(CliError::Driver(DriverError::Custom(
                "Could not find latest unspent coin".to_string(),
            )));
        }
    } else {
        launcher_id
    };

    let mut catalog: Option<CatalogRegistry> = None;
    loop {
        println!("coin record by name");
        let coin_record_response = client.get_coin_record_by_name(last_coin_id).await?;
        let Some(coin_record) = coin_record_response.coin_record else {
            break;
        };
        if !coin_record.spent {
            return Err(CliError::Driver(DriverError::Custom(
                "CATalog coin that should be spent is unspent".to_string(),
            )));
        }

        let puzzle_and_solution_resp = client
            .get_puzzle_and_solution(
                coin_record.coin.coin_id(),
                Some(coin_record.confirmed_block_index),
            )
            .await?;
        let coin_spend = puzzle_and_solution_resp
            .coin_solution
            .ok_or(CliError::Driver(DriverError::Custom(
                "Could not find puzzle and solution for a coin that should be spent".to_string(),
            )))?;

        let puzzle_ptr = ctx.alloc(&coin_spend.puzzle_reveal)?;
        let parent_puzzle = Puzzle::parse(&ctx.allocator, puzzle_ptr);
        let solution_ptr = ctx.alloc(&coin_spend.solution)?;
        if let Some(ref prev_catalog) = catalog {
            let new_slots = prev_catalog.get_new_slots_from_spend(ctx, solution_ptr)?;

            for slot in new_slots {
                let asset_id = slot.info.value.unwrap().asset_id;

                if let Some(previous_value_hash) =
                    db.get_catalog_indexed_slot_value(asset_id).await?
                {
                    db.remove_slot(launcher_id, 0, previous_value_hash).await?;
                }

                db.save_slot(&mut ctx.allocator, slot).await?;
                db.save_catalog_indexed_slot_value(asset_id, slot.info.value_hash)
                    .await?;
            }
        }

        if let Some(some_catalog) = CatalogRegistry::from_parent_spend(
            &mut ctx.allocator,
            coin_record.coin,
            parent_puzzle,
            solution_ptr,
            constants,
        )? {
            last_coin_id = some_catalog.coin.coin_id();
            catalog = Some(some_catalog);
        } else if coin_record.coin.coin_id() == launcher_id {
            let solution = LauncherSolution::<NodePtr>::from_clvm(&ctx.allocator, solution_ptr)
                .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;
            last_coin_id = Coin::new(launcher_id, solution.singleton_puzzle_hash, 1).coin_id();
        } else {
            println!("Breaking");
            break;
        };
    }

    db.save_key_value(
        CATALOG_LAST_UNSPENT_COIN,
        &hex::encode(last_coin_id.to_vec()),
    )
    .await?;

    Ok(catalog.unwrap())
}

use chia::{
    clvm_utils::{tree_hash, ToTreeHash},
    protocol::{Bytes32, Coin},
    puzzles::{singleton::LauncherSolution, LineageProof, Proof},
};
use chia_wallet_sdk::{
    ChiaRpcClient, CoinRecord, CoinsetClient, Condition, Conditions, DriverError, Layer, Puzzle,
    SingletonLayer, SpendContext,
};
use clvm_traits::FromClvm;
use clvmr::NodePtr;

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState,
    CatalogSlotValue, CliError, Db, Slot, SlotInfo, SlotProof, SLOT32_MAX_VALUE, SLOT32_MIN_VALUE,
};

pub async fn sync_catalog(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    constants: CatalogRegistryConstants,
) -> Result<CatalogRegistry, CliError> {
    let last_unspent_coin_info = db
        .get_last_unspent_singleton_coin(constants.launcher_id)
        .await?;

    let mut last_coin_id: Bytes32 = if let Some((_coin_id, parent_coin_id)) = last_unspent_coin_info
    {
        parent_coin_id
    } else {
        constants.launcher_id
    };

    let mut catalog: Option<CatalogRegistry> = None;
    loop {
        let coin_record_response = client.get_coin_record_by_name(last_coin_id).await?;
        let Some(coin_record) = coin_record_response.coin_record else {
            return Err(CliError::CoinNotFound(last_coin_id));
        };
        if !coin_record.spent {
            break;
        }
        db.save_singleton_coin(constants.launcher_id, coin_record)
            .await?;

        let puzzle_and_solution_resp = client
            .get_puzzle_and_solution(
                coin_record.coin.coin_id(),
                Some(coin_record.spent_block_index),
            )
            .await?;
        let Some(coin_spend) = puzzle_and_solution_resp.coin_solution else {
            return Err(CliError::CoinNotSpent(last_coin_id));
        };

        let puzzle_ptr = ctx.alloc(&coin_spend.puzzle_reveal)?;
        let parent_puzzle = Puzzle::parse(&ctx.allocator, puzzle_ptr);
        let solution_ptr = ctx.alloc(&coin_spend.solution)?;
        if let Some(ref prev_catalog) = catalog {
            let new_slots = prev_catalog.get_new_slots_from_spend(ctx, solution_ptr)?;

            for slot in new_slots {
                let asset_id = slot.info.value.asset_id;

                if let Some(previous_value_hash) =
                    db.get_catalog_indexed_slot_value(asset_id).await?
                {
                    db.mark_slot_as_spent(
                        constants.launcher_id,
                        0,
                        previous_value_hash,
                        coin_record.spent_block_index,
                    )
                    .await?;
                }

                db.save_slot(&mut ctx.allocator, slot, None).await?;
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
        } else if coin_record.coin.coin_id() == constants.launcher_id {
            let solution = LauncherSolution::<NodePtr>::from_clvm(&ctx.allocator, solution_ptr)
                .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;
            let catalog_eve_coin =
                Coin::new(constants.launcher_id, solution.singleton_puzzle_hash, 1);
            let catalog_eve_coin_id = catalog_eve_coin.coin_id();

            let eve_coin_puzzle_and_solution_resp = client
                .get_puzzle_and_solution(
                    catalog_eve_coin_id,
                    Some(coin_record.confirmed_block_index),
                )
                .await?;
            let Some(eve_coin_spend) = eve_coin_puzzle_and_solution_resp.coin_solution else {
                break;
            };

            let eve_coin_puzzle_ptr = ctx.alloc(&eve_coin_spend.puzzle_reveal)?;
            let eve_coin_puzzle = Puzzle::parse(&ctx.allocator, eve_coin_puzzle_ptr);
            let Some(eve_coin_puzzle) =
                SingletonLayer::<NodePtr>::parse_puzzle(&ctx.allocator, eve_coin_puzzle)?
            else {
                break;
            };

            let eve_coin_inner_puzzle_hah = tree_hash(&ctx.allocator, eve_coin_puzzle.inner_puzzle);

            let eve_coin_solution_ptr = ctx.alloc(&eve_coin_spend.solution)?;
            let eve_coin_output = ctx.run(eve_coin_puzzle_ptr, eve_coin_solution_ptr)?;
            let eve_coin_output = ctx.extract::<Conditions<NodePtr>>(eve_coin_output)?;

            let Some(Condition::CreateCoin(odd_create_coin)) =
                eve_coin_output.into_iter().find(|c| {
                    if let Condition::CreateCoin(create_coin) = c {
                        // singletons with amount != 1 are weird and I don't support them
                        create_coin.amount % 2 == 1
                    } else {
                        false
                    }
                })
            else {
                break;
            };

            let (decoded_launcher_id, (_decoded_asset_id, (initial_state, ()))) =
                <(Bytes32, (Bytes32, (CatalogRegistryState, ())))>::from_clvm(
                    &ctx.allocator,
                    odd_create_coin.memos.unwrap().value,
                )
                .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;
            if decoded_launcher_id != constants.launcher_id {
                break;
            }

            let new_coin = Coin::new(
                catalog_eve_coin_id,
                odd_create_coin.puzzle_hash,
                odd_create_coin.amount,
            );
            let lineage_proof = LineageProof {
                parent_parent_coin_info: eve_coin_spend.coin.parent_coin_info,
                parent_inner_puzzle_hash: eve_coin_inner_puzzle_hah.into(),
                parent_amount: eve_coin_spend.coin.amount,
            };
            let new_catalog = CatalogRegistry::new(
                new_coin,
                Proof::Lineage(lineage_proof),
                CatalogRegistryInfo::new(initial_state, constants),
            );

            let slot_proof = SlotProof {
                parent_parent_info: lineage_proof.parent_parent_coin_info,
                parent_inner_puzzle_hash: lineage_proof.parent_inner_puzzle_hash,
            };
            let left_slot_value = CatalogSlotValue::left_end(SLOT32_MAX_VALUE.into());
            let right_slot_value = CatalogSlotValue::right_end(SLOT32_MIN_VALUE.into());

            db.save_slot(
                &mut ctx.allocator,
                Slot::new(
                    slot_proof,
                    SlotInfo::from_value(constants.launcher_id, 0, left_slot_value),
                ),
                None,
            )
            .await?;
            db.save_catalog_indexed_slot_value(
                left_slot_value.asset_id,
                left_slot_value.tree_hash().into(),
            )
            .await?;

            db.save_slot(
                &mut ctx.allocator,
                Slot::new(
                    slot_proof,
                    SlotInfo::from_value(constants.launcher_id, 0, right_slot_value),
                ),
                None,
            )
            .await?;
            db.save_catalog_indexed_slot_value(
                right_slot_value.asset_id,
                right_slot_value.tree_hash().into(),
            )
            .await?;

            db.save_singleton_coin(
                constants.launcher_id,
                CoinRecord {
                    coin: new_coin,
                    coinbase: false,
                    confirmed_block_index: coin_record.spent_block_index,
                    spent: false,
                    spent_block_index: 0,
                    timestamp: 0,
                },
            )
            .await?;

            last_coin_id = new_catalog.coin.coin_id();
            catalog = Some(new_catalog);
        } else if coin_record.coin.parent_coin_info == constants.launcher_id {
            last_coin_id = constants.launcher_id;
        } else {
            break;
        };

        db.finish_transaction().await?;
    }

    if let Some(catalog) = catalog {
        Ok(catalog)
    } else {
        Err(CliError::CoinNotFound(last_coin_id))
    }
}

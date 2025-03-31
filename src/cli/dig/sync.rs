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

use crate::{CliError, Db, DigRewardDistributor, DigSlotNonce};

pub async fn sync_distributor(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<DigRewardDistributor, CliError> {
    let (last_spent_coin_id, constants) = if let Some(constants_from_db) = db
        .get_reward_distributor_configuration(&mut ctx.allocator, launcher_id)
        .await?
    {
        let last_unspent_coin_info = db.get_last_unspent_singleton_coin(launcher_id).await?;
        let last_spent_coin_id_from_db =
            if let Some((_coin_id, parent_coin_id)) = last_unspent_coin_info {
                parent_coin_id
            } else {
                constants_from_db.launcher_id
            };

        (last_spent_coin_id_from_db, constants_from_db)
    } else {
        let solution = LauncherSolution::<NodePtr>::from_clvm(&ctx.allocator, solution_ptr)
            .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;
        let catalog_eve_coin = Coin::new(constants.launcher_id, solution.singleton_puzzle_hash, 1);
        let catalog_eve_coin_id = catalog_eve_coin.coin_id();

        let eve_coin_puzzle_and_solution_resp = client
            .get_puzzle_and_solution(catalog_eve_coin_id, Some(coin_record.confirmed_block_index))
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

        let Some(Condition::CreateCoin(odd_create_coin)) = eve_coin_output.into_iter().find(|c| {
            if let Condition::CreateCoin(create_coin) = c {
                // singletons with amount != 1 are weird and I don't support them
                create_coin.amount % 2 == 1
            } else {
                false
            }
        }) else {
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
            0,
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
            0,
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
    };

    let mut last_coin_id = last_spent_coin_id;
    let mut distributor: Option<DigRewardDistributor> = None;
    loop {
        let coin_record_response = client.get_coin_record_by_name(last_coin_id).await?;
        let Some(coin_record) = coin_record_response.coin_record else {
            return Err(CliError::CoinNotFound(last_coin_id));
        };
        if !coin_record.spent {
            break;
        }

        let skip_db_save = last_coin_id == last_spent_coin_id;
        if !skip_db_save {
            db.save_singleton_coin(constants.launcher_id, coin_record)
                .await?;
        }

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
        if !skip_db_save {
            if let Some(ref prev_distributor) = distributor {
                let pending_items =
                    prev_distributor.get_pending_items_from_spend(ctx, solution_ptr)?;

                for (nonce, value_hash) in pending_items.pending_spent_slots {
                    db.mark_slot_as_spent(
                        constants.launcher_id,
                        nonce.to_u64(),
                        value_hash,
                        coin_record.spent_block_index,
                    )
                    .await?;

                    db.delete_dig_indexed_slot_values_by_epoch_start_using_value_hash(value_hash)
                        .await?;
                    db.delete_dig_indexed_slot_values_by_puzzle_hash_using_value_hash(value_hash)
                        .await?;
                }

                for slot in prev_distributor.created_slot_values_to_slots(
                    pending_items.pending_commitment_slot_values,
                    DigSlotNonce::COMMITMENT,
                ) {
                    db.save_slot(&mut ctx.allocator, slot, 0).await?;
                    db.save_dig_indexed_slot_value_by_epoch_start(
                        slot.info.value.epoch_start,
                        slot.info.value_hash,
                    )
                    .await?;
                    db.save_dig_indexed_slot_value_by_puzzle_hash(
                        slot.info.value.clawback_ph,
                        slot.info.value_hash,
                    )
                    .await?;
                }

                for slot in prev_distributor.created_slot_values_to_slots(
                    pending_items.pending_mirror_slot_values,
                    DigSlotNonce::MIRROR,
                ) {
                    db.save_slot(&mut ctx.allocator, slot, 0).await?;
                    db.save_dig_indexed_slot_value_by_puzzle_hash(
                        slot.info.value.payout_puzzle_hash,
                        slot.info.value_hash,
                    )
                    .await?;
                }

                for slot in prev_distributor.created_slot_values_to_slots(
                    pending_items.pending_reward_slot_values,
                    DigSlotNonce::REWARD,
                ) {
                    db.save_slot(&mut ctx.allocator, slot, 0).await?;
                    db.save_dig_indexed_slot_value_by_epoch_start(
                        slot.info.value.epoch_start,
                        slot.info.value_hash,
                    )
                    .await?;
                }
            }
        }

        if let Some(some_distributor) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            coin_record.coin,
            parent_puzzle,
            solution_ptr,
            constants,
        )? {
            last_coin_id = some_distributor.coin.coin_id();
            distributor = Some(some_distributor);
        } else {
            break;
        };
    }

    if let Some(distributor) = distributor {
        Ok(distributor)
    } else {
        Err(CliError::CoinNotFound(last_coin_id))
    }
}

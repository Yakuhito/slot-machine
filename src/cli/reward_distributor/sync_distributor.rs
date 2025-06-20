use chia::{
    clvm_utils::{tree_hash, ToTreeHash},
    protocol::{Bytes32, Coin},
    puzzles::{
        cat::CatArgs,
        singleton::{LauncherSolution, SingletonArgs, SingletonStruct},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{CatLayer, Layer, Puzzle, SingletonLayer, SpendContext},
    types::{Condition, Conditions},
};
use clvmr::NodePtr;

use crate::{
    CliError, Db, P2DelegatedBySingletonLayerArgs, Reserve, RewardDistributor,
    RewardDistributorConstants, RewardDistributorInfo, RewardDistributorRewardSlotValue,
    RewardDistributorSlotNonce, RewardDistributorState, Slot, SlotInfo, SlotProof,
};

pub async fn sync_distributor(
    client: &CoinsetClient,
    db: &Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<RewardDistributor, CliError> {
    let last_unspent_coin_info = db.get_last_unspent_singleton_coin(launcher_id).await?;

    let (last_spent_coin_id, constants, mut skip_db_save, prev_distributor) =
        if let Some((_coin_id, parent_coin_id)) = last_unspent_coin_info {
            let constants_from_db = db
                .get_reward_distributor_configuration(ctx, launcher_id)
                .await?
                .ok_or(CliError::Custom(
                    "Reward distributor configuration not found in database".to_string(),
                ))?;

            (parent_coin_id, constants_from_db, true, None)
        } else {
            let Some(launcher_coin_record) = client
                .get_coin_record_by_name(launcher_id)
                .await?
                .coin_record
            else {
                return Err(CliError::CoinNotFound(launcher_id));
            };
            if !launcher_coin_record.spent {
                return Err(CliError::CoinNotSpent(launcher_id));
            }

            let Some(launcher_coin_spend) = client
                .get_puzzle_and_solution(
                    launcher_coin_record.coin.coin_id(),
                    Some(launcher_coin_record.spent_block_index),
                )
                .await?
                .coin_solution
            else {
                return Err(CliError::CoinNotSpent(launcher_id));
            };

            let launcher_solution_ptr = ctx.alloc(&launcher_coin_spend.solution)?;
            let launcher_solution = ctx
                .extract::<LauncherSolution<(u64, RewardDistributorConstants)>>(
                    launcher_solution_ptr,
                )?;

            let distributor_eve_coin =
                Coin::new(launcher_id, launcher_solution.singleton_puzzle_hash, 1);
            let distributor_eve_coin_id = distributor_eve_coin.coin_id();

            let Some(distributor_eve_coin_spend) = client
                .get_puzzle_and_solution(
                    distributor_eve_coin_id,
                    Some(launcher_coin_record.spent_block_index),
                )
                .await?
                .coin_solution
            else {
                return Err(CliError::CoinNotSpent(distributor_eve_coin_id));
            };

            let eve_coin_puzzle_ptr = ctx.alloc(&distributor_eve_coin_spend.puzzle_reveal)?;
            let eve_coin_puzzle = Puzzle::parse(ctx, eve_coin_puzzle_ptr);
            let Some(eve_coin_puzzle) =
                SingletonLayer::<NodePtr>::parse_puzzle(ctx, eve_coin_puzzle)?
            else {
                return Err(CliError::Custom("Eve coin not a singleton".to_string()));
            };

            let eve_coin_inner_puzzle_hash = tree_hash(ctx, eve_coin_puzzle.inner_puzzle);

            let eve_coin_solution_ptr = ctx.alloc(&distributor_eve_coin_spend.solution)?;
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
                return Err(CliError::Custom(
                    "Eve coin did not create a coin".to_string(),
                ));
            };

            let first_epoch_start = launcher_solution.key_value_list.0;
            let initial_state = RewardDistributorState::initial(first_epoch_start);
            let constants = launcher_solution.key_value_list.1;
            if constants != constants.with_launcher_id(launcher_id) {
                return Err(CliError::Custom(
                    "Distributor constants invalid".to_string(),
                ));
            }

            let new_coin = Coin::new(
                distributor_eve_coin_id,
                odd_create_coin.puzzle_hash,
                odd_create_coin.amount,
            );
            let lineage_proof = LineageProof {
                parent_parent_coin_info: distributor_eve_coin.parent_coin_info,
                parent_inner_puzzle_hash: eve_coin_inner_puzzle_hash.into(),
                parent_amount: distributor_eve_coin.amount,
            };
            let reserve = find_reserve(
                ctx,
                client,
                launcher_id,
                constants.reserve_asset_id,
                0,
                0,
                true,
            )
            .await?;
            let new_distributor = RewardDistributor::new(
                new_coin,
                Proof::Lineage(lineage_proof),
                RewardDistributorInfo::new(initial_state, constants),
                reserve,
            );

            if SingletonArgs::curry_tree_hash(
                constants.launcher_id,
                new_distributor.info.inner_puzzle_hash(),
            ) != new_distributor.coin.puzzle_hash.into()
            {
                return Err(CliError::Custom(
                    "Distributor singleton puzzle hash mismatch".to_string(),
                ));
            }

            let slot_proof = SlotProof {
                parent_parent_info: lineage_proof.parent_parent_coin_info,
                parent_inner_puzzle_hash: lineage_proof.parent_inner_puzzle_hash,
            };
            let slot_value = RewardDistributorRewardSlotValue {
                epoch_start: first_epoch_start,
                next_epoch_initialized: false,
                rewards: 0,
            };

            db.save_slot(
                ctx,
                Slot::new(
                    slot_proof,
                    SlotInfo::from_value(
                        constants.launcher_id,
                        RewardDistributorSlotNonce::REWARD.to_u64(),
                        slot_value,
                    ),
                ),
                0,
            )
            .await?;
            db.save_dig_indexed_slot_value_by_epoch_start(
                slot_value.epoch_start,
                RewardDistributorSlotNonce::REWARD.to_u64(),
                slot_value.tree_hash().into(),
            )
            .await?;
            db.save_reward_distributor_configuration(ctx, constants.launcher_id, constants)
                .await?;

            let Some(distributor_record) = client
                .get_coin_record_by_name(new_distributor.coin.coin_id())
                .await?
                .coin_record
            else {
                return Err(CliError::CoinNotFound(new_distributor.coin.coin_id()));
            };
            if !distributor_record.spent {
                return Ok(new_distributor);
            }

            (
                new_distributor.coin.coin_id(),
                new_distributor.info.constants,
                false,
                Some(new_distributor),
            )
        };

    let mut last_coin_id = last_spent_coin_id;
    let mut distributor: Option<RewardDistributor> = prev_distributor;
    loop {
        let coin_record_response = client.get_coin_record_by_name(last_coin_id).await?;
        let Some(coin_record) = coin_record_response.coin_record else {
            return Err(CliError::CoinNotFound(last_coin_id));
        };
        if !coin_record.spent {
            break;
        }

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
        let parent_puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = ctx.alloc(&coin_spend.solution)?;
        if !skip_db_save {
            if let Some(ref prev_distributor) = distributor {
                let pending_items =
                    prev_distributor.get_pending_items_from_spend(ctx, solution_ptr)?;

                for (nonce, value_hash) in pending_items.pending_spent_slots.iter() {
                    db.mark_slot_as_spent(
                        constants.launcher_id,
                        nonce.to_u64(),
                        *value_hash,
                        coin_record.spent_block_index,
                    )
                    .await?;

                    db.delete_dig_indexed_slot_values_by_epoch_start_using_value_hash(*value_hash)
                        .await?;
                    db.delete_dig_indexed_slot_values_by_puzzle_hash_using_value_hash(*value_hash)
                        .await?;
                }

                for slot in prev_distributor.created_slot_values_to_slots(
                    pending_items.pending_commitment_slot_values,
                    RewardDistributorSlotNonce::COMMITMENT,
                ) {
                    let spent_block_index = if pending_items.pending_spent_slots.contains(&(
                        RewardDistributorSlotNonce::from_u64(slot.info.nonce).unwrap(),
                        slot.info.value_hash,
                    )) {
                        coin_record.spent_block_index
                    } else {
                        0
                    };

                    if spent_block_index == 0 {
                        // ephemeral slot for this spend
                        db.save_dig_indexed_slot_value_by_puzzle_hash(
                            slot.info.value.clawback_ph,
                            RewardDistributorSlotNonce::COMMITMENT.to_u64(),
                            slot.info.value_hash,
                        )
                        .await?;
                        db.save_dig_indexed_slot_value_by_epoch_start(
                            slot.info.value.epoch_start,
                            RewardDistributorSlotNonce::COMMITMENT.to_u64(),
                            slot.info.value_hash,
                        )
                        .await?;
                    }
                    db.save_slot(ctx, slot, spent_block_index).await?;
                }

                for slot in prev_distributor.created_slot_values_to_slots(
                    pending_items.pending_entry_slot_values,
                    RewardDistributorSlotNonce::ENTRY,
                ) {
                    db.save_dig_indexed_slot_value_by_puzzle_hash(
                        slot.info.value.payout_puzzle_hash,
                        RewardDistributorSlotNonce::ENTRY.to_u64(),
                        slot.info.value_hash,
                    )
                    .await?;
                    db.save_slot(ctx, slot, 0).await?;
                }

                for slot in prev_distributor.created_slot_values_to_slots(
                    pending_items.pending_reward_slot_values,
                    RewardDistributorSlotNonce::REWARD,
                ) {
                    db.save_dig_indexed_slot_value_by_epoch_start(
                        slot.info.value.epoch_start,
                        RewardDistributorSlotNonce::REWARD.to_u64(),
                        slot.info.value_hash,
                    )
                    .await?;
                    db.save_slot(ctx, slot, 0).await?;
                }
            }
        }

        if let Some(some_distributor) = RewardDistributor::from_parent_spend(
            ctx,
            coin_record.coin,
            parent_puzzle,
            solution_ptr,
            constants,
        )? {
            last_coin_id = some_distributor.coin.coin_id();
            distributor = Some(some_distributor);
            skip_db_save = false;
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

pub async fn find_reserve(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    launcher_id: Bytes32,
    asset_id: Bytes32,
    nonce: u64,
    amount: u64,
    include_spent: bool,
) -> Result<Reserve, CliError> {
    let controller_singleton_struct_hash = SingletonStruct::new(launcher_id).tree_hash().into();
    let inner_puzzle_hash =
        P2DelegatedBySingletonLayerArgs::curry_tree_hash(controller_singleton_struct_hash, nonce);
    let puzzle_hash: Bytes32 = CatArgs::curry_tree_hash(asset_id, inner_puzzle_hash).into();

    let Some(coin_records) = client
        .get_coin_records_by_puzzle_hash(puzzle_hash, None, None, Some(include_spent))
        .await?
        .coin_records
    else {
        return Err(CliError::CoinNotFound(puzzle_hash));
    };

    let Some(reserve_coin_record) = coin_records.iter().find(|coin_record| {
        coin_record.coin.amount == amount && coin_record.coin.puzzle_hash == puzzle_hash
    }) else {
        return Err(CliError::CoinNotFound(puzzle_hash));
    };

    let Some(parent_spend) = client
        .get_puzzle_and_solution(
            reserve_coin_record.coin.parent_coin_info,
            Some(reserve_coin_record.confirmed_block_index),
        )
        .await?
        .coin_solution
    else {
        return Err(CliError::CoinNotSpent(
            reserve_coin_record.coin.parent_coin_info,
        ));
    };

    let parent_puzzle_ptr = ctx.alloc(&parent_spend.puzzle_reveal)?;
    let parent_puzzle = Puzzle::parse(ctx, parent_puzzle_ptr);
    let Some(parent_cat) = CatLayer::<NodePtr>::parse_puzzle(ctx, parent_puzzle)? else {
        return Err(CliError::Custom("Parent is not a CAT".to_string()));
    };

    let proof = LineageProof {
        parent_parent_coin_info: parent_spend.coin.parent_coin_info,
        parent_inner_puzzle_hash: ctx.tree_hash(parent_cat.inner_puzzle).into(),
        parent_amount: parent_spend.coin.amount,
    };

    Ok(Reserve {
        coin: reserve_coin_record.coin,
        asset_id,
        proof,
        inner_puzzle_hash: inner_puzzle_hash.into(),
        controller_singleton_struct_hash,
        nonce,
    })
}

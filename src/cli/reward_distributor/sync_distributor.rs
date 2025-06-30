use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, CoinSpend},
    puzzles::{cat::CatArgs, singleton::SingletonStruct, LineageProof},
};
use chia_puzzle_types::cat::CatSolution;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        Cat, CatInfo, CatLayer, CatSpend, DriverError, HashedPtr, Layer, Puzzle, SingletonLayer,
        Spend, SpendContext,
    },
};
use clvmr::NodePtr;

use crate::{
    CliError, Db, P2DelegatedBySingletonLayerArgs, Reserve, RewardDistributor,
    RewardDistributorCommitmentSlotValue, RewardDistributorConstants,
    RewardDistributorEntrySlotValue, RewardDistributorRewardSlotValue, RewardDistributorSlotNonce,
    Slot, SlotInfo, SlotProof,
};

pub async fn sync_distributor(
    client: &CoinsetClient,
    db: &Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<RewardDistributor, CliError> {
    let constants = if let Some(cached_constants) = db
        .get_reward_distributor_configuration(ctx, launcher_id)
        .await?
    {
        cached_constants
    } else {
        // configuration not in database, so we need to fetch the launcher
        let launcher_coin_record = client
            .get_coin_record_by_name(launcher_id)
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(launcher_id))?;
        let launcher_coin_spend = client
            .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(launcher_id))?;

        let launcher_solution_ptr = ctx.alloc(&launcher_coin_spend.solution)?;
        let Some((constants, _initial_state, _distributor_eve_coin)) =
            RewardDistributor::from_launcher_solution(
                ctx,
                launcher_coin_spend.coin,
                launcher_solution_ptr,
            )?
        else {
            return Err(CliError::Custom(
                "Could not parse launcher spend".to_string(),
            ));
        };

        constants
    };

    let mut records = client
        .get_coin_records_by_hint(constants.launcher_id, None, None, Some(false))
        .await?
        .coin_records
        .ok_or(CliError::Custom(
            "No unspent coin records found".to_string(),
        ))?;

    while !records.is_empty() {
        let coin_record = records.remove(0);
        if coin_record.spent {
            continue;
        }

        let next_spend = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_record.coin.parent_coin_info))?;

        if let Ok(Some(distributor)) =
            RewardDistributor::from_parent_spend(ctx, &next_spend, constants)
        {
            return mempool_distributor_maybe(ctx, distributor, client).await;
        }
    }

    // Could not find distributor, so we're just after the eve spend and need to do special parsing
    let launcher_coin_record = client
        .get_coin_record_by_name(launcher_id)
        .await?
        .coin_record
        .ok_or(CliError::CoinNotFound(launcher_id))?;
    let launcher_coin_spend = client
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?
        .coin_solution
        .ok_or(CliError::CoinNotSpent(launcher_id))?;

    let launcher_solution_ptr = ctx.alloc(&launcher_coin_spend.solution)?;
    let Some((constants, initial_state, distributor_eve_coin)) =
        RewardDistributor::from_launcher_solution(
            ctx,
            launcher_coin_spend.coin,
            launcher_solution_ptr,
        )?
    else {
        return Err(CliError::Custom(
            "Could not parse launcher spend".to_string(),
        ));
    };

    let distributor_eve_coin_spend = client
        .get_puzzle_and_solution(
            distributor_eve_coin.coin_id(),
            Some(launcher_coin_record.spent_block_index),
        )
        .await?
        .coin_solution
        .ok_or(CliError::CoinNotSpent(distributor_eve_coin.coin_id()))?;

    let reserve = find_reserve(
        ctx,
        client,
        launcher_id,
        constants.reserve_asset_id,
        0,
        0,
        false,
    )
    .await?;

    let (new_distributor, _slot) = RewardDistributor::from_eve_coin_spend(
        ctx,
        constants,
        initial_state,
        distributor_eve_coin_spend,
        reserve.coin.parent_coin_info,
        reserve.proof,
    )?
    .ok_or(CliError::Custom(
        "Could not parse eve coin spend".to_string(),
    ))?;

    Ok(new_distributor)
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

pub async fn mempool_distributor_maybe(
    ctx: &mut SpendContext,
    on_chain_distributor: RewardDistributor,
    client: &CoinsetClient,
) -> Result<RewardDistributor, CliError> {
    let Some(mut mempool_items) = client
        .get_mempool_items_by_coin_name(on_chain_distributor.coin.coin_id())
        .await?
        .mempool_items
    else {
        return Ok(on_chain_distributor);
    };

    if mempool_items.is_empty() {
        return Ok(on_chain_distributor);
    }

    let mempool_item = mempool_items.remove(0);
    let mut distributor = on_chain_distributor;
    let mut parent_id_to_look_for = distributor.coin.parent_coin_info;
    loop {
        let Some(distributor_spend) = mempool_item
            .spend_bundle
            .coin_spends
            .iter()
            .find(|c| c.coin.parent_coin_info == parent_id_to_look_for)
        else {
            break;
        };

        let Some(new_distributor) = RewardDistributor::from_spend(
            ctx,
            distributor_spend,
            Some(distributor.reserve.child(1).proof),
            distributor.info.constants,
        )?
        else {
            break;
        };
        distributor = new_distributor;
        parent_id_to_look_for = distributor.coin.coin_id();
    }

    let reserve_spend = mempool_item
        .spend_bundle
        .coin_spends
        .iter()
        .find(|coin_spend| coin_spend.coin == distributor.reserve.coin)
        .ok_or(DriverError::Custom("Reserve spend not found".to_string()))?
        .clone();
    let spends_to_add: Vec<CoinSpend> = mempool_item
        .spend_bundle
        .coin_spends
        .into_iter()
        .filter(|coin_spend| {
            coin_spend.coin != distributor.coin && coin_spend.coin != distributor.reserve.coin
        })
        .collect();

    // CATs spent with reserve are stored in the pending spend info item
    // since they need to be spent to form a ring (in finish_spend)
    let mut other_cats = Vec::new();
    #[allow(unused_assignments)]
    let (mut cat_spend, mut cat_solution) = spend_to_cat_spend(ctx, reserve_spend)?;
    loop {
        let cat_to_find = cat_solution.prev_coin_id;
        if cat_to_find == distributor.reserve.coin.coin_id() {
            break;
        }

        let cat_coin_spend = spends_to_add
            .iter()
            .find(|coin_spend| coin_spend.coin.coin_id() == cat_to_find)
            .ok_or(DriverError::Custom("CAT spend not found".to_string()))?
            .clone();
        (cat_spend, cat_solution) = spend_to_cat_spend(ctx, cat_coin_spend)?;
        other_cats.push(cat_spend);
    }

    // finally, set things up for RBF
    spends_to_add.into_iter().for_each(|coin_spend| {
        if other_cats
            .iter()
            .all(|cat_spend| cat_spend.cat.coin != coin_spend.coin)
        {
            ctx.insert(coin_spend);
        }
    });
    distributor.set_pending_signature(mempool_item.spend_bundle.aggregated_signature);
    distributor.set_pending_other_cats(other_cats);

    Ok(distributor)
}

pub fn spend_to_cat_spend(
    ctx: &mut SpendContext,
    spend: CoinSpend,
) -> Result<(CatSpend, CatSolution<NodePtr>), DriverError> {
    let puzzle_ptr = ctx.alloc(&spend.puzzle_reveal)?;
    let solution_ptr = ctx.alloc(&spend.solution)?;

    let puzzle = Puzzle::parse(ctx, puzzle_ptr);
    let cat = CatLayer::<HashedPtr>::parse_puzzle(ctx, puzzle)?
        .ok_or(DriverError::Custom("Not a CAT".to_string()))?;

    let solution = ctx.extract::<CatSolution<NodePtr>>(solution_ptr)?;

    Ok((
        CatSpend {
            cat: Cat::new(
                spend.coin,
                solution.lineage_proof,
                CatInfo::new(cat.asset_id, None, cat.inner_puzzle.tree_hash().into()),
            ),
            spend: Spend::new(cat.inner_puzzle.ptr(), solution.inner_puzzle_solution),
            hidden: false,
        },
        solution,
    ))
}

pub async fn find_reward_slot(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    constants: RewardDistributorConstants,
    epoch_start: u64,
) -> Result<Slot<RewardDistributorRewardSlotValue>, CliError> {
    let mut epoch_start = epoch_start;

    loop {
        let mut possible_records = client
            .get_coin_records_by_hint(epoch_start.tree_hash().into(), None, None, Some(false))
            .await?
            .coin_records
            .ok_or(DriverError::MissingHint)?;

        while !possible_records.is_empty() {
            let coin_record = possible_records.remove(0);
            let distributor_spent = client
                .get_puzzle_and_solution(
                    coin_record.coin.parent_coin_info,
                    Some(coin_record.confirmed_block_index),
                )
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(coin_record.coin.parent_coin_info))?;

            let Some(distributor) =
                RewardDistributor::from_spend(ctx, &distributor_spent, None, constants)?
            else {
                // eve spend
                let slot_value = RewardDistributorRewardSlotValue {
                    epoch_start,
                    next_epoch_initialized: false,
                    rewards: 0,
                };
                let slot_info = SlotInfo::<RewardDistributorRewardSlotValue>::from_value(
                    constants.launcher_id,
                    RewardDistributorSlotNonce::REWARD.to_u64(),
                    slot_value,
                );

                let puzzle_ptr = ctx.alloc(&distributor_spent.puzzle_reveal)?;
                let puzzle = Puzzle::parse(ctx, puzzle_ptr);
                let puzzle = SingletonLayer::<HashedPtr>::parse_puzzle(ctx, puzzle)?.unwrap();
                let slot = Slot::<RewardDistributorRewardSlotValue>::new(
                    SlotProof {
                        parent_parent_info: distributor_spent.coin.parent_coin_info,
                        parent_inner_puzzle_hash: puzzle.inner_puzzle.tree_hash().into(),
                    },
                    slot_info,
                );
                return Ok(slot);
            };

            if let Some(slot) = distributor
                .pending_spend
                .created_reward_slots
                .iter()
                .find_map(|slot| {
                    if slot.epoch_start == epoch_start {
                        let slot = distributor
                            .created_slot_value_to_slot(*slot, RewardDistributorSlotNonce::REWARD);
                        if slot.coin == coin_record.coin {
                            Some(slot)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            {
                return Ok(slot);
            };
        }
        epoch_start -= constants.epoch_seconds;
    }
}

pub async fn find_commitment_slots(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    constants: RewardDistributorConstants,
    clawback_ph: Bytes32,
    epoch_start: Option<u64>,
    rewards: Option<u64>,
) -> Result<Vec<Slot<RewardDistributorCommitmentSlotValue>>, CliError> {
    let mut possible_records = client
        .get_coin_records_by_hint(clawback_ph, None, None, Some(false))
        .await?
        .coin_records
        .ok_or(DriverError::MissingHint)?;

    let mut slots = Vec::new();

    while !possible_records.is_empty() {
        let coin_record = possible_records.remove(0);
        let distributor_spent = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_record.coin.parent_coin_info))?;

        let Some(distributor) =
            RewardDistributor::from_spend(ctx, &distributor_spent, None, constants)?
        else {
            continue;
        };

        if let Some(slot) = distributor
            .pending_spend
            .created_commitment_slots
            .iter()
            .find_map(|slot| {
                if slot.clawback_ph == clawback_ph {
                    if let Some(epoch_start) = epoch_start {
                        if slot.epoch_start != epoch_start {
                            return None;
                        }
                    }
                    if let Some(rewards) = rewards {
                        if slot.rewards != rewards {
                            return None;
                        }
                    }
                    let slot = distributor
                        .created_slot_value_to_slot(*slot, RewardDistributorSlotNonce::COMMITMENT);
                    if slot.coin == coin_record.coin {
                        Some(slot)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        {
            slots.push(slot);
        };
    }

    Ok(slots)
}

pub async fn find_entry_slots(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    constants: RewardDistributorConstants,
    payout_puzzle_hash: Bytes32,
    initial_cumulative_payout: Option<u64>,
    shares: Option<u64>,
) -> Result<Vec<Slot<RewardDistributorEntrySlotValue>>, CliError> {
    let mut possible_records = client
        .get_coin_records_by_hint(payout_puzzle_hash, None, None, Some(false))
        .await?
        .coin_records
        .ok_or(DriverError::MissingHint)?;

    let mut slots = Vec::new();

    while !possible_records.is_empty() {
        let coin_record = possible_records.remove(0);
        let distributor_spent = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_record.coin.parent_coin_info))?;

        let Some(distributor) =
            RewardDistributor::from_spend(ctx, &distributor_spent, None, constants)?
        else {
            continue;
        };

        if let Some(slot) = distributor
            .pending_spend
            .created_entry_slots
            .iter()
            .find_map(|slot| {
                if let Some(initial_cumulative_payout) = initial_cumulative_payout {
                    if initial_cumulative_payout != slot.initial_cumulative_payout {
                        return None;
                    }
                }
                if let Some(shares) = shares {
                    if shares != slot.shares {
                        return None;
                    }
                }
                if slot.payout_puzzle_hash == payout_puzzle_hash {
                    let slot = distributor
                        .created_slot_value_to_slot(*slot, RewardDistributorSlotNonce::ENTRY);
                    if slot.coin == coin_record.coin {
                        Some(slot)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        {
            slots.push(slot);
        };
    }

    Ok(slots)
}

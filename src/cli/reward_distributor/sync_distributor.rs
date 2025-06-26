use chia::{
    clvm_utils::ToTreeHash,
    protocol::Bytes32,
    puzzles::{cat::CatArgs, singleton::SingletonStruct, LineageProof},
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{CatLayer, DriverError, Layer, Puzzle, SpendContext},
};
use clvmr::NodePtr;

use crate::{
    CliError, Db, P2DelegatedBySingletonLayerArgs, Reserve, RewardDistributor,
    RewardDistributorCommitmentSlotValue, RewardDistributorConstants,
    RewardDistributorEntrySlotValue, RewardDistributorRewardSlotValue, RewardDistributorSlotNonce,
    Slot,
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
    loop {
        let Some(distributor_spend) = mempool_item
            .spend_bundle
            .coin_spends
            .iter()
            .find(|c| c.coin.coin_id() == distributor.coin.coin_id())
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
    }

    Ok(distributor)
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
                continue;
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

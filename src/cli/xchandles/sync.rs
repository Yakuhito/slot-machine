use std::collections::HashSet;

use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinRecord, CoinsetClient},
    driver::{Puzzle, SpendContext},
};

use crate::{CliError, Db, XchandlesConstants, XchandlesRegistry};

pub async fn sync_xchandles(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<XchandlesRegistry, CliError> {
    let last_unspent_coin_info = db.get_last_unspent_singleton_coin(launcher_id).await?;

    let last_spent_coin_id: Bytes32 =
        if let Some((_coin_id, parent_coin_id)) = last_unspent_coin_info {
            parent_coin_id
        } else {
            launcher_id
        };

    let mut last_coin_id = last_spent_coin_id;
    let mut registry: Option<XchandlesRegistry> = None;
    let mut constants: Option<XchandlesConstants> = None;
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
            db.save_singleton_coin(launcher_id, coin_record).await?;
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

        if coin_record.coin.coin_id() == launcher_id {
            let Some((
                new_registry,
                initial_slots,
                _initial_registration_asset_id,
                _initial_base_price,
            )) = XchandlesRegistry::from_launcher_solution(ctx, coin_record.coin, solution_ptr)?
            else {
                return Err(CliError::CoinNotFound(last_coin_id));
            };

            db.save_slot(ctx, initial_slots[0].clone(), 0).await?;
            db.save_xchandles_indexed_slot_value(
                initial_slots[0].info.launcher_id,
                initial_slots[0].info.value.handle_hash,
                initial_slots[0].info.value_hash,
            )
            .await?;

            db.save_slot(ctx, initial_slots[1].clone(), 0).await?;
            db.save_xchandles_indexed_slot_value(
                initial_slots[1].info.launcher_id,
                initial_slots[1].info.value.handle_hash,
                initial_slots[1].info.value_hash,
            )
            .await?;

            db.save_singleton_coin(
                launcher_id,
                CoinRecord {
                    coin: new_registry.coin,
                    coinbase: false,
                    confirmed_block_index: coin_record.spent_block_index,
                    spent: false,
                    spent_block_index: 0,
                    timestamp: 0,
                },
            )
            .await?;

            last_coin_id = new_registry.coin.coin_id();
            constants = Some(new_registry.info.constants);
            registry = Some(new_registry);
            continue;
        } else if coin_record.coin.parent_coin_info == launcher_id {
            last_coin_id = launcher_id;
            continue;
        }

        let constants = if let Some(cts) = constants {
            cts
        } else {
            // look for constants from launcher spend
            let launcher_record = client
                .get_coin_record_by_name(launcher_id)
                .await?
                .coin_record
                .ok_or(CliError::CoinNotFound(launcher_id))?;

            let launcher_spend = client
                .get_puzzle_and_solution(launcher_id, Some(launcher_record.spent_block_index))
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(launcher_id))?;

            let solution_ptr = ctx.alloc(&launcher_spend.solution)?;

            if let Some((
                new_registry,
                _initial_slots,
                _initial_registration_asset_id,
                _initial_base_price,
            )) =
                XchandlesRegistry::from_launcher_solution(ctx, launcher_record.coin, solution_ptr)?
            {
                constants = Some(new_registry.info.constants);
                new_registry.info.constants
            } else {
                return Err(CliError::ConstantsNotSet);
            }
        };

        let registry = XchandlesRegistry::from_spend(ctx, &coin_spend, constants)?;
        if !skip_db_save {
            if let Some(ref prev_registry) = registry {
                for value in prev_registry.pending_spend.spent_slots {
                    db.mark_slot_as_spent(
                        launcher_id,
                        0,
                        value.tree_hash().into(),
                        coin_record.spent_block_index,
                    )
                    .await?;

                    // no need to actually delete handle indexed value, as
                    //   all actions will overwrite (not remove) the handle
                    //   from the list
                }

                let mut processed_values = HashSet::<Bytes32>::new();
                for slot_value in prev_registry.pending_spend.created_slots {
                    let slot_value_hash: Bytes32 = slot_value.tree_hash().into();
                    if processed_values.contains(&slot_value_hash) {
                        continue;
                    }
                    processed_values.insert(slot_value_hash);

                    // same slot can be created and spent mutliple times in the same block
                    let no_spent = prev_registry
                        .pending_spend
                        .spent_slots
                        .iter()
                        .filter(|sv| sv.tree_hash().into() == slot_value_hash)
                        .count();
                    let no_created = prev_registry
                        .pending_spend
                        .created_slots
                        .iter()
                        .filter(|sv| sv.tree_hash().into() == slot_value_hash)
                        .count();
                    if no_spent != no_created {
                        continue;
                    }

                    db.save_xchandles_indexed_slot_value(
                        prev_registry.info.constants.launcher_id,
                        slot_value.handle_hash,
                        slot_value_hash,
                    )
                    .await?;
                    db.save_slot(ctx, prev_registry.created_slot_value_to_slot(slot_value), 0)
                        .await?;
                }
            }
        }

        if let Some(some_registry) = XchandlesRegistry::from_parent_spend(
            ctx,
            coin_record.coin,
            parent_puzzle,
            solution_ptr,
            constants,
        )? {
            last_coin_id = some_registry.coin.coin_id();
            registry = Some(some_registry);
        } else {
            break;
        };
    }

    if let Some(registry) = registry {
        Ok(registry)
    } else {
        Err(CliError::CoinNotFound(last_coin_id))
    }
}

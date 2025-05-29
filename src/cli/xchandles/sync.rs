use chia::protocol::Bytes32;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinRecord, CoinsetClient},
    driver::{Puzzle, SpendContext},
};

use crate::{CliError, Db, XchandlesConstants, XchandlesRegistry};

pub async fn sync_xchandles(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    constants: XchandlesConstants,
) -> Result<XchandlesRegistry, CliError> {
    let last_unspent_coin_info = db
        .get_last_unspent_singleton_coin(constants.launcher_id)
        .await?;

    let last_spent_coin_id: Bytes32 =
        if let Some((_coin_id, parent_coin_id)) = last_unspent_coin_info {
            parent_coin_id
        } else {
            constants.launcher_id
        };

    let mut last_coin_id = last_spent_coin_id;
    let mut registry: Option<XchandlesRegistry> = None;
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
        let parent_puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = ctx.alloc(&coin_spend.solution)?;
        if !skip_db_save {
            if let Some(ref prev_registry) = registry {
                let new_slots = prev_registry.get_new_slots_from_spend(ctx, solution_ptr)?;

                for slot in new_slots.iter() {
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
                }

                for slot in new_slots {
                    db.save_slot(ctx, slot, 0).await?;
                    db.save_catalog_indexed_slot_value(
                        slot.info.value.asset_id,
                        slot.info.value_hash,
                    )
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
        } else if coin_record.coin.coin_id() == constants.launcher_id {
            let Some((
                new_registry,
                initial_slots,
                initial_registration_asset_id,
                initial_base_price,
            )) = XchandlesRegistry::from_launcher_solution(ctx, coin_record.coin, solution_ptr)?
            else {
                return Err(CliError::CoinNotFound(last_coin_id));
            };

            db.save_slot(ctx, initial_slots[0], 0).await?;
            db.save_xchandles_indexed_slot_value(
                initial_slots[0].info.launcher_id,
                initial_slots[0].info.value.handle_hash,
                initial_slots[0].info.value_hash,
            )
            .await?;

            db.save_slot(ctx, initial_slots[1], 0).await?;
            db.save_xchandles_indexed_slot_value(
                initial_slots[1].info.launcher_id,
                initial_slots[1].info.value.handle_hash,
                initial_slots[1].info.value_hash,
            )
            .await?;

            db.save_singleton_coin(
                constants.launcher_id,
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
            registry = Some(new_registry);
        } else if coin_record.coin.parent_coin_info == constants.launcher_id {
            last_coin_id = constants.launcher_id;
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

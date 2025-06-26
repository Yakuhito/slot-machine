use std::collections::HashSet;

use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::SpendContext,
};

use crate::{CliError, Db, XchandlesRegistry};

pub async fn sync_xchandles(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<XchandlesRegistry, CliError> {
    let (mut registry, mut skip_save): (XchandlesRegistry, bool) =
        if let (Some((_coin_id, parent_coin_id)), Some(constants)) = (
            db.get_last_unspent_singleton_coin(launcher_id).await?,
            db.get_xchandles_configuration(ctx, launcher_id).await?,
        ) {
            let parent_record = client
                .get_coin_record_by_name(parent_coin_id)
                .await?
                .coin_record
                .ok_or(CliError::CoinNotFound(parent_coin_id))?;

            let parent_spend = client
                .get_puzzle_and_solution(parent_coin_id, Some(parent_record.spent_block_index))
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(parent_coin_id))?;

            (
                XchandlesRegistry::from_parent_spend(ctx, &parent_spend, constants)?.ok_or(
                    CliError::Custom("Could not parse latest spent CATalog registry".to_string()),
                )?,
                false,
            )
        } else {
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

            let (registry, initial_slots, _initial_registration_asset_id, _initial_base_price) =
                XchandlesRegistry::from_launcher_solution(ctx, launcher_record.coin, solution_ptr)?
                    .ok_or(CliError::CoinNotFound(launcher_id))?;

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

            // do NOT save eve coin in db
            // db.save_singleton_coin(
            //     launcher_id,
            //     CoinRecord {
            //         coin: launcher_record.coin,
            //         coinbase: false,
            //         confirmed_block_index: launcher_record.spent_block_index,
            //         spent: false,
            //         spent_block_index: 0,
            //         timestamp: 0,
            //     },
            // )
            // .await?;

            (registry, true)
        };

    loop {
        let coin_record = client
            .get_coin_record_by_name(registry.coin.coin_id())
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(registry.coin.coin_id()))?;

        if skip_save {
            skip_save = false;
        } else {
            db.save_singleton_coin(registry.info.constants.launcher_id, coin_record)
                .await?;
        }

        if !coin_record.spent {
            break;
        }

        let coin_spend = client
            .get_puzzle_and_solution(
                coin_record.coin.coin_id(),
                Some(coin_record.spent_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_record.coin.coin_id()))?;

        registry = XchandlesRegistry::from_spend(ctx, &coin_spend, registry.info.constants)?
            .ok_or(CliError::Custom(
                "Could not parse new XCHandles registry spend".to_string(),
            ))?;

        for value in registry.pending_spend.spent_slots.iter() {
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
        for slot_value in registry.pending_spend.created_slots.iter() {
            let slot_value_hash: Bytes32 = slot_value.tree_hash().into();
            if processed_values.contains(&slot_value_hash) {
                continue;
            }
            processed_values.insert(slot_value_hash);

            // same slot can be created and spent mutliple times in the same block
            let no_spent = registry
                .pending_spend
                .spent_slots
                .iter()
                .filter(|sv| sv.tree_hash() == slot_value_hash.into())
                .count();
            let no_created = registry
                .pending_spend
                .created_slots
                .iter()
                .filter(|sv| sv.tree_hash() == slot_value_hash.into())
                .count();
            if no_spent >= no_created {
                continue;
            }

            db.save_xchandles_indexed_slot_value(
                registry.info.constants.launcher_id,
                slot_value.handle_hash,
                slot_value_hash,
            )
            .await?;
            db.save_slot(
                ctx,
                registry.created_slot_value_to_slot(slot_value.clone()),
                0,
            )
            .await?;
        }

        registry = registry.child(registry.pending_spend.latest_state.1);
    }

    mempool_registry_maybe(ctx, registry, client).await
}

pub async fn mempool_registry_maybe(
    ctx: &mut SpendContext,
    on_chain_registry: XchandlesRegistry,
    client: &CoinsetClient,
) -> Result<XchandlesRegistry, CliError> {
    let Some(mut mempool_items) = client
        .get_mempool_items_by_coin_name(on_chain_registry.coin.coin_id())
        .await?
        .mempool_items
    else {
        return Ok(on_chain_registry);
    };

    if mempool_items.is_empty() {
        return Ok(on_chain_registry);
    }

    let mempool_item = mempool_items.remove(0);
    let mut registry = on_chain_registry;
    let mut parent_id_to_look_for = registry.coin.parent_coin_info;
    loop {
        let Some(registry_spend) = mempool_item
            .spend_bundle
            .coin_spends
            .iter()
            .find(|c| c.coin.parent_coin_info == parent_id_to_look_for)
        else {
            break;
        };

        let Some(new_registry) =
            XchandlesRegistry::from_spend(ctx, registry_spend, registry.info.constants)?
        else {
            break;
        };
        registry = new_registry;
        parent_id_to_look_for = registry.coin.coin_id();
    }

    Ok(registry)
}

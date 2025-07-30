use std::collections::HashSet;

use chia::{
    clvm_utils::{tree_hash, ToTreeHash},
    protocol::{Bytes32, Coin},
    puzzles::{singleton::LauncherSolution, LineageProof, Proof},
};
use chia_puzzle_types::Memos;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        CatalogRegistry, CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState,
        DriverError, Layer, Puzzle, SingletonLayer, Slot, SpendContext,
    },
    types::{
        puzzles::{CatalogSlotValue, SlotInfo},
        Condition, Conditions,
    },
};
use clvmr::NodePtr;

use crate::{CliError, Db};

pub async fn sync_catalog(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    constants: CatalogRegistryConstants,
) -> Result<CatalogRegistry, CliError> {
    let (mut catalog, mut skip_save): (CatalogRegistry, bool) =
        if let Some((_coin_id, parent_coin_id)) = db
            .get_last_unspent_singleton_coin(constants.launcher_id)
            .await?
        {
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
                CatalogRegistry::from_parent_spend(ctx, &parent_spend, constants)?.ok_or(
                    CliError::Custom("Could not parse latest spent CATalog registry".to_string()),
                )?,
                false,
            )
        } else {
            // No result -> sync from launcher
            let launcher_record = client
                .get_coin_record_by_name(constants.launcher_id)
                .await?
                .coin_record
                .ok_or(CliError::CoinNotFound(constants.launcher_id))?;

            let launcher_spend = client
                .get_puzzle_and_solution(
                    constants.launcher_id,
                    Some(launcher_record.spent_block_index),
                )
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(constants.launcher_id))?;

            let solution_ptr = ctx.alloc(&launcher_spend.solution)?;
            let solution = ctx.extract::<LauncherSolution<NodePtr>>(solution_ptr)?;
            let catalog_eve_coin =
                Coin::new(constants.launcher_id, solution.singleton_puzzle_hash, 1);
            let catalog_eve_coin_id = catalog_eve_coin.coin_id();

            let eve_coin_spend = client
                .get_puzzle_and_solution(
                    catalog_eve_coin_id,
                    Some(launcher_record.confirmed_block_index),
                )
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(catalog_eve_coin_id))?;

            let eve_coin_puzzle_ptr = ctx.alloc(&eve_coin_spend.puzzle_reveal)?;
            let eve_coin_puzzle = Puzzle::parse(ctx, eve_coin_puzzle_ptr);
            let eve_coin_puzzle = SingletonLayer::<NodePtr>::parse_puzzle(ctx, eve_coin_puzzle)?
                .ok_or(DriverError::Custom(
                    "Could not parse eve CATalog coin puzzle".to_string(),
                ))?;

            let eve_coin_inner_puzzle_hash = tree_hash(ctx, eve_coin_puzzle.inner_puzzle);

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
                return Err(CliError::Custom(
                    "Could not find odd create coin in CATalog eve coin".to_string(),
                ));
            };

            let Memos::Some(memos) = odd_create_coin.memos else {
                return Err(CliError::Driver(DriverError::MissingHint));
            };
            let (decoded_launcher_id, (_decoded_asset_id, (initial_state, ()))) =
                ctx.extract::<(Bytes32, (Bytes32, (CatalogRegistryState, ())))>(memos)?;
            if decoded_launcher_id != constants.launcher_id {
                return Err(CliError::Custom("CATalog launcher ID mismatch".to_string()));
            }

            let new_coin = Coin::new(
                catalog_eve_coin_id,
                odd_create_coin.puzzle_hash,
                odd_create_coin.amount,
            );
            let lineage_proof = LineageProof {
                parent_parent_coin_info: eve_coin_spend.coin.parent_coin_info,
                parent_inner_puzzle_hash: eve_coin_inner_puzzle_hash.into(),
                parent_amount: eve_coin_spend.coin.amount,
            };
            let new_catalog = CatalogRegistry::new(
                new_coin,
                Proof::Lineage(lineage_proof),
                CatalogRegistryInfo::new(initial_state, constants),
            );

            let slot_proof = LineageProof {
                parent_parent_coin_info: lineage_proof.parent_parent_coin_info,
                parent_inner_puzzle_hash: lineage_proof.parent_inner_puzzle_hash,
                parent_amount: lineage_proof.parent_amount,
            };
            let left_slot_value = CatalogSlotValue::initial_left_end();
            let right_slot_value = CatalogSlotValue::initial_right_end();

            db.save_slot(
                ctx,
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
                ctx,
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

            // do NOT save eve coin - we're going through this path until
            // first CATalog is spent
            // db.save_singleton_coin(
            //     constants.launcher_id,
            //     CoinRecord {
            //         coin: new_coin,
            //         coinbase: false,
            //         confirmed_block_index: launcher_record.confirmed_block_index,
            //         spent: false,
            //         spent_block_index: 0,
            //         timestamp: 0,
            //     },
            // )
            // .await?;

            (new_catalog, true)
        };

    loop {
        let coin_record = client
            .get_coin_record_by_name(catalog.coin.coin_id())
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(catalog.coin.coin_id()))?;

        if skip_save {
            skip_save = false;
        } else {
            db.save_singleton_coin(constants.launcher_id, coin_record)
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

        catalog = CatalogRegistry::from_spend(ctx, &coin_spend, constants)?.ok_or(
            CliError::Custom("Could not parse new CATalog registry spend".to_string()),
        )?;

        for slot_value in catalog.pending_spend.spent_slots.iter() {
            let asset_id = slot_value.asset_id;

            if let Some(previous_value_hash) = db.get_catalog_indexed_slot_value(asset_id).await? {
                db.mark_slot_as_spent(
                    constants.launcher_id,
                    0,
                    previous_value_hash,
                    coin_record.spent_block_index,
                )
                .await?;
            }
        }

        let mut processed_values = HashSet::<Bytes32>::new();
        for slot_value in catalog.pending_spend.created_slots.iter() {
            let slot_value_hash: Bytes32 = slot_value.tree_hash().into();
            if processed_values.contains(&slot_value_hash) {
                continue;
            }
            processed_values.insert(slot_value_hash);

            // same slot can be created and spent mutliple times in the same block
            let no_spent = catalog
                .pending_spend
                .spent_slots
                .iter()
                .filter(|sv| sv == &slot_value)
                .count();
            let no_created = catalog
                .pending_spend
                .created_slots
                .iter()
                .filter(|sv| sv == &slot_value)
                .count();
            if no_spent >= no_created {
                continue;
            }

            db.save_catalog_indexed_slot_value(slot_value.asset_id, slot_value_hash)
                .await?;
            db.save_slot(ctx, catalog.created_slot_value_to_slot(*slot_value), 0)
                .await?;
        }

        catalog = catalog.child(catalog.pending_spend.latest_state.1);
    }

    mempool_catalog_maybe(ctx, catalog, client).await
}

pub async fn mempool_catalog_maybe(
    ctx: &mut SpendContext,
    on_chain_catalog: CatalogRegistry,
    client: &CoinsetClient,
) -> Result<CatalogRegistry, CliError> {
    let Some(mut mempool_items) = client
        .get_mempool_items_by_coin_name(on_chain_catalog.coin.coin_id())
        .await?
        .mempool_items
    else {
        return Ok(on_chain_catalog);
    };

    if mempool_items.is_empty() {
        return Ok(on_chain_catalog);
    }

    let mempool_item = mempool_items.remove(0);
    let mut catalog = on_chain_catalog;
    let mut parent_id_to_look_for = catalog.coin.parent_coin_info;
    loop {
        let Some(catalog_spend) = mempool_item
            .spend_bundle
            .coin_spends
            .iter()
            .find(|c| c.coin.parent_coin_info == parent_id_to_look_for)
        else {
            break;
        };

        let Some(new_catalog) =
            CatalogRegistry::from_spend(ctx, catalog_spend, catalog.info.constants)?
        else {
            break;
        };
        catalog = new_catalog;
        parent_id_to_look_for = catalog.coin.coin_id();
    }

    mempool_item
        .spend_bundle
        .coin_spends
        .into_iter()
        .for_each(|coin_spend| {
            if coin_spend.coin != catalog.coin {
                ctx.insert(coin_spend);
            }
        });
    catalog.set_pending_signature(mempool_item.spend_bundle.aggregated_signature);
    Ok(catalog)
}

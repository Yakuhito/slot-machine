use std::time::Instant;

use chia_bls::Signature;
use chia_protocol::Bytes32;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        DriverError, Slot, SpendContext, XchandlesActionLog, XchandlesConstants, XchandlesRegistry,
    },
    types::puzzles::{XchandlesHandleSlotValue, XchandlesSlotNonce, XchandlesUpdateSlotValue},
};
use clvm_utils::ToTreeHash;

use crate::{CliError, Db};

/// One spent registry coin encountered while walking to the unspent tip.
#[derive(Debug, Clone)]
pub struct XchandlesSpentTransition {
    pub height: u32,
    pub logs: Vec<XchandlesActionLog>,
}

/// Result of a full XCHandles singleton walk, including action logs for indexer replay.
#[derive(Debug, Clone)]
pub struct XchandlesSyncResult {
    pub registry: XchandlesRegistry,
    pub spent_transitions: Vec<XchandlesSpentTransition>,
}

fn sync_log(launcher_id: Bytes32, msg: impl AsRef<str>) {
    eprintln!(
        "[xchandles-sync] {} {}",
        hex::encode(launcher_id),
        msg.as_ref()
    );
}

pub async fn sync_xchandles(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<XchandlesRegistry, CliError> {
    Ok(sync_xchandles_detailed(client, db, ctx, launcher_id)
        .await?
        .registry)
}

pub async fn sync_xchandles_detailed(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<XchandlesSyncResult, CliError> {
    let started = Instant::now();
    sync_log(launcher_id, "start");
    let last_unspent = db.get_last_unspent_singleton_coin(launcher_id).await?;
    let saved_constants = db.get_xchandles_configuration(ctx, launcher_id).await?;
    let persist_constants = saved_constants.is_none();
    let (mut registry, mut skip_save): (XchandlesRegistry, bool) =
        if let Some((parent_coin_id, constants)) =
            resume_from_saved_tip(last_unspent, saved_constants)
        {
            sync_log(
                launcher_id,
                format!("resume from parent {}", hex::encode(parent_coin_id)),
            );
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
            sync_log(launcher_id, "cold start from launcher coin");
            let (registry, initial_slots, _initial_registration_asset_id, _initial_base_price) =
                xchandles_registry_from_launcher(client, ctx, launcher_id).await?;

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

    if persist_constants {
        db.save_xchandles_configuration(ctx, registry.info.constants)
            .await?;
    }

    let mut steps = 0u32;
    let mut spent_transitions = Vec::new();
    loop {
        let coin_id = registry.coin.coin_id();
        let coin_record = client
            .get_coin_record_by_name(coin_id)
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(coin_id))?;

        if skip_save {
            skip_save = false;
        } else {
            db.save_singleton_coin(registry.info.constants.launcher_id, coin_record)
                .await?;
        }

        if !coin_record.spent {
            sync_log(
                launcher_id,
                format!(
                    "unspent tip {} after {steps} spent step(s) in {:?}",
                    hex::encode(coin_id),
                    started.elapsed()
                ),
            );
            break;
        }

        let coin_spend = client
            .get_puzzle_and_solution(coin_id, Some(coin_record.spent_block_index))
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_id))?;

        registry = XchandlesRegistry::from_spend(
            ctx,
            &coin_spend,
            registry.info.constants,
            chia_bls::Signature::default(),
        )?
        .ok_or(CliError::Custom(
            "Could not parse new XCHandles registry spend".to_string(),
        ))?;

        for slot_value in registry.pending_spend.created_handle_slots.iter() {
            let slot_value_hash: Bytes32 = slot_value.tree_hash().into();

            db.save_xchandles_indexed_slot_value(
                registry.info.constants.launcher_id,
                slot_value.handle_hash,
                slot_value_hash,
            )
            .await?;
            db.save_slot(
                ctx,
                registry.created_handle_slot_value_to_slot(*slot_value),
                0,
            )
            .await?;
        }

        for value in registry.pending_spend.spent_handle_slots.iter() {
            db.mark_slot_as_spent(
                launcher_id,
                XchandlesSlotNonce::HANDLE.to_u64(),
                value.tree_hash().into(),
                coin_record.spent_block_index,
            )
            .await?;
        }

        let logs = registry.pending_spend.logs.clone();
        let height = coin_record.spent_block_index;
        registry = registry.child(registry.pending_spend.latest_state.1);
        spent_transitions.push(XchandlesSpentTransition { height, logs });
        steps = steps.saturating_add(1);
        if steps.is_multiple_of(10) {
            sync_log(
                launcher_id,
                format!(
                    "still walking history; {steps} spent coins in {:?}",
                    started.elapsed()
                ),
            );
        }
    }

    if let Some(mempool_items) = client
        .get_mempool_items_by_coin_name(registry.coin.coin_id())
        .await?
        .mempool_items
    {
        if !mempool_items.is_empty() {
            if let Some(new_registry) = XchandlesRegistry::from_mempool_item(
                ctx,
                mempool_items[0].spend_bundle.clone(),
                registry.info.constants,
            )? {
                sync_log(
                    launcher_id,
                    format!("done (mempool tip) in {:?}", started.elapsed()),
                );
                return Ok(XchandlesSyncResult {
                    registry: new_registry,
                    spent_transitions,
                });
            }
        }
    }

    sync_log(launcher_id, format!("done in {:?}", started.elapsed()));
    Ok(XchandlesSyncResult {
        registry,
        spent_transitions,
    })
}

/// Resume from the parent of the last unspent registry coin when constants are
/// already in the DB. Missing either piece means a launcher cold start.
fn resume_from_saved_tip(
    last_unspent: Option<(Bytes32, Bytes32)>,
    constants: Option<XchandlesConstants>,
) -> Option<(Bytes32, XchandlesConstants)> {
    match (last_unspent, constants) {
        (Some((_, parent_coin_id)), Some(constants)) => Some((parent_coin_id, constants)),
        _ => None,
    }
}

async fn xchandles_registry_from_launcher(
    client: &CoinsetClient,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<
    (
        XchandlesRegistry,
        [Slot<XchandlesHandleSlotValue>; 2],
        Bytes32,
        u64,
    ),
    CliError,
> {
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

    XchandlesRegistry::from_launcher_solution(ctx, launcher_record.coin, solution_ptr)?
        .ok_or(CliError::CoinNotFound(launcher_id))
}

pub async fn find_xchandles_update_slot(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    constants: XchandlesConstants,
    update_initiator_coin_id: Bytes32,
    handle_hash: Bytes32,
) -> Result<Slot<XchandlesUpdateSlotValue>, CliError> {
    let mut possible_records = client
        .get_coin_records_by_hint(update_initiator_coin_id, None, None, Some(false), None)
        .await?
        .coin_records
        .ok_or(CliError::Driver(DriverError::MissingHint))?;

    while !possible_records.is_empty() {
        let coin_record = possible_records.remove(0);
        let registry_spent = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_record.coin.parent_coin_info))?;

        let Some(registry) =
            XchandlesRegistry::from_spend(ctx, &registry_spent, constants, Signature::default())?
        else {
            continue;
        };

        if let Some(slot) =
            registry
                .pending_spend
                .created_update_slots
                .iter()
                .find_map(|slot_value| {
                    if slot_value.handle_hash != handle_hash
                        || slot_value.update_initiator_coin_id != update_initiator_coin_id
                    {
                        return None;
                    }
                    let slot = registry.created_update_slot_value_to_slot(*slot_value);
                    if slot.coin == coin_record.coin {
                        Some(slot)
                    } else {
                        None
                    }
                })
        {
            return Ok(slot);
        }
    }

    Err(CliError::SlotNotFound("Update"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Bytes32 {
        Bytes32::new([byte; 32])
    }

    fn constants() -> XchandlesConstants {
        XchandlesConstants::new(h(1), h(2), 32, h(3))
    }

    #[test]
    fn resume_uses_saved_unspent_tip_and_constants() {
        let parent = h(0x94);
        let (parent_coin_id, got) =
            resume_from_saved_tip(Some((h(0x79), parent)), Some(constants())).unwrap();
        assert_eq!(parent_coin_id, parent);
        assert_eq!(got, constants());
    }

    #[test]
    fn resume_skips_when_constants_were_never_saved() {
        assert_eq!(resume_from_saved_tip(Some((h(0x79), h(0x94))), None), None);
    }

    #[test]
    fn resume_skips_when_no_unspent_singleton_in_db() {
        assert_eq!(resume_from_saved_tip(None, Some(constants())), None);
    }
}

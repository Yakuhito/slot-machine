use chia::protocol::Bytes32;
use chia::{clvm_utils::ToTreeHash, protocol::Bytes};
use clvmr::{Allocator, NodePtr};

use crate::{
    find_entry_slots, get_constants, get_last_onchain_timestamp, hex_string_to_bytes32,
    multisig_broadcast_thing_finish, multisig_broadcast_thing_start, sync_distributor, CliError,
    Db, MedievalVault, RewardDistributorAddEntryAction, RewardDistributorRemoveEntryAction,
    RewardDistributorSyncAction, StateSchedulerLayerSolution,
};

pub async fn reward_distributor_broadcast_entry_update(
    launcher_id_str: String,
    entry_payout_puzzle_hash_str: String,
    entry_shares: u64,
    signatures_str: String,
    remove_entry: bool,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let entry_payout_puzzle_hash = hex_string_to_bytes32(&entry_payout_puzzle_hash_str)?;

    println!("\nGetting distributor constants... ");
    let db = Db::new(true).await?;

    let mut temp_allocator = Allocator::new();
    let distributor_constants = db
        .get_reward_distributor_configuration(&mut temp_allocator, launcher_id)
        .await?
        .ok_or(CliError::Custom(
            "Could not get reward distributor constants - try running another command to sync it first".to_string(),
        ))?;

    let (signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(
            signatures_str,
            hex::encode(distributor_constants.manager_or_collection_did_launcher_id),
            testnet11,
        )
        .await?;

    println!("\nSyncing reward distributor... ");
    let mut reward_distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let update_time = get_last_onchain_timestamp(&client).await?;
    if reward_distributor.info.state.round_time_info.last_update < update_time - 180 {
        if update_time > reward_distributor.info.state.round_time_info.epoch_end {
            return Err(CliError::Custom(
                "You need to start a new epoch before you can broadcast an entry update"
                    .to_string(),
            ));
        }

        println!("Will also sync reward distributor to {}", update_time);
        let _conds = reward_distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut reward_distributor, update_time)?;
    }

    if remove_entry {
        println!("You'll *REMOVE* the following entry from the reward list:");
    } else {
        println!("You'll *ADD* a new entry with the following parameters:");
    }
    println!(
        "  Entry payout puzzle hash: {}",
        hex::encode(entry_payout_puzzle_hash)
    );
    println!("  Entry shares: {}", entry_shares);

    let message: Bytes32 = (entry_payout_puzzle_hash, entry_shares).tree_hash().into();
    let mut message: Vec<u8> = message.to_vec();
    if remove_entry {
        message.insert(0, b'r');
    } else {
        message.insert(0, b'a');
    }

    let constants = get_constants(testnet11);
    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    let medieval_vault_inner_ph = medieval_vault.info.inner_puzzle_hash();

    let delegated_puzzle_ptr = MedievalVault::delegated_puzzle_for_flexible_send_message::<Bytes>(
        &mut ctx,
        Bytes::from(message),
        launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        constants.genesis_challenge,
    )?;

    let delegated_solution_ptr = ctx.alloc(&StateSchedulerLayerSolution {
        other_singleton_inner_puzzle_hash: reward_distributor.info.inner_puzzle_hash().into(),
        inner_solution: NodePtr::NIL,
    })?;

    medieval_vault.spend_sunsafe(
        &mut ctx,
        &pubkeys,
        delegated_puzzle_ptr,
        delegated_solution_ptr,
    )?;

    if remove_entry {
        println!("Finding entry slot...");
        let entry_slot = find_entry_slots(
            &mut ctx,
            &client,
            reward_distributor.info.constants,
            entry_payout_puzzle_hash,
            None,
            Some(entry_shares),
        )
        .await?
        .into_iter()
        .next()
        .ok_or(CliError::SlotNotFound("Mirror"))?;

        let (_conds, last_payment_amount) = reward_distributor
            .new_action::<RewardDistributorRemoveEntryAction>()
            .spend(
                &mut ctx,
                &mut reward_distributor,
                entry_slot,
                medieval_vault_inner_ph.into(),
            )?;
        println!(
            "Last payment ammount to entry: {} CAT mojos",
            last_payment_amount
        );
    } else {
        let _conds = reward_distributor
            .new_action::<RewardDistributorAddEntryAction>()
            .spend(
                &mut ctx,
                &mut reward_distributor,
                entry_payout_puzzle_hash,
                entry_shares,
                medieval_vault_inner_ph.into(),
            )?;
    }
    let mut _new_distributor = reward_distributor.finish_spend(&mut ctx, vec![])?;

    multisig_broadcast_thing_finish(
        client,
        &mut ctx,
        signature_from_signers,
        fee_str,
        testnet11,
        medieval_vault_coin_id,
        None,
    )
    .await
}

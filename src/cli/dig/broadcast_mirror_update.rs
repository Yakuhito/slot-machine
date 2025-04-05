use chia::protocol::Bytes32;
use chia::puzzles::singleton::SingletonSolution;
use chia::{clvm_utils::ToTreeHash, protocol::Bytes};
use chia_wallet_sdk::driver::{Layer, Spend};
use clvmr::{Allocator, NodePtr};

use crate::{
    find_mirror_slot_for_puzzle_hash, get_constants, get_last_onchain_timestamp,
    hex_string_to_bytes32, multisig_broadcast_thing_finish, multisig_broadcast_thing_start,
    sync_distributor, CliError, Db, DigAddMirrorAction, DigRemoveMirrorAction, DigSyncAction,
    MedievalVault, P2MOfNDelegateDirectArgs, P2MOfNDelegateDirectSolution,
    StateSchedulerLayerSolution,
};

pub async fn dig_broadcast_mirror_update(
    launcher_id_str: String,
    mirror_payout_puzzle_hash_str: String,
    mirror_shares: u64,
    signatures_str: String,
    remove_mirror: bool,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let mirror_payout_puzzle_hash = hex_string_to_bytes32(&mirror_payout_puzzle_hash_str)?;

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
            hex::encode(distributor_constants.validator_launcher_id),
            testnet11,
        )
        .await?;

    println!("\nSyncing reward distributor... ");
    let mut reward_distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let update_time = get_last_onchain_timestamp(&client).await?;
    if reward_distributor.info.state.round_time_info.last_update < update_time - 180 {
        if update_time > reward_distributor.info.state.round_time_info.epoch_end {
            return Err(CliError::Custom(
                "You need to start a new epoch before you can broadcast a mirror update"
                    .to_string(),
            ));
        }

        println!("Will also sync reward distributor to {}", update_time);
        let _conds = reward_distributor.new_action::<DigSyncAction>().spend(
            &mut ctx,
            &mut reward_distributor,
            update_time,
        )?;
    }

    if remove_mirror {
        println!("You'll *REMOVE* the following mirror from the rewarded mirror lists:");
    } else {
        println!("You'll *ADD* a new mirror reward entry with the following parameters:");
    }
    println!(
        "  Mirror payout puzzle hash: {}",
        hex::encode(mirror_payout_puzzle_hash)
    );
    println!("  Mirror shares: {}", mirror_shares);

    let message: Bytes32 = (mirror_payout_puzzle_hash, mirror_shares)
        .tree_hash()
        .into();
    let mut message: Vec<u8> = message.to_vec();
    if remove_mirror {
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

    let medieval_vault_layers = medieval_vault.info.into_layers();
    let medieval_vault_puzzle = medieval_vault_layers.construct_puzzle(&mut ctx)?;
    let medieval_vault_solution = medieval_vault_layers.construct_solution(
        &mut ctx,
        SingletonSolution {
            lineage_proof: medieval_vault.proof,
            amount: medieval_vault.coin.amount,
            inner_solution: P2MOfNDelegateDirectSolution {
                selectors: P2MOfNDelegateDirectArgs::selectors_for_used_pubkeys(
                    &medieval_vault.info.public_key_list,
                    &pubkeys,
                ),
                delegated_puzzle: delegated_puzzle_ptr,
                delegated_solution: delegated_solution_ptr,
            },
        },
    )?;

    ctx.spend(
        medieval_vault.coin,
        Spend::new(medieval_vault_puzzle, medieval_vault_solution),
    )?;

    if remove_mirror {
        println!("Finding mirror slot...");
        let mirror_slot = find_mirror_slot_for_puzzle_hash(
            &mut ctx,
            &db,
            launcher_id,
            mirror_payout_puzzle_hash,
            Some(mirror_shares),
        )
        .await?
        .ok_or(CliError::SlotNotFound("Mirror"))?;

        let (_conds, last_payment_amount) = reward_distributor
            .new_action::<DigRemoveMirrorAction>()
            .spend(
                &mut ctx,
                &mut reward_distributor,
                mirror_slot,
                medieval_vault_inner_ph.into(),
            )?;
        println!(
            "Last payment ammount to mirror: {} CAT mojos",
            last_payment_amount
        );
    } else {
        let (_conds, _new_slot) = reward_distributor
            .new_action::<DigAddMirrorAction>()
            .spend(
                &mut ctx,
                &mut reward_distributor,
                mirror_payout_puzzle_hash,
                mirror_shares,
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
    )
    .await
}

use chia_protocol::Bytes;
use chia_wallet_sdk::{
    driver::{
        DelegatedStateAction, MedievalVault, SingletonInfo, XchandlesRegistryReceivedMessagePrefix,
    },
    types::puzzles::StateSchedulerLayerSolution,
};
use clvm_utils::ToTreeHash;
use clvmr::NodePtr;

use crate::{
    get_constants, multisig_broadcast_thing_finish, multisig_broadcast_thing_start,
    sync_show_changes_and_compute_new_state, CliError,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_broadcast_state_update(
    registry_launcher_id_str: String,
    new_payment_asset_id_str: String,
    new_payment_cat_base_price_str: String,
    new_registration_period: u64,
    payment_asset_id_str: Option<String>,
    payment_cat_base_price_str: Option<String>,
    registration_period: Option<u64>,
    multisig_launcher_id_str: String,
    signatures_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let (signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(signatures_str, multisig_launcher_id_str, testnet11).await?;

    let (new_state, mut registry) = sync_show_changes_and_compute_new_state(
        &mut ctx,
        &client,
        registry_launcher_id_str,
        new_payment_asset_id_str,
        new_payment_cat_base_price_str,
        new_registration_period,
        payment_asset_id_str,
        payment_cat_base_price_str,
        registration_period,
    )
    .await?;

    let constants = get_constants(testnet11);
    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    let medieval_vault_inner_ph = medieval_vault.info.inner_puzzle_hash();

    let delegated_puzzle_ptr = MedievalVault::delegated_puzzle_for_flexible_send_message::<Bytes>(
        &mut ctx,
        XchandlesRegistryReceivedMessagePrefix::update_state(new_state.tree_hash()).into(),
        registry.info.constants.launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        constants.genesis_challenge,
    )?;

    let delegated_solution_ptr = ctx.alloc(&StateSchedulerLayerSolution {
        other_singleton_inner_puzzle_hash: registry.info.inner_puzzle_hash().into(),
        inner_solution: NodePtr::NIL,
    })?;

    medieval_vault.spend_sunsafe(
        &mut ctx,
        &pubkeys,
        delegated_puzzle_ptr,
        delegated_solution_ptr,
    )?;

    let (_conds, inner_spend) = registry.new_action::<DelegatedStateAction>().spend(
        &mut ctx,
        registry.coin,
        new_state,
        medieval_vault_inner_ph.into(),
    )?;
    registry.insert_action_spend(&mut ctx, inner_spend)?;
    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    multisig_broadcast_thing_finish(
        client,
        &mut ctx,
        signature_from_signers + &pending_sig,
        fee_str,
        testnet11,
        medieval_vault_coin_id,
        None,
    )
    .await
}

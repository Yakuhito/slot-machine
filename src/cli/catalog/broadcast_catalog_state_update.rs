use chia::clvm_utils::ToTreeHash;
use chia::protocol::Bytes32;
use clvmr::NodePtr;

use crate::{
    get_constants, hex_string_to_bytes32, multisig_broadcast_thing_finish,
    multisig_broadcast_thing_start, parse_amount, quick_sync_catalog, CatalogRegistryConstants,
    CatalogRegistryState, CliError, DefaultCatMakerArgs, DelegatedStateAction, MedievalVault,
    StateSchedulerLayerSolution,
};

pub async fn catalog_broadcast_state_update(
    new_payment_asset_id_str: String,
    new_payment_asset_amount_str: String,
    launcher_id_str: String,
    signatures_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let new_payment_asset_id = hex_string_to_bytes32(&new_payment_asset_id_str)?;
    let new_payment_asset_amount = parse_amount(&new_payment_asset_amount_str, true)?;

    let (signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(signatures_str, launcher_id_str, testnet11).await?;

    println!("\nSyncing CATalog... ");
    let catalog_constants = CatalogRegistryConstants::get(testnet11);
    let mut catalog = quick_sync_catalog(&client, &mut ctx, catalog_constants).await?;
    println!("Done!");

    println!("Current CATalog state:");
    println!(
        "  CAT Maker: {}",
        hex::encode(catalog.info.state.cat_maker_puzzle_hash.to_bytes())
    );
    println!(
        "  Registration price (mojos): {}",
        catalog.info.state.registration_price
    );
    println!("You'll update the CATalog state to:");

    let new_cat_maker_puzzle_hash: Bytes32 =
        DefaultCatMakerArgs::curry_tree_hash(new_payment_asset_id.tree_hash().into()).into();
    let new_state = CatalogRegistryState {
        cat_maker_puzzle_hash: new_cat_maker_puzzle_hash,
        registration_price: new_payment_asset_amount,
    };
    println!(
        "  CAT Maker: {}",
        hex::encode(new_state.cat_maker_puzzle_hash.to_bytes())
    );
    println!(
        "  Registration price (mojos): {}",
        new_state.registration_price
    );
    println!(
        "  Payment asset id: {}",
        hex::encode(new_state.cat_maker_puzzle_hash.to_bytes())
    );

    let constants = get_constants(testnet11);
    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    let medieval_vault_inner_ph = medieval_vault.info.inner_puzzle_hash();

    let delegated_puzzle_ptr = MedievalVault::delegated_puzzle_for_flexible_send_message::<Bytes32>(
        &mut ctx,
        new_state.tree_hash().into(),
        catalog_constants.launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        constants.genesis_challenge,
    )?;

    let delegated_solution_ptr = ctx.alloc(&StateSchedulerLayerSolution {
        other_singleton_inner_puzzle_hash: catalog.info.inner_puzzle_hash().into(),
        inner_solution: NodePtr::NIL,
    })?;

    medieval_vault.spend_sunsafe(
        &mut ctx,
        &pubkeys,
        delegated_puzzle_ptr,
        delegated_solution_ptr,
    )?;

    let (_conds, inner_spend) = catalog.new_action::<DelegatedStateAction>().spend(
        &mut ctx,
        catalog.coin,
        new_state,
        medieval_vault_inner_ph.into(),
    )?;
    catalog.insert_action_spend(&mut ctx, inner_spend)?;
    let (_new_catalog, pending_sig) = catalog.finish_spend(&mut ctx)?;

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

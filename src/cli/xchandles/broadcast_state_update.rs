use chia_protocol::Bytes;
use chia_wallet_sdk::{
    driver::{
        DelegatedStateAction, MedievalVault, SingletonInfo, XchandlesExpirePricingPuzzle,
        XchandlesRegistryReceivedMessagePrefix, XchandlesRegistryState,
    },
    types::{
        puzzles::{
            DefaultCatMakerArgs, StateSchedulerLayerSolution, XchandlesFactorPricingPuzzleArgs,
        },
        Mod,
    },
};
use clvm_utils::ToTreeHash;
use clvmr::NodePtr;

use crate::{
    get_constants, hex_string_to_bytes32, multisig_broadcast_thing_finish,
    multisig_broadcast_thing_start, parse_amount, quick_sync_xchandles, CliError, Db,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_broadcast_state_update(
    registry_launcher_id_str: String,
    new_payment_asset_id_str: String,
    new_payment_cat_base_price_str: String,
    new_registration_period: u64,
    multisig_launcher_id_str: String,
    signatures_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let registry_launcher_id = hex_string_to_bytes32(&registry_launcher_id_str)?;
    let new_payment_asset_id = hex_string_to_bytes32(&new_payment_asset_id_str)?;
    let new_payment_cat_base_price = parse_amount(&new_payment_cat_base_price_str, true)?;

    let (signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(signatures_str, multisig_launcher_id_str, testnet11).await?;

    println!("\nSyncing XCHandles registry... ");
    let mut db = Db::new(true).await?;
    let mut registry =
        quick_sync_xchandles(&client, &mut db, &mut ctx, registry_launcher_id).await?;
    println!("Done!");

    println!("Current registry state:");
    println!(
        "  CAT Maker: {}",
        hex::encode(registry.info.state.cat_maker_puzzle_hash.to_bytes())
    );
    println!(
        "  Registration pricing puzzle hash: {}",
        hex::encode(registry.info.state.pricing_puzzle_hash.to_bytes())
    );
    println!(
        "  Expired handle pricing puzzle hash: {}",
        hex::encode(
            registry
                .info
                .state
                .expired_handle_pricing_puzzle_hash
                .to_bytes()
        )
    );
    let new_state = XchandlesRegistryState {
        cat_maker_puzzle_hash: DefaultCatMakerArgs::new(new_payment_asset_id.tree_hash().into())
            .curry_tree_hash()
            .into(),
        pricing_puzzle_hash: XchandlesFactorPricingPuzzleArgs {
            base_price: new_payment_cat_base_price,
            registration_period: new_registration_period,
        }
        .curry_tree_hash()
        .into(),
        expired_handle_pricing_puzzle_hash: XchandlesExpirePricingPuzzle::curry_tree_hash(
            new_payment_cat_base_price,
            new_registration_period,
        )
        .into(),
    };
    println!("You'll update the registry state to:");
    println!(
        "  CAT Maker: {}",
        hex::encode(new_state.cat_maker_puzzle_hash.to_bytes())
    );
    println!(
        "  Registration pricing puzzle hash: {}",
        hex::encode(new_state.pricing_puzzle_hash.to_bytes())
    );
    println!(
        "  Expired handle pricing puzzle hash: {}",
        hex::encode(new_state.expired_handle_pricing_puzzle_hash.to_bytes())
    );
    println!(
        "  Payment asset id: {}",
        hex::encode(new_payment_asset_id.to_bytes())
    );

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

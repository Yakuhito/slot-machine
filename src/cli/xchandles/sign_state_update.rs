use chia_protocol::Bytes;
use chia_wallet_sdk::{
    driver::{
        MedievalVault, XchandlesExpirePricingPuzzle, XchandlesRegistryReceivedMessagePrefix,
        XchandlesRegistryState,
    },
    types::{
        puzzles::{DefaultCatMakerArgs, XchandlesFactorPricingPuzzleArgs},
        Mod,
    },
};
use clvm_utils::ToTreeHash;

use crate::{
    get_constants, hex_string_to_bytes32, multisig_sign_thing_finish, multisig_sign_thing_start,
    parse_amount, quick_sync_xchandles, CliError, Db,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_sign_state_update(
    registry_launcher_id_str: String,
    new_payment_asset_id_str: String,
    new_payment_cat_base_price_str: String,
    new_registration_period: u64,
    my_pubkey_str: String,
    multisig_launcher_id_str: String,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let registry_launcher_id = hex_string_to_bytes32(&registry_launcher_id_str)?;
    let new_payment_asset_id = hex_string_to_bytes32(&new_payment_asset_id_str)?;
    let new_payment_cat_base_price = parse_amount(&new_payment_cat_base_price_str, true)?;

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

    let (my_pubkey, mut ctx, client, medieval_vault) =
        multisig_sign_thing_start(my_pubkey_str, multisig_launcher_id_str, testnet11).await?;

    println!("\nSyncing XCHandles registry... ");
    let mut db = Db::new(true).await?;
    let registry = quick_sync_xchandles(&client, &mut db, &mut ctx, registry_launcher_id).await?;
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

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_flexible_send_message::<Bytes>(
        &mut ctx,
        XchandlesRegistryReceivedMessagePrefix::update_state(new_state.tree_hash()).into(),
        registry.info.constants.launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        get_constants(testnet11).genesis_challenge,
    )
    .map_err(CliError::Driver)?;

    multisig_sign_thing_finish(
        &mut ctx,
        delegated_puzzle,
        &medieval_vault,
        my_pubkey,
        testnet11,
        debug,
    )
    .await
}

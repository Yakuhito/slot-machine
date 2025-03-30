use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::SpendContext;
use clvmr::NodePtr;

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, parse_amount, quick_sync_catalog,
    CatalogRegistryConstants, CatalogRegistryState, CliError, DefaultCatMakerArgs, MedievalVault,
};

use super::multisig_sign_thing;

async fn summary_and_delegated_puzzle_for_catalog_state_update(
    ctx: &mut SpendContext,
    medieval_vault: &MedievalVault,
    my_alias: &String,
    testnet11: bool,
    (new_payment_asset_id, new_state): (Bytes32, CatalogRegistryState),
) -> Result<NodePtr, CliError> {
    let client = get_coinset_client(testnet11);

    println!("\nSyncing CATalog... ");
    let catalog_constants = CatalogRegistryConstants::get(testnet11);
    let catalog = quick_sync_catalog(&client, ctx, catalog_constants).await?;
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
        hex::encode(new_payment_asset_id.to_bytes())
    );

    println!("\nYou'll sign this CATALOG STATE UPDATE with the following pubkey:");
    println!("  {}", my_alias);

    MedievalVault::delegated_puzzle_for_catalog_state_update(
        ctx,
        new_state.tree_hash().into(),
        catalog_constants.launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        get_constants(testnet11).genesis_challenge,
    )
    .map_err(CliError::Driver)
}

pub async fn multisig_sign_catalog_state_update(
    new_payment_asset_id_str: String,
    new_payment_asset_amount_str: String,
    my_pubkey_str: String,
    launcher_id_str: String,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let new_payment_asset_id = hex_string_to_bytes32(&new_payment_asset_id_str)?;
    let new_payment_asset_amount = parse_amount(&new_payment_asset_amount_str, true)?;

    let new_state = CatalogRegistryState {
        cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(
            new_payment_asset_id.tree_hash().into(),
        )
        .into(),
        registration_price: new_payment_asset_amount,
    };

    multisig_sign_thing(
        summary_and_delegated_puzzle_for_catalog_state_update,
        (new_payment_asset_id, new_state),
        my_pubkey_str,
        launcher_id_str,
        testnet11,
        debug,
    )
    .await
}

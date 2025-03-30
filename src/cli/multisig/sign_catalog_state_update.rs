use chia::clvm_utils::ToTreeHash;

use crate::{
    get_constants, hex_string_to_bytes32, multisig_sign_thing_finish, multisig_sign_thing_start,
    parse_amount, quick_sync_catalog, CatalogRegistryConstants, CatalogRegistryState, CliError,
    DefaultCatMakerArgs, MedievalVault,
};

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

    let (my_pubkey, mut ctx, client, medieval_vault) =
        multisig_sign_thing_start(my_pubkey_str, launcher_id_str, testnet11).await?;

    println!("\nSyncing CATalog... ");
    let catalog_constants = CatalogRegistryConstants::get(testnet11);
    let catalog = quick_sync_catalog(&client, &mut ctx, catalog_constants).await?;
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

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_catalog_state_update(
        &mut ctx,
        new_state.tree_hash().into(),
        catalog_constants.launcher_id,
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

use chia_wallet_sdk::{driver::Launcher, prelude::Memos};
use clvm_traits::clvm_quote;

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32, multisig_sign_thing_finish,
    multisig_sign_thing_start, CliError, MedievalVault, Verification, VerificationInfo,
    VerificationLauncherKVList, VerifiedData,
};

#[allow(unused_variables)]
pub async fn verifications_sign_launch(
    launcher_id_str: String,
    asset_id_str: String,
    comment: String,
    my_pubkey_str: String,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let asset_id = hex_string_to_bytes32(&asset_id_str)?;

    let (my_pubkey, mut ctx, client, medieval_vault) =
        multisig_sign_thing_start(my_pubkey_str, launcher_id_str, testnet11).await?;

    println!(
        "\nFetching latest data for asset id {}... ",
        hex::encode(asset_id)
    );
    let latest_data = get_latest_data_for_asset_id(&mut ctx, &client, asset_id, testnet11).await?;

    println!("CAT NFT Metadata: ");
    latest_data.pretty_print("  ");
    println!("Note: Attestations cover the following: ticker, name, description, image hash, metadata hash, license hash.");
    println!("It is your responsibility to ensure the hashes are correct.");

    let verified_data = VerifiedData::from_cat_nft_metadata(asset_id, &latest_data, comment);

    let launcher_memos = Memos::new(ctx.alloc(&verified_data.get_hint())?);
    let launcher = Launcher::with_memos(medieval_vault.coin.coin_id(), 0, launcher_memos)
        .with_singleton_amount(1);
    println!(
        "Verification launcher id: {}",
        hex::encode(launcher.coin().coin_id())
    );
    let verification = Verification::after_mint(
        medieval_vault.coin.coin_id(),
        VerificationInfo {
            launcher_id: launcher.coin().coin_id(),
            revocation_singleton_launcher_id: launcher_id,
            verified_data: verified_data.clone(),
        },
    );

    let (launch_conds, _coin) = launcher.spend(
        &mut ctx,
        Verification::inner_puzzle_hash(launcher_id, verified_data.clone()).into(),
        &VerificationLauncherKVList {
            revocation_singleton_launcher_id: launcher_id,
            verified_data,
        },
    )?;

    let genesis_challenge = ctx.alloc(&get_constants(testnet11).genesis_challenge)?;
    let launch_conds_with_recreate = launch_conds.create_coin(
        medieval_vault.info.inner_puzzle_hash().into(),
        medieval_vault.coin.amount,
        Memos::some(ctx.alloc(&medieval_vault.info.launcher_id)?),
    );
    let delegated_puzzle = ctx.alloc(&clvm_quote!(MedievalVault::delegated_conditions(
        launch_conds_with_recreate,
        medieval_vault.coin.coin_id(),
        genesis_challenge,
    )))?;

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

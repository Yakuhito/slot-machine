use chia_wallet_sdk::{driver::Launcher, prelude::Memos};
use clvm_traits::clvm_quote;
use clvmr::NodePtr;

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32,
    multisig_broadcast_thing_finish, multisig_broadcast_thing_start, CliError, MedievalVault,
    Verification, VerificationLauncherKVList, VerifiedData,
};

pub async fn verifications_broadcast_launch(
    launcher_id_str: String,
    asset_id_str: String,
    comment: String,
    signatures_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let asset_id = hex_string_to_bytes32(&asset_id_str)?;

    let (signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(signatures_str, launcher_id_str, testnet11).await?;

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

    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    medieval_vault.spend_sunsafe(&mut ctx, &pubkeys, delegated_puzzle, NodePtr::NIL)?;

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

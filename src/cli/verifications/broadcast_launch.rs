use bech32::Variant;
use chia::{
    protocol::{Bytes, Bytes32, SpendBundle},
    traits::Streamable,
};
use chia_wallet_sdk::{
    driver::{decompress_offer_bytes, DriverError, Launcher, OfferError},
    types::Conditions,
};
use clvm_traits::clvm_quote;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32,
    multisig_broadcast_thing_finish, multisig_broadcast_thing_start, CliError, MedievalVault,
    Verification, VerificationInfo, VerificationLauncherKVList, VerifiedData,
};

pub async fn verifications_broadcast_launch(
    launcher_id_str: String,
    asset_id_str: String,
    comment: String,
    request_offer: Option<String>,
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

    let launcher = Launcher::with_memos(
        medieval_vault.coin.coin_id(),
        0,
        ctx.hint(verified_data.get_hint())?,
    )
    .with_singleton_amount(1);
    let launcher_coin = launcher.coin();
    println!(
        "Verification launcher id: {}",
        hex::encode(launcher_coin.coin_id())
    );

    let (launch_conds, _coin) = launcher.spend(
        &mut ctx,
        Verification::inner_puzzle_hash(launcher_id, verified_data.clone()).into(),
        &VerificationLauncherKVList {
            revocation_singleton_launcher_id: launcher_id,
            verified_data: verified_data.clone(),
        },
    )?;

    let genesis_challenge = ctx.alloc(&get_constants(testnet11).genesis_challenge)?;
    let launch_conds_with_recreate = launch_conds.create_coin(
        medieval_vault.info.inner_puzzle_hash().into(),
        medieval_vault.coin.amount,
        Some(ctx.hint(medieval_vault.info.launcher_id)?),
    );
    let delegated_puzzle = ctx.alloc(&clvm_quote!(MedievalVault::delegated_conditions(
        launch_conds_with_recreate,
        medieval_vault.coin.coin_id(),
        genesis_challenge,
    )))?;

    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    medieval_vault.spend_sunsafe(&mut ctx, &pubkeys, delegated_puzzle, NodePtr::NIL)?;

    let additional_conditions = if let Some(request_offer) = request_offer {
        // spend verification as well to make offer work
        let verification = Verification::after_mint(
            launcher_coin.parent_coin_info,
            launcher_id,
            verified_data.clone(),
        );
        let verification_inner_ph =
            Verification::inner_puzzle_hash(launcher_id, verified_data.clone());
        verification.spend(&mut ctx, None)?;

        let (hrp, data, variant) = bech32::decode(&request_offer)?;
        if variant != Variant::Bech32m || hrp.as_str() != "verificationrequest" {
            return Err(CliError::Custom(
                "Invalid verification request offer provided".to_string(),
            ));
        }
        let bytes = bech32::convert_bits(&data, 5, 8, false)?;
        let decompressed = decompress_offer_bytes(&bytes)?;
        let ptr = node_from_bytes(&mut ctx, &decompressed)?;
        let (asset_id_verif, (spend_bundle_bytes, ())) =
            ctx.extract::<(Bytes32, (Bytes, ()))>(ptr)?;
        if asset_id_verif != asset_id {
            return Err(CliError::Custom(
                "Verification request offer made for another asset id :(".to_string(),
            ));
        }
        let spend_bundle = SpendBundle::from_bytes(&spend_bundle_bytes).unwrap();

        // todo: find payment and send it to recipient
        // todo: find verification asserter
        // todo: spend verification asserter

        Some(Conditions::new()) // todo
    } else {
        None
    };

    multisig_broadcast_thing_finish(
        client,
        &mut ctx,
        signature_from_signers,
        fee_str,
        testnet11,
        medieval_vault_coin_id,
        additional_conditions,
    )
    .await
}

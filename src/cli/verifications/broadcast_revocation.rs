use clvmr::NodePtr;

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32,
    multisig_broadcast_thing_finish, multisig_broadcast_thing_start, sync_verifications, CliError,
    MedievalVault, StateSchedulerLayerSolution, Verification, VerifiedData,
};

pub async fn verifications_broadcast_revocation(
    launcher_id_str: String,
    asset_id_str: String,
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

    let data_hash = VerifiedData::data_hash_from_cat_nft_metadata(&latest_data);

    let verifications =
        sync_verifications(&mut ctx, &client, data_hash, Some(vec![launcher_id]), true).await?;

    if verifications.is_empty() {
        return Err(CliError::Custom(
            "Verification for this asset from this vault not found (or already revoked)"
                .to_string(),
        ));
    }

    let verification = verifications[0].clone();
    verification.spend(
        &mut ctx,
        Some(medieval_vault.info.inner_puzzle_hash().into()),
    )?;

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_flexible_send_message::<()>(
        &mut ctx,
        (),
        verifications[0].info.launcher_id,
        medieval_vault.coin,
        &medieval_vault.info,
        get_constants(testnet11).genesis_challenge,
    )
    .map_err(CliError::Driver)?;

    let delegated_solution_ptr = ctx.alloc(&StateSchedulerLayerSolution {
        other_singleton_inner_puzzle_hash: Verification::inner_puzzle_hash(
            verifications[0].info.revocation_singleton_launcher_id,
            verifications[0].info.verified_data.clone(),
        )
        .into(),
        inner_solution: NodePtr::NIL,
    })?;

    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    medieval_vault.spend_sunsafe(&mut ctx, &pubkeys, delegated_puzzle, delegated_solution_ptr)?;

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

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32, multisig_sign_thing_finish,
    multisig_sign_thing_start, sync_verifications, CliError, MedievalVault, VerifiedData,
};

pub async fn verifications_sign_revocation(
    launcher_id_str: String,
    asset_id_str: String,
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

    let data_hash = VerifiedData::data_hash_from_cat_nft_metadata(&latest_data);

    let verifications =
        sync_verifications(&mut ctx, &client, data_hash, Some(vec![launcher_id]), true).await?;

    if verifications.is_empty() {
        return Err(CliError::Custom(
            "Verification for this asset from this vault not found (or already revoked)"
                .to_string(),
        ));
    }

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_flexible_send_message::<()>(
        &mut ctx,
        (),
        launcher_id,
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

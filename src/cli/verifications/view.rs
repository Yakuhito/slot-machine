use chia::protocol::Bytes32;
use chia_wallet_sdk::driver::SpendContext;

use crate::{
    get_coinset_client, get_latest_data_for_asset_id, hex_string_to_bytes32, sync_verifications,
    CliError, VerifiedData,
};

pub async fn verifications_view(
    asset_id_str: String,
    filter: Option<String>,
    testnet11: bool,
) -> Result<(), CliError> {
    let asset_id = hex_string_to_bytes32(&asset_id_str)?;

    let mut ctx = SpendContext::new();
    let client = get_coinset_client(testnet11);

    println!(
        "\nFetching latest data for asset id {}... ",
        hex::encode(asset_id)
    );
    let latest_data = get_latest_data_for_asset_id(&mut ctx, &client, asset_id, testnet11).await?;

    println!("CAT NFT Metadata: ");
    latest_data.pretty_print("  ");
    println!("Note: Attestations cover the following: ticker, name, description, image hash, metadata hash, license hash.");

    let verified_data_hash = VerifiedData::data_hash_from_cat_nft_metadata(&latest_data);
    println!("Verified data hash: {}", hex::encode(verified_data_hash));

    let filters = if let Some(filter) = filter {
        Some(
            filter
                .split(",")
                .map(hex_string_to_bytes32)
                .collect::<Result<Vec<Bytes32>, _>>()?,
        )
    } else {
        None
    };

    let _verifs = sync_verifications(&mut ctx, &client, verified_data_hash, filters, true).await?;

    Ok(())
}

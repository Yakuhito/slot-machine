use crate::{CatNftMetadata, CliError};

#[allow(clippy::too_many_arguments)]
pub async fn catalog_register(
    tail_reveal_str: String,
    ticker: String,
    name: String,
    description: String,
    precision: u8,
    image_uris_str: String,
    image_hash_str: String,
    metadata_uris_str: String,
    metadata_hash_str: Option<String>,
    license_uris_str: String,
    license_hash_str: Option<String>,
    recipient_address: Option<String>,
    testnet11: bool,
    payment_asset_id_str: String,
    fee_str: String,
) -> Result<(), CliError> {
    println!("Welcome to the CATalog registration process, issuer!");

    Ok(())
}

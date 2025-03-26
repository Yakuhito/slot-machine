use chia::protocol::Bytes32;

use crate::{CatalogRegistryConstants, CliError};

pub async fn catalog_unroll_state_scheduler(
    price_singleton_launcher_id_str: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let constants = if let Some(price_singleton_launcher_id_str) = price_singleton_launcher_id_str {
        CatalogRegistryConstants::get(testnet11).with_price_singleton(Bytes32::new(
            hex::decode(price_singleton_launcher_id_str)
                .map_err(CliError::ParseHex)?
                .try_into()
                .unwrap(),
        ))
    } else {
        CatalogRegistryConstants::get(testnet11)
    };

    if constants.price_singleton_launcher_id == Bytes32::default() {
        return Err(CliError::Custom(
            "Price singleton launcher id is not set".to_string(),
        ));
    }

    Ok(())
}

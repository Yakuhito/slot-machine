use chia::protocol::Bytes32;
use chia_wallet_sdk::{hex_string_to_bytes32, CoinsetClient};

use crate::{hex_string_to_bytes32, sync_multisig_singleton, CatalogRegistryConstants, CliError};

pub async fn catalog_unroll_state_scheduler(
    price_singleton_launcher_id_str: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let constants = if let Some(price_singleton_launcher_id_str) = price_singleton_launcher_id_str {
        CatalogRegistryConstants::get(testnet11).with_price_singleton(hex_string_to_bytes32(&price_singleton_launcher_id_str)?)
    } else {
        CatalogRegistryConstants::get(testnet11)
    };

    if constants.price_singleton_launcher_id == Bytes32::default() {
        return Err(CliError::Custom(
            "Price singleton launcher id is not set".to_string(),
        ));
    }

    let cli = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };
    let price_singleton = sync_multisig_singleton(&cli, ctx, launcher_id, print_state_info)

    Ok(())
}

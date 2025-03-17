use chia_wallet_sdk::CoinsetClient;

use crate::CliError;

pub async fn catalog_continue_launch(
    _cats_per_spend: usize,
    testnet11: bool,
    _fee_str: String,
) -> Result<(), CliError> {
    println!("Time to unroll a CATalog! Yee-haw!");

    let _client = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };

    Ok(())
}

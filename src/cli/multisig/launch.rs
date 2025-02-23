use crate::CliError;

pub async fn multisig_launch(pubkeys: String, m: usize, testnet11: bool) -> Result<(), CliError> {
    println!(
        "Launching multisig with pubkeys: {} and m: {} (testnet11: {})",
        pubkeys, m, testnet11
    );

    Ok(())
}

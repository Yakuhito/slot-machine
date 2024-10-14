use crate::cli::{
    chia_client::ChiaRpcClient,
    csv::load_catalog_premine_csv,
    utils::{yes_no_prompt, CliError},
};

pub async fn initiate_catalog_launch(csv_filename: &str) -> Result<(), CliError> {
    println!("Welcome to the CATalog launch setup, deployer.");

    println!("Loading premine data...");

    let data = load_catalog_premine_csv(csv_filename)?;
    println!(
        "Loaded {} CATs to be premined. First few records:",
        data.len()
    );
    for record in data.iter().take(7) {
        println!("  code: {:?}, name: {:?}", record.code, record.name);
    }

    yes_no_prompt(
        format!(
            "Premine data was be loaded from '{}' - is this the correct data?",
            csv_filename
        )
        .as_str(),
    )?;

    // todo: debug
    let client = ChiaRpcClient::coinset_testnet11();
    let blockchain_state = client.get_blockchain_state().await.unwrap();
    println!("Current state: {:?}", blockchain_state);
    // yes_no_prompt("Spend bundle built - do you want to commence with launch?")?;

    Ok(())
}

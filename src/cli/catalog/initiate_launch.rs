use crate::cli::{
    chia_client::ChiaRpcClient,
    csv::load_catalog_premine_csv,
    utils::{yes_no_prompt, CliError},
    Db, CATALOG_LAUNCH_GENERATION_KEY, CATALOG_LAUNCH_LAUNCHER_ID_KEY,
};

pub async fn initiate_catalog_launch(testnet11: bool) -> Result<(), CliError> {
    println!("Welcome to the CATalog launch setup, deployer.");

    println!("Opening database...");
    let db = Db::new().await?;

    println!("Loading premine data...");

    let csv_filename = if testnet11 {
        "catalog_premine_testnet11.csv"
    } else {
        "catalog_premine_mainnet.csv"
    };
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

    println!("Initializing Chia RPC client...");
    let client = ChiaRpcClient::coinset(testnet11);

    let launcher_id = db.get_value_by_key(CATALOG_LAUNCH_LAUNCHER_ID_KEY).await?;
    if launcher_id.is_some() {
        yes_no_prompt("Previous deployment found in db - do you wish to override?")?;

        db.remove_key(CATALOG_LAUNCH_LAUNCHER_ID_KEY).await?;
        let generation = db.get_value_by_key(CATALOG_LAUNCH_GENERATION_KEY).await?;
        if generation.is_some() {
            db.remove_key(CATALOG_LAUNCH_GENERATION_KEY).await?;
        }
    }

    // yes_no_prompt("Spend bundle built - do you want to commence with launch?")?;

    Ok(())
}

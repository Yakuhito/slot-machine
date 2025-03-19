use chia::protocol::Bytes32;
use chia_wallet_sdk::{CoinsetClient, SpendContext};

use crate::{
    load_catalog_premine_csv, sync_catalog, CatalogRegistryConstants, CliError, Db,
    CATALOG_LAUNCH_LAUNCHER_ID_KEY,
};

pub async fn catalog_continue_launch(
    cats_per_spend: usize,
    price_singleton_launcher_id_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    println!("Time to unroll a CATalog! Yee-haw!");

    let premine_csv_filename = if testnet11 {
        "catalog_premine_testnet11.csv"
    } else {
        "catalog_premine_mainnet.csv"
    };

    println!("Loading premine data from '{}'...", premine_csv_filename);
    let cats_to_launch = load_catalog_premine_csv(premine_csv_filename)?;

    println!("Initializing Chia RPC client...");
    let client = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };

    println!("Opening database...");
    let db = Db::new().await?;

    let Some(launcher_id) = db.get_value_by_key(CATALOG_LAUNCH_LAUNCHER_ID_KEY).await? else {
        eprintln!("No launcher ID found in database - please run 'catalog initiate-launch' first");
        return Ok(());
    };
    let launcher_id = Bytes32::new(
        hex::decode(launcher_id)
            .map_err(CliError::ParseHex)?
            .try_into()
            .unwrap(),
    );

    println!("Syncing CATalog...");
    let mut ctx = SpendContext::new();

    let constants = CatalogRegistryConstants::get(testnet11).with_price_singleton(Bytes32::new(
        hex::decode(price_singleton_launcher_id_str)
            .map_err(CliError::ParseHex)?
            .try_into()
            .unwrap(),
    ));

    let catalog = sync_catalog(&client, &db, &mut ctx, launcher_id, constants).await?;
    println!("Latest catalog coin id: {}", catalog.coin.coin_id());

    Ok(())
}

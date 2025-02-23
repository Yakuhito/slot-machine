use crate::{
    cli::{
        csv::load_catalog_premine_csv,
        prompt_for_value,
        utils::{yes_no_prompt, CliError},
        Db, CATALOG_LAUNCH_CATS_PER_SPEND_KEY, CATALOG_LAUNCH_GENERATION_KEY,
        CATALOG_LAUNCH_LAUNCHER_ID_KEY,
    },
    CatalogRegistryConstants,
};
use chia::protocol::Bytes32;
use chia_wallet_sdk::{encode_address, CoinsetClient, SpendContext};
use hex::FromHex;

pub async fn catalog_initiate_launch(testnet11: bool) -> Result<(), CliError> {
    println!("Welcome to the CATalog launch setup, deployer.");

    println!("Opening database...");
    let db = Db::new().await?;

    println!("Loading premine data...");

    let csv_filename = if testnet11 {
        "catalog_premine_testnet11.csv"
    } else {
        "catalog_premine_mainnet.csv"
    };
    let cats_to_launch = load_catalog_premine_csv(csv_filename)?;
    println!(
        "Loaded {} CATs to be premined. First few records:",
        cats_to_launch.len()
    );
    for record in cats_to_launch.iter().take(7) {
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
    let client = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };

    let launcher_id = db.get_value_by_key(CATALOG_LAUNCH_LAUNCHER_ID_KEY).await?;
    if launcher_id.is_some() {
        yes_no_prompt("Previous deployment found in db - do you wish to override?")?;

        db.remove_key(CATALOG_LAUNCH_LAUNCHER_ID_KEY).await?;
        let cats_per_spend = db
            .get_value_by_key(CATALOG_LAUNCH_CATS_PER_SPEND_KEY)
            .await?;
        if cats_per_spend.is_some() {
            db.remove_key(CATALOG_LAUNCH_CATS_PER_SPEND_KEY).await?;
        }

        let generation = db.get_value_by_key(CATALOG_LAUNCH_GENERATION_KEY).await?;
        if generation.is_some() {
            db.remove_key(CATALOG_LAUNCH_GENERATION_KEY).await?;
        }
    }

    let cats_per_unroll_str =
        prompt_for_value("How many CATs should be deployed per unroll spend?")?;
    let cats_per_unroll: u64 = cats_per_unroll_str.parse().map_err(CliError::ParseInt)?;

    let constants = CatalogRegistryConstants::get(testnet11);
    let prefix = if testnet11 { "txch" } else { "xch" };
    let royalty_address =
        encode_address(constants.royalty_address.into(), prefix).map_err(CliError::Bech32)?;
    let precommit_payout_address =
        encode_address(constants.precommit_payout_puzzle_hash.into(), prefix)
            .map_err(CliError::Bech32)?;

    println!("Default constants will be used:");
    println!("  royalty address: {}", royalty_address);
    println!(
        "  royalty ten thousandths: {}",
        constants.royalty_ten_thousandths
    );
    println!("  precommit payout address: {}", precommit_payout_address);
    println!(
        "  relative block height: {}",
        constants.relative_block_height
    );
    println!("  price singleton id: (will be launched as well)");
    yes_no_prompt("Do the constants above have the correct values?")?;

    let cat_payment_asset_id_str = prompt_for_value("What is the asset id of the payment CAT?")?;
    let cat_payment_asset_id =
        Bytes32::new(<[u8; 32]>::from_hex(cat_payment_asset_id_str).map_err(CliError::ParseHex)?);

    let cat_registration_price_str =
        prompt_for_value("What should be the price of a CAT registration in CAT **MOJOS**?")?;
    let cat_registration_price: u64 = cat_registration_price_str
        .parse()
        .map_err(CliError::ParseInt)?;

    // println!("A one-sided offer (2 mojos) will be needed for launch.");
    // let offer = prompt_for_value("Offer: ")?;

    // let ctx = &mut SpendContext::new();
    // let initial_registration_price = price_schedule[0].1 * 2;
    // let (sig, _, scheduler, preroller) = initiate_catalog_launch(
    //     ctx,
    //     Offer::decode(&offer).map_err(CliError::Offer)?,
    //     price_schedule,
    //     initial_registration_price,
    //     todo!("convert cats_to_launch to a vector of AddCat"),
    //     cats_per_unroll,
    //     CatalogConstants::get(testnet11),
    //     if testnet11 {
    //         &TESTNET11_CONSTANTS
    //     } else {
    //         &MAINNET_CONSTANTS
    //     },
    // )
    // .map_err(CliError::Driver)?;

    // yes_no_prompt("Spend bundle built - do you want to commence with launch?")?;

    //     // launch
    //     // follow in mempool; wait for confirmation
    //     // save values to db for unroll

    Ok(())
}

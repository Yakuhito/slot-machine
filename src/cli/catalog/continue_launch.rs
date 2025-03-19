use chia::protocol::Bytes32;
use chia_wallet_sdk::{CoinsetClient, SpendContext};
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    load_catalog_premine_csv, parse_amount, sync_catalog, yes_no_prompt, CatalogRegistryConstants,
    CliError, Db, SageClient, CATALOG_LAUNCH_LAUNCHER_ID_KEY, CATALOG_LAUNCH_PAYMENT_ASSET_ID_KEY,
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

    println!("Finding last registered CAT from list...");
    let mut i = 0;
    while i < cats_to_launch.len() {
        let cat = &cats_to_launch[i];
        let resp = db.get_catalog_indexed_slot_value(cat.asset_id).await?;
        if resp.is_none() {
            break;
        }

        i += 1;
    }

    if i == cats_to_launch.len() {
        eprintln!("All CATs have already been registered - nothing to do!");
        return Ok(());
    }

    let mut cats = Vec::with_capacity(cats_per_spend);
    while i < cats_to_launch.len() && cats.len() < cats_per_spend {
        cats.push(cats_to_launch[i].clone());
        i += 1;
    }

    println!("These cats will be launched (total number={}):", cats.len());
    for cat in &cats {
        println!("  code: {:?}, name: {:?}", cat.code, cat.name);
    }

    let fee = parse_amount(fee_str.clone(), false)?;
    println!("A one-sided offer will be created; it will consume:");
    println!("  - {} special registration CAT mojos", cats.len());
    println!("  - {} mojos for minting CAT NFTs", cats.len());
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;

    let offer_resp = sage
        .make_offer(MakeOffer {
            requested_assets: Assets {
                xch: Amount::u64(0),
                cats: vec![],
                nfts: vec![],
            },
            offered_assets: Assets {
                xch: Amount::u64(cats.len() as u64),
                cats: vec![CatAmount {
                    asset_id: db
                        .get_value_by_key(CATALOG_LAUNCH_PAYMENT_ASSET_ID_KEY)
                        .await?
                        .unwrap(),
                    amount: Amount::u64(cats.len() as u64),
                }],
                nfts: vec![],
            },
            fee: Amount::u64(fee),
            receive_address: None,
            expires_at_second: None,
            auto_import: false,
        })
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    Ok(())
}

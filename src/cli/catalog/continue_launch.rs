use chia::{
    clvm_utils::ToTreeHash,
    protocol::Bytes32,
    puzzles::{cat::CatArgs, singleton::SingletonStruct},
};
use chia_wallet_sdk::{ChiaRpcClient, CoinsetClient, SpendContext};
use clvmr::serde::node_from_bytes;
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    load_catalog_premine_csv, parse_amount, sync_catalog, yes_no_prompt, CatNftMetadata,
    CatalogPrecommitValue, CatalogRegistryConstants, CliError, Db, PrecommitLayer, SageClient,
    CATALOG_LAUNCH_LAUNCHER_ID_KEY, CATALOG_LAUNCH_PAYMENT_ASSET_ID_KEY,
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

    let payment_asset_id_str = db
        .get_value_by_key(CATALOG_LAUNCH_PAYMENT_ASSET_ID_KEY)
        .await?
        .unwrap();
    let payment_asset_id = Bytes32::new(
        hex::decode(payment_asset_id_str.clone())
            .map_err(CliError::ParseHex)?
            .try_into()
            .unwrap(),
    );

    if i == 0 {
        println!("No CATs registered yet - looking for precommitment coins...");
        let mut i = 0;
        while i < cats_to_launch.len() {
            let cat = &cats_to_launch[i];
            let tail_ptr = node_from_bytes(&mut ctx.allocator, &cat.tail)?;
            let tail_hash = ctx.tree_hash(tail_ptr);
            if tail_hash != cat.asset_id.into() {
                eprintln!("CAT {} has a tail hash mismatch - aborting", cat.asset_id);
                return Err(CliError::Custom("TAIL hash mismatch".to_string()));
            }

            let initial_inner_puzzle_ptr = CatalogPrecommitValue::<()>::initial_inner_puzzle(
                &mut ctx,
                cat.owner,
                CatNftMetadata {
                    ticker: cat.code.clone(),
                    name: cat.name.clone(),
                    description: "".to_string(),
                    precision: cat.precision,
                    image_uris: cat.image_uris.clone(),
                    image_hash: cat.image_hash,
                    metadata_uris: vec![],
                    metadata_hash: None,
                    license_uris: vec![],
                    license_hash: None,
                },
            )?;
            let precommit_value = CatalogPrecommitValue::with_default_cat_maker(
                payment_asset_id.tree_hash(),
                ctx.tree_hash(initial_inner_puzzle_ptr).into(),
                tail_hash, // treehash
            );

            let precommit_inner_puzzle = PrecommitLayer::<CatalogPrecommitValue>::puzzle_hash(
                SingletonStruct::new(launcher_id).tree_hash().into(),
                constants.relative_block_height,
                constants.precommit_payout_puzzle_hash,
                Bytes32::default(),
                precommit_value.tree_hash(),
            );

            let precommit_puzzle =
                CatArgs::curry_tree_hash(payment_asset_id, precommit_inner_puzzle);

            let records_resp = client
                .get_coin_records_by_puzzle_hash(precommit_puzzle.into(), None, None, Some(true))
                .await?;
            if let Some(records) = records_resp.coin_records {
                if !records.is_empty() {
                    break;
                }
            }

            i += 1;
        }

        // todo: if there are unlaunched precommitment coins, launch those first and exit
    }

    println!("These cats will be launched (total number={}):", cats.len());
    for cat in &cats {
        println!("  code: {:?}, name: {:?}", cat.code, cat.name);
    }

    let fee = parse_amount(fee_str.clone(), false)?;
    println!("A one-sided offer will be created; it will consume:");
    println!("  - {} mojos for minting CAT NFTs", cats.len());
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;

    // todo: check if precommitment coins are available and have the appropriate age

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
                    asset_id: payment_asset_id_str,
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

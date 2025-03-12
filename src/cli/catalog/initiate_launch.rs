use crate::{
    cli::{
        csv::load_catalog_premine_csv,
        prompt_for_value,
        utils::{yes_no_prompt, CliError},
        Db, CATALOG_LAUNCH_CATS_PER_SPEND_KEY, CATALOG_LAUNCH_GENERATION_KEY,
        CATALOG_LAUNCH_LAUNCHER_ID_KEY,
    },
    get_alias_map, launch_catalog_registry, load_catalog_state_schedule_csv, parse_amount,
    CatalogRegistryConstants, CatalogRegistryState, DefaultCatMakerArgs, MedievalVaultHint,
    MedievalVaultInfo, SageClient, StateSchedulerInfo,
};
use chia::{
    bls::PublicKey,
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
};
use chia_wallet_sdk::{
    encode_address, Cat, CoinsetClient, Conditions, DriverError, Launcher, Memos, Offer,
    SpendContext,
};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, GetDerivations, MakeOffer};

fn get_additional_info_for_launch(
    ctx: &mut SpendContext,
    catalog_launcher_id: Bytes32,
    security_coin: Coin,
    (
        catalog_constants,
        state_schedule,
        medieval_vault_memos,
        cat_amount,
        cat_destination_puzzle_hash,
    ): (
        CatalogRegistryConstants,
        Vec<(u32, CatalogRegistryState)>,
        MedievalVaultHint,
        u64,
        Bytes32,
    ),
) -> Result<(Conditions<NodePtr>, CatalogRegistryConstants, Bytes32), DriverError> {
    let mut conditions = Conditions::new();

    let price_singleton_launcher =
        Launcher::new(security_coin.coin_id(), 3).with_singleton_amount(1);
    let price_singleton_launcher_coin = price_singleton_launcher.coin();
    let price_singleton_launcher_id = price_singleton_launcher_coin.coin_id();

    let medieval_vault_memos_ptr = ctx.alloc(&medieval_vault_memos)?;
    let multisig_info = MedievalVaultInfo::from_hint(medieval_vault_memos);
    let state_scheduler_info = StateSchedulerInfo::new(
        price_singleton_launcher_id,
        catalog_launcher_id,
        state_schedule,
        0,
        multisig_info.inner_puzzle_hash().into(),
    );
    let (price_singleton_launch_conds, _coin) = price_singleton_launcher.spend(
        ctx,
        state_scheduler_info.inner_puzzle_hash().into(),
        state_scheduler_info.to_hints(medieval_vault_memos_ptr),
    )?;
    conditions = conditions.extend(price_singleton_launch_conds);

    let cat_memos = Memos::some(ctx.alloc(&cat_destination_puzzle_hash)?);
    let (cat_creation_conds, eve_cat) = Cat::single_issuance_eve(
        ctx,
        security_coin.coin_id(),
        cat_amount,
        Conditions::new().create_coin(cat_destination_puzzle_hash, cat_amount, cat_memos),
    )?;
    conditions = conditions.extend(cat_creation_conds);

    Ok((
        conditions,
        catalog_constants.with_price_singleton(price_singleton_launcher_id),
        eve_cat.asset_id,
    ))
}

pub async fn catalog_initiate_launch(
    pubkeys_str: String,
    m: usize,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    println!("Welcome to the CATalog launch setup, deployer.");

    let alias_map = get_alias_map()?;
    let mut pubkeys = Vec::new();
    for pubkey_str in pubkeys_str.split(',') {
        let pubkey = PublicKey::from_bytes(
            &hex::decode(pubkey_str.trim().replace("0x", ""))
                .map_err(CliError::ParseHex)?
                .try_into()
                .unwrap(),
        )
        .map_err(CliError::InvalidPublicKey)?;
        pubkeys.push(pubkey);
    }

    let fee = parse_amount(fee_str.clone(), false)?;

    println!("First things first, this multisig will have control over the price singleton once the state schedule is over:");
    println!("  Public Key List:");
    for pubkey in pubkeys.iter() {
        println!(
            "    - {}",
            alias_map
                .get(pubkey)
                .unwrap_or(&format!("0x{}", hex::encode(pubkey.to_bytes())))
        );
    }
    println!("  Signature Threshold: {}", m);
    println!("  Testnet: {}", testnet11);

    let price_schedule_csv_filename = if testnet11 {
        "catalog_price_schedule_testnet11.csv"
    } else {
        "catalog_price_schedule_mainnet.csv"
    };
    println!(
        "Loading price schedule from '{}'...",
        price_schedule_csv_filename
    );

    let price_schedule = load_catalog_state_schedule_csv(price_schedule_csv_filename)?;
    println!("Price schedule:");
    for record in price_schedule.iter() {
        println!(
            "  After block height {}, a registration will cost {} CAT mojos (asset id: {}).",
            record.block_height, record.registration_price, record.asset_id
        );
    }

    let premine_csv_filename = if testnet11 {
        "catalog_premine_testnet11.csv"
    } else {
        "catalog_premine_mainnet.csv"
    };

    println!("Loading premine data from '{}'...", premine_csv_filename);
    let cats_to_launch = load_catalog_premine_csv(premine_csv_filename)?;
    println!(
        "Loaded {} CATs to be premined. First few records:",
        cats_to_launch.len()
    );
    for record in cats_to_launch.iter().take(7) {
        println!("  code: {:?}, name: {:?}", record.code, record.name);
    }

    yes_no_prompt("Is all the data above correct?")?;

    println!("Initializing Chia RPC client...");
    let _client = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };

    println!("Opening database...");
    let db = Db::new().await?;

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
    let _cats_per_unroll: u64 = cats_per_unroll_str.parse().map_err(CliError::ParseInt)?;

    db.save_key_value(CATALOG_LAUNCH_CATS_PER_SPEND_KEY, &cats_per_unroll_str)
        .await?;
    db.save_key_value(CATALOG_LAUNCH_GENERATION_KEY, "0")
        .await?;

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

    println!("A one-sided offer ({} mojos) will be needed for launch. The value will be distributed as follows:", 2 + cats_to_launch.len());
    println!("  CATalog registry singleton - 1 mojo");
    println!("  CATalog price singleton - 1 mojo");
    println!(
        "  CATalog premine registration CAT - {} mojos",
        cats_to_launch.len()
    );
    println!(
        "The offer will also use {} XCH ({} mojos) as fee.",
        fee_str, fee
    );

    let sage = SageClient::new()?;
    let derivation_resp = sage
        .get_derivations(GetDerivations {
            hardened: false,
            offset: 0,
            limit: 1,
        })
        .await?;
    println!(
        "Newly-minted CATs will be sent to the active wallet (address: {})",
        derivation_resp.derivations[0].address
    );

    yes_no_prompt("Do you want to continue generating the offer?")?;

    let offer_resp = sage
        .make_offer(MakeOffer {
            requested_assets: Assets {
                xch: Amount::u64(0),
                cats: vec![],
                nfts: vec![],
            },
            offered_assets: Assets {
                xch: Amount::u64(2 + cats_to_launch.len() as u64),
                cats: vec![],
                nfts: vec![],
            },
            fee: Amount::u64(fee),
            receive_address: None,
            expires_at_second: None,
            auto_import: false,
        })
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let ctx = &mut SpendContext::new();

    let (sig, _, scheduler, preroller) = launch_catalog_registry(
        ctx,
        Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?,
        1,
        get_additional_info_for_launch,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
        (
            CatalogRegistryConstants::get(testnet11),
            price_schedule
                .into_iter()
                .map(|ps| {
                    (
                        ps.block_height,
                        CatalogRegistryState {
                            cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(
                                ps.asset_id.tree_hash().into(),
                            )
                            .into(),
                            registration_price: ps.registration_price,
                        },
                    )
                })
                .collect(),
            medieval_vault_memos,
            cat_amount,
            cat_destination_puzzle_hash,
        ),
    )
    .map_err(CliError::Driver)?;

    // yes_no_prompt("Spend bundle built - do you want to commence with launch?")?;

    //     // launch
    //     // follow in mempool; wait for confirmation
    //     // save values to db for unroll

    Ok(())
}

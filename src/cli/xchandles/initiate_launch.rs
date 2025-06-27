use crate::{
    assets_xch_only,
    cli::{
        utils::{yes_no_prompt, CliError},
        Db,
    },
    get_coinset_client, get_prefix, launch_xchandles_registry, load_xchandles_premine_csv,
    load_xchandles_state_schedule_csv, no_assets, parse_amount, print_medieval_vault_configuration,
    wait_for_coin, MedievalVaultHint, MedievalVaultInfo, SageClient, StateSchedulerInfo,
    XchandlesConstants, XchandlesFactorPricingPuzzleArgs, XchandlesRegistryState,
};
use chia::{
    bls::PublicKey,
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin, SpendBundle},
    puzzles::cat::GenesisByCoinIdTailArgs,
};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Cat, DriverError, Launcher, Offer, SpendContext},
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
    utils::Address,
};
use clvmr::NodePtr;

#[allow(clippy::type_complexity)]
fn get_additional_info_for_launch(
    ctx: &mut SpendContext,
    xchandles_launcher_id: Bytes32,
    security_coin: Coin,
    (xchandles_constants, state_schedule, pubkeys, m, cat_amount, cat_destination_puzzle_hash): (
        XchandlesConstants,
        Vec<(u32, XchandlesRegistryState)>,
        Vec<PublicKey>,
        usize,
        u64,
        Bytes32,
    ),
) -> Result<(Conditions<NodePtr>, XchandlesConstants, Bytes32), DriverError> {
    println!(
        "XCHandles registry launcher id (SAVE THIS): {}",
        hex::encode(xchandles_launcher_id)
    );

    let mut conditions = Conditions::new();

    let price_singleton_launcher =
        Launcher::new(security_coin.coin_id(), 3).with_singleton_amount(1);
    let price_singleton_launcher_coin = price_singleton_launcher.coin();
    let price_singleton_launcher_id = price_singleton_launcher_coin.coin_id();

    let medieval_vault_memos = MedievalVaultHint {
        my_launcher_id: price_singleton_launcher_id,
        public_key_list: pubkeys,
        m,
    };
    let medieval_vault_memos_ptr = ctx.alloc(&medieval_vault_memos)?;
    let multisig_info = MedievalVaultInfo::from_hint(medieval_vault_memos);
    let state_scheduler_info = StateSchedulerInfo::new(
        price_singleton_launcher_id,
        xchandles_launcher_id,
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

    let cat_memos = ctx.hint(cat_destination_puzzle_hash)?;
    let (cat_creation_conds, eve_cat) = Cat::issue_with_coin(
        ctx,
        security_coin.coin_id(),
        cat_amount,
        Conditions::new().create_coin(cat_destination_puzzle_hash, cat_amount, cat_memos),
    )?;
    conditions = conditions.extend(cat_creation_conds);

    println!(
        "Premine payment asset id: {}",
        hex::encode(eve_cat[0].info.asset_id)
    );
    println!(
        "Price singleton id (SAVE THIS): {}",
        hex::encode(price_singleton_launcher_id)
    );

    Ok((
        conditions,
        xchandles_constants
            .with_price_singleton(price_singleton_launcher_id)
            .with_launcher_id(xchandles_launcher_id),
        eve_cat[0].info.asset_id,
    ))
}

pub async fn xchandles_initiate_launch(
    pubkeys_str: String,
    m: usize,
    payout_address: String,
    relative_block_height: u32,
    registration_period: u64,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let payout_info = Address::decode(&payout_address)?;

    println!("Welcome to the XCHandles (sub)registry launch setup, deployer.");

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

    let fee = parse_amount(&fee_str, false)?;

    println!("First things first, this multisig will have control over the price singleton once the state schedule is over:");
    print_medieval_vault_configuration(m, &pubkeys)?;
    println!("  Testnet: {}", testnet11);

    let price_schedule_csv_filename = if testnet11 {
        "xchandles_price_schedule_testnet11.csv"
    } else {
        "xchandles_price_schedule_mainnet.csv"
    };
    println!(
        "Loading price schedule from '{}'...",
        price_schedule_csv_filename
    );

    let price_schedule = load_xchandles_state_schedule_csv(price_schedule_csv_filename)?;
    println!("Price schedule:");
    for record in price_schedule.iter() {
        println!(
            "  After block height {}, the base registration price will be {} CAT mojos (asset id: {}).",
            record.block_height, record.registration_price, record.asset_id
        );
    }

    let premine_csv_filename = if testnet11 {
        "xchandles_premine_testnet11.csv"
    } else {
        "xchandles_premine_mainnet.csv"
    };

    println!("Loading premine data from '{}'...", premine_csv_filename);
    let handles_to_launch = load_xchandles_premine_csv(premine_csv_filename)?;
    println!(
        "Loaded {} handles to be premined. First few records:",
        handles_to_launch.len()
    );
    for record in handles_to_launch.iter().take(7) {
        println!(
            "  handle: {:}, owner_nft: {:}",
            record.handle, record.owner_nft
        );
    }

    yes_no_prompt("Is all the data above correct?")?;

    println!("Initializing RPC client...");
    let client = get_coinset_client(testnet11);

    println!("Opening database...");
    let db = Db::new(false).await?;

    let constants = XchandlesConstants {
        launcher_id: Bytes32::default(),
        precommit_payout_puzzle_hash: payout_info.puzzle_hash,
        relative_block_height,
        price_singleton_launcher_id: Bytes32::default(),
    };

    let prefix = get_prefix(testnet11);
    if prefix != payout_info.prefix {
        return Err(CliError::Custom(format!(
            "Wrong prefix in payout address: expected {}, got {}",
            prefix, payout_info.prefix
        )));
    }
    let precommit_payout_address =
        Address::new(constants.precommit_payout_puzzle_hash, prefix).encode()?;

    println!("Default constants will be used:");
    println!("  precommit payout address: {}", precommit_payout_address);
    println!(
        "  relative block height: {}",
        constants.relative_block_height
    );
    println!("  price singleton id: (will be launched as well)");
    yes_no_prompt("Do the constants above have the correct values?")?;

    let mut value_needed_for_registration = 0;
    for handle_record in handles_to_launch.iter() {
        value_needed_for_registration +=
            XchandlesFactorPricingPuzzleArgs::get_price(1, &handle_record.handle, 1);
    }

    println!("A one-sided offer ({} mojos) will be needed for launch. The value will be distributed as follows:", 2 + value_needed_for_registration);
    println!("  XCHandles registry singleton - 1 mojo");
    println!("  XCHandles price singleton - 1 mojo");
    println!(
        "  XCHandles premine registration CAT - {} mojos",
        value_needed_for_registration
    );
    println!(
        "The offer will also use {} XCH ({} mojos) as fee.",
        fee_str, fee
    );

    let sage = SageClient::new()?;
    let derivation_resp = sage.get_derivations(false, 0, 1).await?;
    println!(
        "Newly-minted CATs will be sent to the active wallet (address: {})",
        derivation_resp.derivations[0].address
    );

    yes_no_prompt("Do you want to continue generating the offer?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_only(2 + value_needed_for_registration),
            fee,
            None,
            None,
            false,
        )
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let mut ctx = SpendContext::new();

    let (sig, _, registry, slots, security_coin) = launch_xchandles_registry(
        &mut ctx,
        Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?,
        1,
        registration_period,
        get_additional_info_for_launch,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
        (
            constants,
            price_schedule
                .into_iter()
                .map(|ps| {
                    (
                        ps.block_height,
                        XchandlesRegistryState::from(
                            ps.asset_id.tree_hash().into(),
                            ps.registration_price,
                            ps.registration_period,
                        ),
                    )
                })
                .collect(),
            pubkeys,
            m,
            value_needed_for_registration,
            Address::decode(&derivation_resp.derivations[0].address)?.puzzle_hash,
        ),
    )
    .map_err(CliError::Driver)?;

    let premine_payment_asset_id: Bytes32 =
        GenesisByCoinIdTailArgs::curry_tree_hash(security_coin.coin_id()).into();
    println!(
        "Premine payment asset id (SAVE THIS): {}",
        hex::encode(premine_payment_asset_id)
    );

    yes_no_prompt("Spend bundle built - do you want to commence with launch?")?;

    db.save_xchandles_configuration(&mut ctx, registry.info.constants)
        .await?;

    for slot in slots {
        db.save_xchandles_indexed_slot_value(
            registry.info.constants.launcher_id,
            slot.info.value.handle_hash,
            slot.info.value_hash,
        )
        .await?;
        db.save_slot(&mut ctx, slot, 0).await?;
    }

    let spend_bundle = SpendBundle::new(ctx.take(), sig);

    println!("Submitting transaction...");
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

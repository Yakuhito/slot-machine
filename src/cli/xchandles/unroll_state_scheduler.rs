use chia::{clvm_utils::ToTreeHash, protocol::SpendBundle};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, Offer, SpendContext},
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};

use crate::{
    assets_xch_only, create_security_coin, get_coinset_client, hex_string_to_bytes32,
    load_xchandles_state_schedule_csv, no_assets, parse_amount, quick_sync_xchandles,
    spend_security_coin, sync_multisig_singleton, sync_xchandles, wait_for_coin, yes_no_prompt,
    CliError, Db, DefaultCatMakerArgs, DelegatedStateAction, MultisigSingleton, SageClient,
    XchandlesExponentialPremiumRenewPuzzleArgs, XchandlesFactorPricingPuzzleArgs,
    XchandlesRegistryState,
};

pub async fn xchandles_unroll_state_scheduler(
    launcher_id_str: String,
    testnet11: bool,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    let cli = get_coinset_client(testnet11);
    let mut db = Db::new(false).await?;
    let mut ctx = SpendContext::new();

    let mut registry = if local {
        sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?
    } else {
        quick_sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?
    };

    let (MultisigSingleton::StateScheduler(state_scheduler), _) =
        sync_multisig_singleton::<XchandlesRegistryState>(
            &cli,
            &mut ctx,
            registry.info.constants.price_singleton_launcher_id,
            None,
        )
        .await?
    else {
        return Err(CliError::Custom(
            "Price singleton is not (or no longer) a state scheduler".to_string(),
        ));
    };

    let sage = SageClient::new()?;
    let fee = parse_amount(&fee_str, false)?;

    let (required_height, new_state) =
        state_scheduler.info.state_schedule[state_scheduler.info.generation];

    if let Some(blockchain_state) = cli.get_blockchain_state().await?.blockchain_state {
        if blockchain_state.peak.height < required_height {
            return Err(CliError::Custom(format!(
                "Current blockchain height is {}, but required height for new state is {}",
                blockchain_state.peak.height, required_height
            )));
        }
    } else {
        println!(
            "Couldn't check current blockchain height; will assume needed height was acheived"
        );
    }

    println!(
        "Next state sets a pricing puzzle hash of {} and an expired handle pricing puzzle hash of {} with CAT maker puzzle hash={}",
        hex::encode(new_state.pricing_puzzle_hash),
        hex::encode(new_state.expired_handle_pricing_puzzle_hash),
        hex::encode(new_state.cat_maker_puzzle_hash)
    );

    let filename = if testnet11 {
        "xchandles_price_schedule_testnet11.csv"
    } else {
        "xchandles_price_schedule_mainnet.csv"
    };
    let schedule = load_xchandles_state_schedule_csv(filename)?;
    let mut found = false;
    for record in schedule.iter() {
        let cmph = DefaultCatMakerArgs::curry_tree_hash(record.asset_id.tree_hash().into());
        let pph = XchandlesFactorPricingPuzzleArgs::curry_tree_hash(
            record.registration_price,
            record.registration_period,
        );
        let eph = XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(
            record.registration_price,
            record.registration_period,
            1000,
        );
        if cmph == new_state.cat_maker_puzzle_hash.into()
            && pph == new_state.pricing_puzzle_hash.into()
            && eph == new_state.expired_handle_pricing_puzzle_hash.into()
        {
            println!(
                "These hashes correspond to a base price of {} mojos of the CAT with asset_id={}",
                record.registration_price,
                hex::encode(record.asset_id)
            );
            found = true;
        }
    }
    if !found {
        println!("Could *NOT* figure out what those hashes translate to.");
        println!("PROCEED WITH CAUTION.\n\n\n")
    }

    println!("An offer will be generated offering:");
    println!(" - 1 mojo");
    println!(" - {} XCH ({} mojos) as fee", fee_str, fee);
    yes_no_prompt("The state scheduler and the XCHandles registry have been synced. This is the last check - do you wish to continue?")?;

    // spend state scheduler & CATalog

    // no need to include security conditions as we assert the state scheduler is spent
    // which means the right message is consumed
    let (_action_secure_conds, registry_action_spend) = registry
        .new_action::<DelegatedStateAction>()
        .spend::<XchandlesRegistryState>(
            &mut ctx,
            registry.coin,
            new_state,
            state_scheduler.info.inner_puzzle_hash().into(),
        )?;
    registry.insert_action_spend(&mut ctx, registry_action_spend)?;

    let registry_inner_ph = registry.info.inner_puzzle_hash();
    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let security_coin_conditions = Conditions::new()
        .assert_concurrent_spend(state_scheduler.coin.coin_id())
        .reserve_fee(1);

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        security_coin_conditions,
        &security_coin_sk,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
    )?;

    state_scheduler.spend(&mut ctx, registry_inner_ph.into())?;

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&cli, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

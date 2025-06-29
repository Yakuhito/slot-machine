use chia::protocol::{Bytes32, SpendBundle};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, Offer, SpendContext},
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};

use crate::{
    assets_xch_only, create_security_coin, get_coinset_client, no_assets, parse_amount,
    spend_security_coin, sync_multisig_singleton, wait_for_coin, yes_no_prompt,
    CatalogRegistryConstants, CatalogRegistryState, CliError, Db, DelegatedStateAction,
    MultisigSingleton, SageClient,
};

use super::sync_catalog;

pub async fn catalog_unroll_state_scheduler(
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let constants = CatalogRegistryConstants::get(testnet11);

    if constants.price_singleton_launcher_id == Bytes32::default()
        || constants.launcher_id == Bytes32::default()
    {
        return Err(CliError::ConstantsNotSet);
    }

    let cli = get_coinset_client(testnet11);
    let mut ctx = SpendContext::new();

    let (MultisigSingleton::StateScheduler(state_scheduler), _) =
        sync_multisig_singleton::<CatalogRegistryState>(
            &cli,
            &mut ctx,
            constants.price_singleton_launcher_id,
            None,
        )
        .await?
    else {
        return Err(CliError::Custom(
            "Price singleton is not (or no longer) a state scheduler".to_string(),
        ));
    };

    let mut db = Db::new(false).await?;

    let mut catalog = sync_catalog(&cli, &mut db, &mut ctx, constants).await?;

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
        "Next state sets a price of {} mojos with CAT maker puzzle hash={}",
        new_state.registration_price,
        hex::encode(new_state.cat_maker_puzzle_hash)
    );

    println!("An offer will be generated offering:");
    println!(" - 1 mojo");
    println!(" - {} XCH ({} mojos) as fee", fee_str, fee);
    yes_no_prompt("The state scheduler and the CATalog registry have been synced. This is the last check - do you wish to continue?")?;

    // spend state scheduler & CATalog

    // no need to include security conditions as we assert the state scheduler is spent
    // which means the right message is consumed
    let (_action_secure_conds, catalog_action_spend) = catalog
        .new_action::<DelegatedStateAction>()
        .spend::<CatalogRegistryState>(
            &mut ctx,
            catalog.coin,
            new_state,
            state_scheduler.info.inner_puzzle_hash().into(),
        )?;
    catalog.insert_action_spend(&mut ctx, catalog_action_spend)?;

    let catalog_inner_ph = catalog.info.inner_puzzle_hash();
    let _new_catalog = catalog.finish_spend(&mut ctx)?;

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

    state_scheduler.spend(&mut ctx, catalog_inner_ph.into())?;

    let sb = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&cli, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

use crate::{
    assets_xch_only, create_security_coin, find_reward_slot, get_coinset_client, get_constants,
    hex_string_to_bytes32, no_assets, parse_amount, spend_security_coin, sync_distributor,
    wait_for_coin, yes_no_prompt, CliError, Db, RewardDistributorNewEpochAction,
    RewardDistributorSyncAction, SageClient,
};
use chia::protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, Offer, SpendContext},
};

pub async fn reward_distributor_new_epoch(
    launcher_id_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();

    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let next_epoch_start = distributor.info.state.round_time_info.epoch_end;
    if distributor.info.state.round_time_info.last_update < next_epoch_start {
        println!(
            "Distributor last updated at {}; will also sync up until epoch end",
            distributor.info.state.round_time_info.last_update
        );

        // no reason to assert this as the next action would fail if the
        // distributor was not synced
        let _conds = distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut distributor, next_epoch_start)?;
    }

    println!("Finding appropriate reward slot...");
    let reward_slot = find_reward_slot(
        &mut ctx,
        &client,
        distributor.info.constants,
        next_epoch_start,
    )
    .await?;

    println!("A one-sided offer will be created. It will contain:");
    println!("  1 mojo",);
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let (sec_conds, fee) = distributor
        .new_action::<RewardDistributorNewEpochAction>()
        .spend(&mut ctx, &mut distributor, reward_slot)?;
    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

    println!("Fee for new epoch: {} CAT mojos", fee);

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        sec_conds,
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let spend_bundle = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

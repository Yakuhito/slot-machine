use chia::protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Offer, SpendContext},
};
use sage_api::{Amount, Assets, MakeOffer};

use crate::{
    find_reward_slot_for_epoch, get_coinset_client, get_constants, hex_string_to_bytes32, new_sk,
    parse_amount, parse_one_sided_offer, spend_security_coin, sync_distributor, wait_for_coin,
    yes_no_prompt, CliError, Db, DigNewEpochAction, DigSyncAction, SageClient,
};

pub async fn dig_new_epoch(
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
        let _conds = distributor.new_action::<DigSyncAction>().spend(
            &mut ctx,
            &mut distributor,
            next_epoch_start,
        )?;
    }

    println!("Finding appropriate reward slot...");
    let reward_slot = find_reward_slot_for_epoch(
        &mut ctx,
        &db,
        launcher_id,
        next_epoch_start,
        distributor.info.constants.epoch_seconds,
    )
    .await?
    .ok_or(CliError::Custom("No reward slot found".to_string()))?;

    println!("A one-sided offer will be created. It will contain:");
    println!("  1 mojo",);
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

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
                xch: Amount::u64(1),
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

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, false)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let (sec_conds, _new_slot, validator_fee) = distributor
        .new_action::<DigNewEpochAction>()
        .spend(&mut ctx, &mut distributor, reward_slot)?;
    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

    println!("Validator fee for new epoch: {} CAT mojos", validator_fee);

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer.security_base_conditions.extend(sec_conds),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let spend_bundle =
        SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig);

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

use chia::protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Offer, SpendContext},
    types::Conditions,
};
use sage_api::{Amount, Assets, MakeOffer};

use crate::{
    find_entry_slot_for_puzzle_hash, get_coinset_client, get_constants, get_last_onchain_timestamp,
    hex_string_to_bytes32, new_sk, parse_amount, parse_one_sided_offer, spend_security_coin,
    sync_distributor, wait_for_coin, yes_no_prompt, CliError, Db,
    RewardDistributorInitiatePayoutAction, RewardDistributorSyncAction, SageClient,
};

pub async fn reward_distributor_initiate_payout(
    launcher_id_str: String,
    payout_puzzle_hash_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let payout_puzzle_hash = hex_string_to_bytes32(&payout_puzzle_hash_str)?;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();

    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let update_time = get_last_onchain_timestamp(&client).await?;
    if update_time > distributor.info.state.round_time_info.epoch_end {
        return Err(CliError::Custom(
            "The current epoch has already ended - start a new epoch first".to_string(),
        ));
    }

    let also_sync = distributor.info.state.round_time_info.last_update < update_time - 180;
    if also_sync {
        println!(
            "Will also sync the distributor to timestamp {}",
            update_time
        );
    }

    println!("Finding reward slot...");
    let slot =
        find_entry_slot_for_puzzle_hash(&mut ctx, &db, launcher_id, payout_puzzle_hash, None)
            .await?
            .ok_or(CliError::SlotNotFound("Mirror reward"))?;

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
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let mut sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut distributor, update_time)?
    } else {
        Conditions::new()
    };

    let (new_conds, _new_slot, payout_amount) = distributor
        .new_action::<RewardDistributorInitiatePayoutAction>()
        .spend(&mut ctx, &mut distributor, slot)?;
    sec_conds = sec_conds.extend(new_conds);
    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

    println!("Payout amount: {} CAT mojos", payout_amount);

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

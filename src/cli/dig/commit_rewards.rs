use chia::protocol::{Bytes32, SpendBundle};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{CatSpend, Offer, Spend, SpendContext},
    utils::Address,
};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    find_reward_slot_for_epoch, get_coinset_client, get_constants, hex_string_to_bytes32, new_sk,
    parse_amount, parse_one_sided_offer, spend_security_coin, sync_distributor, wait_for_coin,
    yes_no_prompt, CliError, Db, DigCommitIncentivesAction, SageClient,
};

pub async fn dig_commit_rewards(
    launcher_id_str: String,
    reward_amount_str: String,
    epoch_start: u64,
    clawback_address: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let reward_amount = parse_amount(&reward_amount_str, true)?;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();
    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    println!("A one-sided offer will be created. It will contain:");
    println!(
        "  {} reward CATs ({} CAT mojos) to add to the committed rewards",
        reward_amount_str, reward_amount
    );
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    println!("\nWARNING: Only addresses from Sage (standard puzzle) will be able to claw back the commitments via this CLI.\n");
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
    let clawback_ph = Address::decode(&clawback_address)?.puzzle_hash;

    let offer_resp = sage
        .make_offer(MakeOffer {
            requested_assets: Assets {
                xch: Amount::u64(0),
                cats: vec![],
                nfts: vec![],
            },
            offered_assets: Assets {
                xch: Amount::u64(1),
                cats: vec![CatAmount {
                    asset_id: hex::encode(distributor.info.constants.reserve_asset_id),
                    amount: Amount::u64(reward_amount),
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

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let cat_destination_inner_puzzle = ctx.alloc(&(1, ()))?;
    let cat_destination_inner_puzzle_hash: Bytes32 =
        ctx.tree_hash(cat_destination_inner_puzzle).into();
    let offer = parse_one_sided_offer(
        &mut ctx,
        offer,
        security_coin_sk.public_key(),
        Some(cat_destination_inner_puzzle_hash),
        false,
    )?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let reward_slot = find_reward_slot_for_epoch(
        &mut ctx,
        &db,
        launcher_id,
        epoch_start,
        distributor.info.constants.epoch_seconds,
    )
    .await?
    .ok_or(CliError::Custom(
        "Reward slot value hash could not be found".to_string(),
    ))?;

    let (sec_conds, _slot1, _slot2) = distributor
        .new_action::<DigCommitIncentivesAction>()
        .spend(
            &mut ctx,
            &mut distributor,
            reward_slot,
            epoch_start,
            clawback_ph,
            reward_amount,
        )?;
    let _new_distributor = distributor.finish_spend(
        &mut ctx,
        vec![CatSpend {
            cat: offer.created_cat.unwrap(),
            inner_spend: Spend::new(cat_destination_inner_puzzle, NodePtr::NIL),
            extra_delta: 0,
        }],
    )?;

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

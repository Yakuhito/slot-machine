use chia::protocol::{Bytes32, SpendBundle};
use chia_wallet_sdk::{decode_address, CatSpend, ChiaRpcClient, Offer, Spend, SpendContext};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, new_sk, parse_amount,
    parse_one_sided_offer, spend_security_coin, sync_distributor, wait_for_coin, yes_no_prompt,
    CliError, Db, DigCommitIncentivesAction, DigRewardSlotValue, DigSlotNonce, SageClient,
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

    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
    let clawback_ph = Bytes32::new(decode_address(&clawback_address)?.0);

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
                    amount: Amount::u64(1),
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

    let mut next_slot_epoch = epoch_start;
    let mut reward_slot_value_hash = None;

    let mut n = 0;
    while reward_slot_value_hash.is_none() && n <= 52 {
        reward_slot_value_hash = db
            .get_dig_indexed_slot_value_by_epoch_start(next_slot_epoch)
            .await?;
        next_slot_epoch -= distributor.info.constants.epoch_seconds;
        n += 1;
    }

    let reward_slot_value_hash = reward_slot_value_hash.ok_or(CliError::Custom(
        "Reward slot value hash could not be found".to_string(),
    ))?;
    let reward_slot = db
        .get_slot::<DigRewardSlotValue>(
            &mut ctx.allocator,
            launcher_id,
            DigSlotNonce::REWARD.to_u64(),
            reward_slot_value_hash,
            0,
        )
        .await?
        .ok_or(CliError::Custom(
            "Reward slot could not be found".to_string(),
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

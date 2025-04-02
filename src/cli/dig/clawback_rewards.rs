use chia::protocol::{Bytes32, Coin, SpendBundle};
use chia_wallet_sdk::{decode_address, Offer, Spend, SpendContext, StandardLayer};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, MakeOffer};

use crate::{
    find_commitment_slot_for_puzzle_hash, find_reward_slot_for_epoch, get_coin_public_key,
    get_coinset_client, get_constants, hex_string_to_bytes32, new_sk, parse_amount,
    parse_one_sided_offer, print_spend_bundle_to_file, spend_security_coin, sync_distributor,
    yes_no_prompt, CliError, Db, DigWithdrawIncentivesAction, SageClient,
};

pub async fn dig_clawback_rewards(
    launcher_id_str: String,
    clawback_address: String,
    epoch_start: Option<u64>,
    reward_amount_str: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let reward_amount = reward_amount_str
        .map(|s| parse_amount(&s, true))
        .transpose()?;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();
    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    println!("Fetching slots...");
    let clawback_ph = Bytes32::new(decode_address(&clawback_address)?.0);
    let commitment_slot = find_commitment_slot_for_puzzle_hash(
        &mut ctx,
        &db,
        launcher_id,
        clawback_ph,
        epoch_start,
        reward_amount,
    )
    .await?
    .ok_or(CliError::Custom(
        "Commitment slot could not be found".to_string(),
    ))?;
    let reward_slot = find_reward_slot_for_epoch(
        &mut ctx,
        &db,
        launcher_id,
        commitment_slot.info.value.epoch_start,
        distributor.info.constants.epoch_seconds,
    )
    .await?
    .ok_or(CliError::Custom(
        "Reward slot could not be found".to_string(),
    ))?;

    println!(
        "Will use commitment slot with rewards={} for epoch_start={}",
        commitment_slot.info.value.rewards, commitment_slot.info.value.epoch_start
    );

    println!("A one-sided offer will be created. It will contain:");
    println!("  1 mojo",);
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);
    println!("Additionally, another 1-mojo coin with the clawback puzzle will be automatically created and used.");

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

    let (sec_conds, _slot1, _slot2) = distributor
        .new_action::<DigWithdrawIncentivesAction>()
        .spend(&mut ctx, &mut distributor, commitment_slot, reward_slot)?;
    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer
            .security_base_conditions
            .extend(sec_conds)
            .create_coin(clawback_ph, 0, None),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    println!("Fetching clawback public key...");
    let wallet_pk = get_coin_public_key(&sage, &clawback_address, 10000).await?;
    let message_coin = Coin::new(offer.security_coin.coin_id(), clawback_ph, 0);
    let p2 = StandardLayer::new(wallet_pk);
    let spend = p2.delegated_inner_spend(&mut ctx, Spend::new(NodePtr::NIL, NodePtr::NIL))?;
    println!(
        "Message coin ph: {}; computed ph: {}",
        message_coin.puzzle_hash,
        ctx.tree_hash(spend.puzzle)
    );

    println!("Signing spend...");

    let spend_bundle =
        SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig);

    print_spend_bundle_to_file(
        spend_bundle.coin_spends,
        spend_bundle.aggregated_signature,
        "sb.debug",
    );
    // println!("Submitting transaction...");
    // let client = get_coinset_client(testnet11);
    // let resp = client.push_tx(spend_bundle).await?;

    // println!("Transaction submitted; status='{}'", resp.status);

    // wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    // println!("Confirmed!");

    Ok(())
}

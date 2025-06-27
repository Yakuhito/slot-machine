use chia::protocol::{Coin, SpendBundle};
use chia_puzzle_types::Memos;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Offer, Spend, SpendContext, StandardLayer},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvmr::NodePtr;

use crate::{
    assets_xch_only, find_commitment_slots, find_reward_slot, get_coin_public_key,
    get_coinset_client, get_constants, hex_string_to_bytes32, hex_string_to_signature, new_sk,
    no_assets, parse_amount, parse_one_sided_offer, spend_security_coin, spend_to_coin_spend,
    sync_distributor, wait_for_coin, yes_no_prompt, CliError, Db,
    RewardDistributorWithdrawIncentivesAction, SageClient,
};

pub async fn reward_distributor_clawback_rewards(
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
    let clawback_ph = Address::decode(&clawback_address)?.puzzle_hash;

    let commitment_slot = find_commitment_slots(
        &mut ctx,
        &client,
        distributor.info.constants,
        clawback_ph,
        epoch_start,
        reward_amount,
    )
    .await?
    .into_iter()
    .next()
    .ok_or(CliError::SlotNotFound("Commitment"))?;
    let reward_slot = find_reward_slot(
        &mut ctx,
        &client,
        distributor.info.constants,
        commitment_slot.info.value.epoch_start,
    )
    .await?;

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

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let (send_message_conds, returned_amount) = distributor
        .new_action::<RewardDistributorWithdrawIncentivesAction>()
        .spend(&mut ctx, &mut distributor, commitment_slot, reward_slot)?;
    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

    println!("Returned amount: {} CAT mojos", returned_amount);

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer
            .security_base_conditions
            .create_coin(clawback_ph, 0, Memos::None),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    println!("Fetching clawback public key...");
    let wallet_pk = get_coin_public_key(&sage, &clawback_address, 10000).await?;
    let message_coin = Coin::new(offer.security_coin.coin_id(), clawback_ph, 0);
    let p2 = StandardLayer::new(wallet_pk);
    let inner_spend = Spend::new(ctx.alloc(&clvm_quote!(send_message_conds))?, NodePtr::NIL);
    let spend = p2.delegated_inner_spend(&mut ctx, inner_spend)?;

    if ctx.tree_hash(spend.puzzle) != clawback_ph.into() {
        return Err(CliError::Custom(
            "Clawback puzzle hash does not match - address is using custom puzzle :(".to_string(),
        ));
    }

    println!("Signing spend...");
    let resp = sage
        .sign_coin_spends(
            vec![spend_to_coin_spend(&mut ctx, message_coin, spend)?],
            false,
            true,
        )
        .await?;
    ctx.spend(message_coin, spend)?;

    let message_sig = hex_string_to_signature(&resp.spend_bundle.aggregated_signature)?;

    let spend_bundle = SpendBundle::new(
        ctx.take(),
        offer.aggregated_signature + &security_coin_sig + &message_sig,
    );

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

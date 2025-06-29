use chia::protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, CatSpend, Offer, Spend, SpendContext},
    types::{puzzles::SettlementPayment, Conditions},
};
use clvmr::NodePtr;

use crate::{
    assets_xch_and_cat, create_security_coin, get_coinset_client, get_constants,
    get_last_onchain_timestamp, hex_string_to_bytes32, no_assets, parse_amount,
    spend_security_coin, sync_distributor, wait_for_coin, yes_no_prompt, CliError, Db,
    RewardDistributorAddIncentivesAction, RewardDistributorSyncAction, SageClient,
};

pub async fn reward_distributor_add_rewards(
    launcher_id_str: String,
    reward_amount_str: String,
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

    let latest_timestamp = get_last_onchain_timestamp(&client).await?;
    if latest_timestamp > distributor.info.state.round_time_info.epoch_end {
        return Err(CliError::Custom(
            "The current epoch has already ended - start a new epoch first".to_string(),
        ));
    }

    let also_sync = distributor.info.state.round_time_info.last_update + 180 < latest_timestamp;
    if also_sync {
        println!(
            "Will also sync the distributor to timestamp {}",
            latest_timestamp
        );
    }

    println!("A one-sided offer will be created. It will contain:");
    println!(
        "  {} reward CATs ({} CAT mojos) to add to the rewards of the *current* epoch",
        reward_amount_str, reward_amount
    );
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_cat(
                1,
                hex::encode(distributor.info.constants.reserve_asset_id),
                reward_amount,
            ),
            fee,
            None,
            None,
            false,
        )
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let mut sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    sec_conds = sec_conds.extend(
        distributor
            .new_action::<RewardDistributorAddIncentivesAction>()
            .spend(&mut ctx, &mut distributor, reward_amount)?,
    );

    let settlement_cat = offer
        .offered_coins()
        .cats
        .get(&distributor.info.constants.reserve_asset_id)
        .ok_or(CliError::Custom(
            "Reward CAT not found in offer".to_string(),
        ))?[0];
    let offer_puzzle = ctx.alloc_mod::<SettlementPayment>()?;

    let _new_distributor = distributor.finish_spend(
        &mut ctx,
        vec![CatSpend {
            cat: settlement_cat,
            spend: Spend::new(offer_puzzle, NodePtr::NIL),
            hidden: false,
        }],
    )?;

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

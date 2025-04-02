use chia::protocol::{Bytes32, SpendBundle};
use chia_wallet_sdk::{decode_address, ChiaRpcClient, SpendContext};
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, hex_string_to_bytes32, parse_amount, sync_distributor, wait_for_coin,
    yes_no_prompt, CliError, Db, SageClient,
};

pub async fn dig_commit_rewards(
    launcher_id_str: String,
    reward_amount_str: String,
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
    let distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

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

    // todo: one-sided offer

    println!(
        "Reward distributor launcher id (SAVE THIS): {}",
        hex::encode(reward_distributor.info.constants.launcher_id)
    );

    let db = Db::new(false).await?;
    db.save_reward_distributor_configuration(
        &mut ctx.allocator,
        reward_distributor.info.constants.launcher_id,
        reward_distributor.info.constants,
    )
    .await?;

    let spend_bundle = SpendBundle::new(ctx.take(), sig);

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

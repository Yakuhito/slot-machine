use chia::protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, Offer, SpendContext},
    utils::Address,
};

use crate::{
    assets_xch_and_cat, get_coinset_client, get_constants, hex_string_to_bytes32,
    launch_dig_reward_distributor, no_assets, parse_amount, wait_for_coin, yes_no_prompt, CliError,
    Db, RewardDistributorConstants, RewardDistributorType, SageClient,
};

#[allow(clippy::too_many_arguments)]
pub async fn reward_distributor_launch(
    manager_launcher_id_str: Option<String>,
    collection_did_str: Option<String>,
    fee_payout_address_str: String,
    first_epoch_start_timestamp: u64,
    epoch_seconds: u64,
    max_seconds_offset: u64,
    payout_threshold_str: String,
    fee_bps: u64,
    withdrawal_share_bps: u64,
    reserve_asset_id_str: String,
    comment_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let (manager_or_did_launcher_id, distributor_type) =
        if let Some(manager_launcher_id_str) = manager_launcher_id_str {
            (
                hex_string_to_bytes32(&manager_launcher_id_str)?,
                RewardDistributorType::Manager,
            )
        } else if let Some(collection_did_str) = collection_did_str {
            (
                Address::decode(&collection_did_str)?.puzzle_hash,
                RewardDistributorType::Nft,
            )
        } else {
            return Err(CliError::Custom(
                "Either manager or collection DID launcher ID must be provided".to_string(),
            ));
        };
    let fee_payout_puzzle_hash = Address::decode(&fee_payout_address_str)?.puzzle_hash;
    let reserve_asset_id = hex_string_to_bytes32(&reserve_asset_id_str)?;
    let fee = parse_amount(&fee_str, false)?;
    let payout_threshold = parse_amount(&payout_threshold_str, true)?;
    if fee_bps > 2500 || withdrawal_share_bps < 5000 {
        return Err(CliError::Custom("really? that big of a fee?".to_string()));
    }

    println!("A one-sided offer will be needed for launch. It will contain:");
    println!("  1 mojo to create the distributor");
    println!("  1 reward CATs to create the reserve");
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    println!("Before continuing, please confirm the parameters above.");
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
    let derivation_resp = sage.get_derivations(false, 0, 1).await?;
    let user_address = &derivation_resp.derivations[0].address;
    let user_puzzle_hash = Address::decode(user_address)?.puzzle_hash;
    println!(
        "CAT will be returned to the active wallet (address: {})",
        user_address
    );

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_cat(1, hex::encode(reserve_asset_id), 1),
            fee,
            None,
            None,
            false,
        )
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let mut ctx = SpendContext::new();

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (sig, _sk, reward_distributor, _slot, _change_cat) = launch_dig_reward_distributor(
        &mut ctx,
        &offer,
        first_epoch_start_timestamp,
        user_puzzle_hash,
        RewardDistributorConstants::without_launcher_id(
            distributor_type,
            manager_or_did_launcher_id,
            fee_payout_puzzle_hash,
            epoch_seconds,
            max_seconds_offset,
            payout_threshold,
            fee_bps,
            withdrawal_share_bps,
            reserve_asset_id,
        ),
        get_constants(testnet11),
        &comment_str,
    )
    .map_err(CliError::Driver)?;

    println!(
        "Reward distributor launcher id (SAVE THIS): {}",
        hex::encode(reward_distributor.info.constants.launcher_id)
    );

    let db = Db::new(false).await?;
    db.save_reward_distributor_configuration(
        &mut ctx,
        reward_distributor.info.constants.launcher_id,
        reward_distributor.info.constants,
    )
    .await?;

    let spend_bundle = SpendBundle::new(ctx.take(), sig);

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, reward_distributor.coin.parent_coin_info, true).await?;
    println!("Confirmed!");

    Ok(())
}

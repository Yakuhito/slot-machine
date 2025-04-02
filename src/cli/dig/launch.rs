use chia::protocol::{Bytes32, SpendBundle};
use chia_wallet_sdk::{decode_address, ChiaRpcClient, Offer, SpendContext};
use sage_api::{Amount, Assets, CatAmount, GetDerivations, MakeOffer};

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, launch_dig_reward_distributor,
    parse_amount, wait_for_coin, yes_no_prompt, CliError, Db, DigRewardDistributorConstants,
    SageClient,
};

#[allow(clippy::too_many_arguments)]
pub async fn dig_launch(
    validator_launcher_id_str: String,
    validator_payout_address_str: String,
    first_epoch_start_height: u64,
    epoch_seconds: u64,
    max_seconds_offset: u64,
    payout_threshold_str: String,
    validator_fee_bps: u64,
    withdrawal_share_bps: u64,
    reserve_asset_id_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let validator_launcher_id = hex_string_to_bytes32(&validator_launcher_id_str)?;
    let validator_payout_puzzle_hash =
        Bytes32::new(decode_address(&validator_payout_address_str)?.0);
    let reserve_asset_id = hex_string_to_bytes32(&reserve_asset_id_str)?;
    let fee = parse_amount(&fee_str, false)?;
    let payout_threshold = parse_amount(&payout_threshold_str, true)?;
    if validator_fee_bps > 2500 || withdrawal_share_bps < 5000 {
        return Err(CliError::Custom("really? that big of a fee?".to_string()));
    }

    println!("A one-sided offer will be needed for launch. It will contain:");
    println!("  1 mojo to create the distributor");
    println!("  1 reward CATs to create the reserve");
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    println!("Before continuing, please confirm the parameters above.");
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
    let derivation_resp = sage
        .get_derivations(GetDerivations {
            hardened: false,
            offset: 0,
            limit: 1,
        })
        .await?;
    let user_address = &derivation_resp.derivations[0].address;
    let user_puzzle_hash = Bytes32::new(decode_address(user_address)?.0);
    println!(
        "CAT will be returned to the active wallet (address: {})",
        user_address
    );

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
                    asset_id: hex::encode(reserve_asset_id),
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

    let mut ctx = SpendContext::new();

    let (sig, _sk, reward_distributor, _slot) = launch_dig_reward_distributor(
        &mut ctx,
        Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?,
        first_epoch_start_height,
        user_puzzle_hash,
        DigRewardDistributorConstants::without_launcher_id(
            validator_launcher_id,
            validator_payout_puzzle_hash,
            epoch_seconds,
            max_seconds_offset,
            payout_threshold,
            validator_fee_bps,
            withdrawal_share_bps,
            reserve_asset_id,
        ),
        get_constants(testnet11),
    )
    .map_err(CliError::Driver)?;

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

    wait_for_coin(&client, reward_distributor.coin.parent_coin_info, true).await?;
    println!("Confirmed!");

    Ok(())
}

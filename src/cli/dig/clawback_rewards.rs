use chia::protocol::{Bytes32, SpendBundle};
use chia_wallet_sdk::{decode_address, CatSpend, ChiaRpcClient, Offer, Spend, SpendContext};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, new_sk, parse_amount,
    parse_one_sided_offer, spend_security_coin, sync_distributor, wait_for_coin, yes_no_prompt,
    CliError, Db, DigCommitIncentivesAction, DigRewardSlotValue, DigSlotNonce, SageClient,
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

    println!("A one-sided offer will be created. It will contain:");
    println!("  1 mojo",);
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

    Ok(())
}

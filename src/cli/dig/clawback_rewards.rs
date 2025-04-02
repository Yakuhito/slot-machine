use chia::protocol::Bytes32;
use chia_wallet_sdk::{decode_address, SpendContext};
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, hex_string_to_bytes32, parse_amount, sync_distributor, yes_no_prompt,
    CliError, Db, DigRewardSlotValue, DigSlotNonce, SageClient,
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
    let value_hashes = db
        .get_dig_indexed_slot_values_by_puzzle_hash(clawback_ph)
        .await?;
    let mut slot = None;
    for value_hash in value_hashes {
        let reward_slot = db
            .get_slot::<DigRewardSlotValue>(
                &mut ctx.allocator,
                launcher_id,
                DigSlotNonce::COMMITMENT.to_u64(),
                reward_slot_value_hash,
                0,
            )
            .await?
            .ok_or(CliError::Custom(
                "Reward slot could not be found".to_string(),
            ))?;
    }

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

use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, SpendBundle},
};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Offer, SpendContext},
    types::Conditions,
    utils::Address,
};

use sage_api::{Amount, Assets, GetDerivations, MakeOffer};

use crate::{
    get_coinset_client, get_constants, get_last_onchain_timestamp, get_prefix,
    hex_string_to_bytes32, new_sk, parse_amount, parse_one_sided_offer, spend_security_coin,
    sync_distributor, wait_for_coin, yes_no_prompt, CliError, Db, NonceWrapperArgs,
    RewardDistributorEntrySlotValue, RewardDistributorSlotNonce, RewardDistributorStakeAction,
    RewardDistributorStakeActionArgs, RewardDistributorSyncAction, RewardDistributorUnstakeAction,
    SageClient, Slot, NONCE_WRAPPER_PUZZLE_HASH,
};

pub async fn reward_distributor_unstake(
    launcher_id_str: String,
    custody_puzzle_hash_str: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
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

    let sage = SageClient::new()?;
    let custody_puzzle_hash = if let Some(custody_puzzle_hash_str) = custody_puzzle_hash_str {
        hex_string_to_bytes32(&custody_puzzle_hash_str)?
    } else {
        Address::decode(
            &sage
                .get_derivations(GetDerivations {
                    hardened: false,
                    offset: 0,
                    limit: 1,
                })
                .await?
                .derivations[0]
                .address,
        )?
        .puzzle_hash
    };

    println!(
        "Using the following address as custody: {}",
        Address::new(custody_puzzle_hash, get_prefix(testnet11)).encode()?
    );

    println!("A one-sided offer will be created. It will contain:");
    println!("  1 mojo");
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    println!("Getting entry slot...");
    let entry_slot_value_hashes = db
        .get_dig_indexed_slot_values_by_puzzle_hash(
            custody_puzzle_hash,
            RewardDistributorSlotNonce::ENTRY.to_u64(),
        )
        .await?;
    if entry_slot_value_hashes.is_empty() {
        return Err(CliError::Custom(
            "No entry slot found - you may be using the wrong custody address/puzzle hash"
                .to_string(),
        ));
    }

    let entry_slot: Slot<RewardDistributorEntrySlotValue> = db
        .get_slot(
            &mut ctx,
            launcher_id,
            RewardDistributorSlotNonce::ENTRY.to_u64(),
            entry_slot_value_hashes[0],
            0,
        )
        .await?
        .unwrap();

    println!("Fetching locked NFT...");
    let locked_nft_hint: Bytes32 = CurriedProgram {
        program: NONCE_WRAPPER_PUZZLE_HASH,
        args: NonceWrapperArgs::<TreeHash, TreeHash> {
            nonce: custody_puzzle_hash.into(),
            inner_puzzle: RewardDistributorStakeActionArgs::my_p2_puzzle_hash(launcher_id).into(),
        },
    }
    .tree_hash()
    .into();

    let possible_locked_nft_coins = client
        .get_coin_records_by_hint(locked_nft_hint, None, None, Some(false))
        .await?
        .coin_records
        .unwrap();
    todo_here_future_yak

    yes_no_prompt("Proceed?")?;

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

    let sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    // accept offer
    let (conds, last_payment_amount) = distributor
        .new_action::<RewardDistributorUnstakeAction>()
        .spend(&mut ctx, &mut distributor, entry_slot, locked_nft)?;
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    println!(
        "Last reward payment amount: {:.3} CATs",
        last_payment_amount as f64 / 1000.0
    );

    let sec_conds = sec_conds.extend(conds).reserve_fee(1);

    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

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

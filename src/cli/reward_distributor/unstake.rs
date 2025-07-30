use chia::{
    clvm_utils::TreeHash,
    protocol::{Bytes32, Coin, SpendBundle},
};
use chia_puzzle_types::{
    offer::{NotarizedPayment, Payment, SettlementPaymentsSolution},
    standard::StandardArgs,
    Memos,
};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        decode_offer, Nft, Offer, Puzzle, RewardDistributorStakeAction,
        RewardDistributorSyncAction, RewardDistributorUnstakeAction, Spend, SpendContext,
        SpendWithConditions, StandardLayer,
    },
    types::{
        puzzles::{NonceWrapperArgs, SettlementPayment},
        Conditions, Mod,
    },
    utils::Address,
};

use crate::{
    assets_xch_only, find_entry_slots, get_coinset_client, get_last_onchain_timestamp, get_prefix,
    hex_string_to_bytes32, hex_string_to_pubkey, hex_string_to_signature, no_assets, parse_amount,
    prompt_for_value, spend_to_coin_spend, sync_distributor, wait_for_coin, yes_no_prompt,
    CliError, Db, SageClient,
};

pub async fn reward_distributor_unstake(
    launcher_id_str: String,
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
    let custody_info = sage.get_derivations(false, 0, 1).await?.derivations[0].clone();
    let custody_puzzle_hash = Address::decode(&custody_info.address)?.puzzle_hash;
    let custody_public_key = hex_string_to_pubkey(&custody_info.public_key)?;
    if StandardArgs::curry_tree_hash(custody_public_key) != custody_puzzle_hash.into() {
        return Err(CliError::Custom(
            "Custody puzzle hash does not match the retrieved public key".to_string(),
        ));
    }

    println!(
        "Using the following address as custody: {}",
        Address::new(custody_puzzle_hash, get_prefix(testnet11)).encode()?
    );

    println!("Getting entry slot...");
    let entry_slot = find_entry_slots(
        &mut ctx,
        &client,
        distributor.info.constants,
        custody_puzzle_hash,
        None,
        None,
    )
    .await?
    .into_iter()
    .next()
    .ok_or(CliError::SlotNotFound("Entry"))?;

    println!("Fetching locked NFT...");
    let locked_nft_hint: Bytes32 = NonceWrapperArgs::<Bytes32, TreeHash> {
        nonce: custody_puzzle_hash,
        inner_puzzle: RewardDistributorStakeAction::my_p2_puzzle_hash(launcher_id).into(),
    }
    .curry_tree_hash()
    .into();

    let possible_locked_nft_coins = client
        .get_coin_records_by_hint(locked_nft_hint, None, None, Some(false))
        .await?
        .coin_records
        .unwrap();
    let mut locked_nfts = Vec::new();

    for coin_record in possible_locked_nft_coins {
        let parent_coin_spend = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotFound(coin_record.coin.parent_coin_info))?;

        let parent_puzzle = ctx.alloc(&parent_coin_spend.puzzle_reveal)?;
        let parent_puzzle = Puzzle::parse(&ctx, parent_puzzle);
        let parent_solution = ctx.alloc(&parent_coin_spend.solution)?;

        if let Ok(Some(nft)) = Nft::parse_child(
            &mut ctx,
            parent_coin_spend.coin,
            parent_puzzle,
            parent_solution,
        ) {
            if nft.info.p2_puzzle_hash == locked_nft_hint {
                locked_nfts.push(nft);
            }
        }
    }

    if locked_nfts.is_empty() {
        return Err(CliError::Custom(
            "No locked NFTs found - you may be using the wrong custody address/puzzle hash"
                .to_string(),
        ));
    }

    let mut locked_nft = locked_nfts[0];
    if locked_nfts.len() > 1 {
        println!("Found multiple NFTs:");
        for (i, nft) in locked_nfts.iter().enumerate() {
            println!(
                "  - {}: {}",
                i,
                Address::new(nft.info.launcher_id, "nft".to_string()).encode()?
            );
        }

        let nft_index = prompt_for_value("NFT index to unstake: ")?;
        let nft_index = nft_index.parse::<usize>()?;

        if nft_index >= locked_nfts.len() {
            return Err(CliError::Custom("Invalid NFT index".to_string()));
        }
        locked_nft = locked_nfts[nft_index];
    }

    println!(
        "Unstaking NFT: {}",
        Address::new(locked_nft.info.launcher_id, "nft".to_string()).encode()?
    );

    println!("A one-sided offer will be created. It will contain:");
    println!("  1 mojo");
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let xch_settlement_coin = offer.offered_coins().xch[0];
    let security_coin_puzzle_hash: Bytes32 =
        StandardArgs::curry_tree_hash(custody_public_key).into();
    let notarized_payment = NotarizedPayment {
        nonce: xch_settlement_coin.coin_id(),
        payments: vec![Payment::new(
            security_coin_puzzle_hash,
            xch_settlement_coin.amount,
            Memos::None,
        )],
    };
    let settlement_puzzle = ctx.alloc_mod::<SettlementPayment>()?;
    let settlement_solution = ctx.alloc(&SettlementPaymentsSolution {
        notarized_payments: vec![notarized_payment],
    })?;
    ctx.spend(
        xch_settlement_coin,
        Spend::new(settlement_puzzle, settlement_solution),
    )?;

    let security_coin = Coin::new(
        xch_settlement_coin.coin_id(),
        security_coin_puzzle_hash,
        xch_settlement_coin.amount,
    );

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

    println!(
        "Last reward payment amount: {:.3} CATs",
        last_payment_amount as f64 / 1000.0
    );

    let sec_conds = sec_conds.extend(conds).reserve_fee(1);

    let (_new_distributor, pending_sig) = distributor.finish_spend(&mut ctx, vec![])?;

    // security coin has custody puzzle!
    println!("Signing custody coin...");
    let security_coin_spend =
        StandardLayer::new(custody_public_key).spend_with_conditions(&mut ctx, sec_conds)?;
    ctx.spend(security_coin, security_coin_spend)?;

    let security_coin_sig = hex_string_to_signature(
        &sage
            .sign_coin_spends(
                vec![spend_to_coin_spend(
                    &mut ctx,
                    security_coin,
                    security_coin_spend,
                )?],
                false,
                true,
            )
            .await?
            .spend_bundle
            .aggregated_signature
            .replace("0x", ""),
    )?;

    let spend_bundle = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

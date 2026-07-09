use chia_protocol::{Bytes32, Coin, SpendBundle};
use chia_puzzle_types::{
    offer::{NotarizedPayment, Payment, SettlementPaymentsSolution},
    standard::StandardArgs,
    Memos,
};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        decode_offer, Cat, Nft, Offer, RewardDistributorSyncAction, RewardDistributorType,
        RewardDistributorUnstakeAction, Spend, SpendContext, SpendWithConditions, StandardLayer,
    },
    types::{puzzles::SettlementPayment, Conditions},
    utils::Address,
};

use crate::{
    assets_xch_only, confirm_pushed_transaction, ensure_epoch_open, find_entry_slots,
    find_locked_cat, find_locked_nfts, format_cat_mojos, get_coinset_client,
    get_last_onchain_timestamp, get_prefix, hex_string_to_bytes32, hex_string_to_signature,
    no_assets, parse_amount, prompt_for_value, resolve_custody, spend_to_coin_spend,
    sync_distributor, yes_no_prompt, CliError, Db, SageClient,
};

enum LockedStake {
    Nft(Nft, u64),
    Cat(Cat),
}

pub async fn reward_distributor_unstake(
    launcher_id_str: String,
    custody_address: Option<String>,
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

    let distributor_type = distributor.info.constants.reward_distributor_type;
    match distributor_type {
        RewardDistributorType::NftCollection { .. }
        | RewardDistributorType::CuratedNft { .. }
        | RewardDistributorType::Cat { .. } => {}
        RewardDistributorType::Managed { .. } => {
            return Err(CliError::Custom(
                "Managed distributors use remove-entry / broadcast-entry-update, not unstake"
                    .to_string(),
            ));
        }
    }

    ensure_epoch_open(&client, &distributor).await?;

    let latest_timestamp = get_last_onchain_timestamp(&client).await?;
    let also_sync = distributor.info.state.round_time_info.last_update + 180 < latest_timestamp;
    if also_sync {
        println!(
            "Will also sync the distributor to timestamp {}",
            latest_timestamp
        );
    }

    let sage = SageClient::new()?;
    let custody = resolve_custody(&sage, custody_address).await?;
    println!(
        "Using the following address as custody: {}",
        Address::new(custody.puzzle_hash, get_prefix(testnet11)).encode()?
    );

    println!("Getting entry slot...");
    let entry_slot = find_entry_slots(
        &mut ctx,
        &client,
        distributor.info.constants,
        custody.puzzle_hash,
        None,
        None,
    )
    .await?
    .into_iter()
    .next()
    .ok_or(CliError::SlotNotFound("Entry"))?;

    let locked_stake = match distributor_type {
        RewardDistributorType::Cat { asset_id, .. } => {
            println!("Fetching locked CAT...");
            let locked_cat = find_locked_cat(
                &mut ctx,
                &client,
                launcher_id,
                custody.puzzle_hash,
                asset_id,
                entry_slot.info.value.shares,
            )
            .await?;
            println!("Unstaking CAT: {} mojos", locked_cat.coin.amount);
            LockedStake::Cat(locked_cat)
        }
        RewardDistributorType::NftCollection { .. } | RewardDistributorType::CuratedNft { .. } => {
            println!("Fetching locked NFT...");
            let locked_nfts = find_locked_nfts(
                &mut ctx,
                &client,
                launcher_id,
                custody.puzzle_hash,
                distributor_type,
                entry_slot.info.value.shares,
            )
            .await?;

            if locked_nfts.is_empty() {
                return Err(CliError::Custom(
                    "No locked NFTs found - you may be using the wrong custody address/puzzle hash"
                        .to_string(),
                ));
            }

            let mut locked_nft = locked_nfts[0].0;
            let mut locked_nft_share = locked_nfts[0].1;
            if locked_nfts.len() > 1 {
                println!("Found multiple NFTs:");
                for (i, (nft, shares)) in locked_nfts.iter().enumerate() {
                    println!(
                        "  - {}: {} ({} shares)",
                        i,
                        Address::new(nft.info.launcher_id, "nft".to_string()).encode()?,
                        shares
                    );
                }

                let nft_index = prompt_for_value("NFT index to unstake: ")?;
                let nft_index = nft_index.parse::<usize>()?;

                if nft_index >= locked_nfts.len() {
                    return Err(CliError::Custom("Invalid NFT index".to_string()));
                }
                locked_nft = locked_nfts[nft_index].0;
                locked_nft_share = locked_nfts[nft_index].1;
            }

            println!(
                "Unstaking NFT: {} ({} shares)",
                Address::new(locked_nft.info.launcher_id, "nft".to_string()).encode()?,
                locked_nft_share
            );
            LockedStake::Nft(locked_nft, locked_nft_share)
        }
        RewardDistributorType::Managed { .. } => unreachable!(),
    };

    println!("A one-sided offer will be created. It will contain:");
    println!("  - 1 mojo");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let xch_settlement_coin = offer.offered_coins().xch[0];
    let security_coin_puzzle_hash: Bytes32 =
        StandardArgs::curry_tree_hash(custody.public_key).into();
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

    let (conds, last_payment_amount) = match locked_stake {
        LockedStake::Nft(locked_nft, locked_nft_share) => distributor
            .new_action::<RewardDistributorUnstakeAction>()
            .spend_for_locked_nfts(
                &mut ctx,
                &mut distributor,
                entry_slot,
                std::slice::from_ref(&locked_nft),
                std::slice::from_ref(&locked_nft_share),
            )?,
        LockedStake::Cat(locked_cat) => distributor
            .new_action::<RewardDistributorUnstakeAction>()
            .spend_for_locked_cats(&mut ctx, &mut distributor, entry_slot, locked_cat)?,
    };

    println!(
        "Last reward payment amount: {}",
        format_cat_mojos(last_payment_amount)
    );

    let sec_conds = sec_conds.extend(conds).reserve_fee(1);
    let (_new_distributor, pending_sig) = distributor.finish_spend(&mut ctx, vec![])?;

    println!("Signing custody coin...");
    let security_coin_spend =
        StandardLayer::new(custody.public_key).spend_with_conditions(&mut ctx, sec_conds)?;
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
    let resp = client.push_tx(spend_bundle).await?;

    if confirm_pushed_transaction(&client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

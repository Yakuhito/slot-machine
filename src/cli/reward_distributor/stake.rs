use chia_bls::Signature;
use chia_protocol::{Bytes32, Coin, SpendBundle};
use chia_puzzle_types::{LineageProof, Memos};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        create_security_coin, decode_offer, spend_security_coin,
        spend_settlement_cats_with_payments, spend_settlement_nft_with_payment, HashedPtr, Layer,
        Offer, Puzzle, RewardDistributor, RewardDistributorStakeAction,
        RewardDistributorSyncAction, RewardDistributorType, SingletonLayer, Slot, Spend,
        SpendContext, StandardLayer,
    },
    types::{
        puzzles::{
            CompactLineageProof, IntermediaryCoinProof, NftLauncherProof,
            RewardDistributorEntrySlotValue,
        },
        Conditions,
    },
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvmr::NodePtr;

use crate::{
    assets_xch_and_cat, assets_xch_and_nft, confirm_pushed_transaction, curated_datastore_fields,
    delegated_puzzles, ensure_epoch_open, find_entry_slots, get_coin_public_key,
    get_coinset_client, get_constants, get_last_onchain_timestamp, get_prefix,
    hex_string_to_bytes32, hex_string_to_signature, load_csv_matching_root, merkle_proof_for_nft,
    no_assets, parse_amount, resolve_custody, spend_datastore_oracle, spend_to_coin_spend,
    sync_datastore, sync_distributor, yes_no_prompt, CliError, CustodyInfo, Db, SageClient,
};

pub async fn reward_distributor_stake(
    launcher_id_str: String,
    nft_id_str: Option<String>,
    stake_amount_str: Option<String>,
    csv_path: Option<String>,
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
    let distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let distributor_type = distributor.info.constants.reward_distributor_type;
    if matches!(distributor_type, RewardDistributorType::Managed { .. }) {
        return Err(CliError::Custom(
            "Managed distributors use sign-entry-update / broadcast-entry-update, not stake"
                .to_string(),
        ));
    }

    let latest_timestamp = get_last_onchain_timestamp(&client).await?;
    ensure_epoch_open(&distributor, latest_timestamp)?;
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

    let existing_slot = find_entry_slots(
        &mut ctx,
        &client,
        distributor.info.constants,
        custody.puzzle_hash,
        None,
        None,
    )
    .await?
    .into_iter()
    .next();
    if existing_slot.is_some() {
        println!("Found existing entry slot; shares will be consolidated.");
    }

    match distributor_type {
        RewardDistributorType::NftCollection {
            collection_did_launcher_id,
        } => {
            let nft_id_str = nft_id_str.ok_or(CliError::Custom(
                "NFT id (--nft) is required for NFT collection distributors".to_string(),
            ))?;
            stake_nft_collection(
                &client,
                &sage,
                &mut ctx,
                distributor,
                collection_did_launcher_id,
                &nft_id_str,
                custody.puzzle_hash,
                existing_slot,
                also_sync,
                latest_timestamp,
                fee,
                &fee_str,
                testnet11,
            )
            .await
        }
        RewardDistributorType::CuratedNft {
            store_launcher_id, ..
        } => {
            let nft_id_str = nft_id_str.ok_or(CliError::Custom(
                "NFT id (--nft) is required for curated NFT distributors".to_string(),
            ))?;
            let csv_path = csv_path.ok_or(CliError::Custom(
                "Whitelist CSV (--csv) is required for curated NFT distributors".to_string(),
            ))?;
            stake_curated_nft(
                &client,
                &sage,
                &mut ctx,
                distributor,
                store_launcher_id,
                &nft_id_str,
                &csv_path,
                custody,
                existing_slot,
                also_sync,
                latest_timestamp,
                fee,
                &fee_str,
                testnet11,
            )
            .await
        }
        RewardDistributorType::Cat { asset_id, .. } => {
            let stake_amount_str = stake_amount_str.ok_or(CliError::Custom(
                "Stake amount (--stake-amount) is required for CAT distributors".to_string(),
            ))?;
            let stake_amount = parse_amount(&stake_amount_str, true)?;
            stake_cat(
                &client,
                &sage,
                &mut ctx,
                distributor,
                asset_id,
                stake_amount,
                &stake_amount_str,
                custody.puzzle_hash,
                existing_slot,
                also_sync,
                latest_timestamp,
                fee,
                &fee_str,
                testnet11,
            )
            .await
        }
        RewardDistributorType::Managed { .. } => unreachable!(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn stake_nft_collection(
    client: &CoinsetClient,
    sage: &SageClient,
    ctx: &mut SpendContext,
    distributor: RewardDistributor,
    collection_did_launcher_id: Bytes32,
    nft_id_str: &str,
    custody_puzzle_hash: Bytes32,
    existing_slot: Option<Slot<RewardDistributorEntrySlotValue>>,
    also_sync: bool,
    latest_timestamp: u64,
    fee: u64,
    fee_str: &str,
    testnet11: bool,
) -> Result<(), CliError> {
    let nft_launcher_id = Address::decode(nft_id_str)?.puzzle_hash;

    print!("Generating NFT launcher proof...");
    let mut intemrediary_coins = Vec::new();
    let did_proof;
    let mut latest_coin_id = nft_launcher_id;
    loop {
        let coin_record = client
            .get_coin_record_by_name(latest_coin_id)
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(latest_coin_id))?;

        if coin_record.coin.amount % 2 == 1 {
            let coin_spend = client
                .get_puzzle_and_solution(latest_coin_id, Some(coin_record.spent_block_index))
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(latest_coin_id))?;
            let puzzle = ctx.alloc(&coin_spend.puzzle_reveal)?;
            let puzzle = Puzzle::parse(ctx, puzzle);

            if let Ok(Some(layer)) = SingletonLayer::<HashedPtr>::parse_puzzle(ctx, puzzle) {
                did_proof = LineageProof {
                    parent_parent_coin_info: coin_record.coin.parent_coin_info,
                    parent_inner_puzzle_hash: layer.inner_puzzle.tree_hash().into(),
                    parent_amount: coin_record.coin.amount,
                };
                if layer.launcher_id != collection_did_launcher_id {
                    println!("FAILED");
                    return Err(CliError::Custom(
                        "The DID launcher ID does not match the reward distributor's configuration - does the NFT belong to the right collection?"
                            .to_string(),
                    ));
                }
                break;
            }
        }

        latest_coin_id = coin_record.coin.parent_coin_info;
        intemrediary_coins.push(IntermediaryCoinProof {
            full_puzzle_hash: coin_record.coin.puzzle_hash,
            amount: coin_record.coin.amount,
        });
    }

    let nft_launcher_proof = NftLauncherProof {
        did_proof: CompactLineageProof::from(did_proof),
        intermediary_coin_proofs: intemrediary_coins.into_iter().rev().collect(),
    };
    println!(
        "done ({} intermediary coins).",
        nft_launcher_proof
            .intermediary_coin_proofs
            .len()
            .saturating_sub(1)
    );

    println!("A one-sided offer will be created. It will contain:");
    println!("  - the NFT to be deposited");
    println!("  - 1 mojo");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_nft(1, nft_id_str.to_string()),
            fee,
            None,
            None,
            false,
        )
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    submit_nft_collection_stake(
        client,
        sage,
        ctx,
        distributor,
        &offer_resp.offer,
        nft_launcher_id,
        nft_launcher_proof,
        custody_puzzle_hash,
        existing_slot,
        also_sync,
        latest_timestamp,
        testnet11,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn stake_curated_nft(
    client: &CoinsetClient,
    sage: &SageClient,
    ctx: &mut SpendContext,
    mut distributor: RewardDistributor,
    store_launcher_id: Bytes32,
    nft_id_str: &str,
    csv_path: &str,
    custody: CustodyInfo,
    existing_slot: Option<Slot<RewardDistributorEntrySlotValue>>,
    also_sync: bool,
    latest_timestamp: u64,
    fee: u64,
    fee_str: &str,
    testnet11: bool,
) -> Result<(), CliError> {
    let nft_launcher_id = Address::decode(nft_id_str)?.puzzle_hash;

    println!("Syncing datastore...");
    let datastore = sync_datastore(client, ctx, store_launcher_id, &delegated_puzzles()).await?;
    let records = load_csv_matching_root(csv_path, datastore.info.metadata.root_hash)?;
    let merkle_entry = merkle_proof_for_nft(&records, nft_launcher_id)?;

    let dl_fields = curated_datastore_fields(&datastore, ctx)?;

    println!(
        "Staking NFT with whitelist weight {} shares",
        merkle_entry.weight
    );
    println!("A one-sided offer will be created. It will contain:");
    println!("  - the NFT to be deposited");
    println!("  - 1 mojo");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_nft(1, nft_id_str.to_string()),
            fee,
            None,
            None,
            false,
        )
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(ctx, offer.offered_coins().xch[0])?;

    let dl_spend = spend_datastore_oracle(ctx, datastore, &delegated_puzzles())?;
    ctx.insert(dl_spend);

    let mut sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    let payout_puzzle_hash = existing_slot
        .as_ref()
        .map(|slot| slot.info.value.payout_puzzle_hash);
    let current_nft = offer
        .offered_coins()
        .nfts
        .get(&nft_launcher_id)
        .ok_or(CliError::Custom("NFT not found in offer".to_string()))?;

    let (conds, notarized_payments, _created_nfts) = distributor
        .new_action::<RewardDistributorStakeAction>()
        .spend_for_curated_nft_mode(
            ctx,
            &mut distributor,
            std::slice::from_ref(current_nft),
            std::slice::from_ref(&merkle_entry.weight),
            std::slice::from_ref(&merkle_entry.proof),
            custody.puzzle_hash,
            existing_slot,
            dl_fields.dl_root_hash,
            dl_fields.dl_metadata_rest_hash,
            dl_fields.dl_metadata_updater_hash_hash,
            dl_fields.dl_inner_puzzle_hash,
        )?;
    let (_new_nft, nft_assert) = spend_settlement_nft_with_payment(
        ctx,
        &offer,
        nft_launcher_id,
        notarized_payments[0].nonce,
        notarized_payments[0].payments[0].clone(),
    )?;

    let (_new_distributor, pending_sig) = distributor.finish_spend(ctx, vec![])?;
    sec_conds = sec_conds.extend(nft_assert).reserve_fee(1);

    let custody_sig = if let Some(custody_ph) = payout_puzzle_hash {
        let custody_coin = Coin::new(security_coin.coin_id(), custody_ph, 0);
        sec_conds = sec_conds
            .create_coin(custody_ph, 0, Memos::None)
            .assert_concurrent_spend(custody_coin.coin_id());

        let custody_pk = get_coin_public_key(
            sage,
            &Address::new(custody_ph, get_prefix(testnet11)).encode()?,
            10000,
        )
        .await?;
        let p2 = StandardLayer::new(custody_pk);
        let inner_spend = Spend::new(ctx.alloc(&clvm_quote!(conds))?, NodePtr::NIL);
        let spend = p2.delegated_inner_spend(ctx, inner_spend)?;

        if ctx.tree_hash(spend.puzzle) != custody_ph.into() {
            return Err(CliError::Custom(
                "Payout puzzle hash does not match - address is using custom puzzle :(".to_string(),
            ));
        }

        ctx.spend(custody_coin, spend)?;

        hex_string_to_signature(
            &sage
                .sign_coin_spends(
                    vec![spend_to_coin_spend(ctx, custody_coin, spend)?],
                    false,
                    true,
                )
                .await?
                .spend_bundle
                .aggregated_signature,
        )?
    } else {
        sec_conds = sec_conds.extend(conds);
        Signature::default()
    };

    let security_coin_sig = spend_security_coin(
        ctx,
        security_coin,
        sec_conds,
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let spend_bundle = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig + &custody_sig,
    ));

    println!("Submitting transaction...");
    let resp = client.push_tx(spend_bundle).await?;
    if confirm_pushed_transaction(client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn stake_cat(
    client: &CoinsetClient,
    sage: &SageClient,
    ctx: &mut SpendContext,
    mut distributor: RewardDistributor,
    asset_id: Bytes32,
    stake_amount: u64,
    stake_amount_str: &str,
    custody_puzzle_hash: Bytes32,
    existing_slot: Option<Slot<RewardDistributorEntrySlotValue>>,
    also_sync: bool,
    latest_timestamp: u64,
    fee: u64,
    fee_str: &str,
    testnet11: bool,
) -> Result<(), CliError> {
    println!("A one-sided offer will be created. It will contain:");
    println!("  - {} stakeable CAT mojos (shares)", stake_amount_str);
    println!("  - 1 mojo");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_cat(1, hex::encode(asset_id), stake_amount),
            fee,
            None,
            None,
            false,
        )
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(ctx, offer.offered_coins().xch[0])?;

    let offered_cat = offer
        .offered_coins()
        .cats
        .get(&asset_id)
        .and_then(|cats| cats.iter().find(|cat| cat.coin.amount == stake_amount))
        .copied()
        .ok_or(CliError::Custom(
            "Stakeable CAT with the requested amount not found in offer".to_string(),
        ))?;

    let mut sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    let payout_puzzle_hash = existing_slot
        .as_ref()
        .map(|slot| slot.info.value.payout_puzzle_hash);
    let (conds, notarized_payment, _locked_cat) = distributor
        .new_action::<RewardDistributorStakeAction>()
        .spend_for_cat_mode(
            ctx,
            &mut distributor,
            offered_cat,
            custody_puzzle_hash,
            existing_slot,
        )?;

    let (_cats, cat_assert) = spend_settlement_cats_with_payments(
        ctx,
        &offer,
        asset_id,
        notarized_payment.nonce,
        notarized_payment.payments,
    )?;

    let (_new_distributor, pending_sig) = distributor.finish_spend(ctx, vec![])?;
    sec_conds = sec_conds.extend(cat_assert).reserve_fee(1);

    // if consolidating a slot, we need 'conds' to be outputted by a custody coin
    let custody_sig = if let Some(custody_ph) = payout_puzzle_hash {
        let custody_coin = Coin::new(security_coin.coin_id(), custody_ph, 0);
        sec_conds = sec_conds
            .create_coin(custody_ph, 0, Memos::None)
            .assert_concurrent_spend(custody_coin.coin_id());

        let custody_pk = get_coin_public_key(
            sage,
            &Address::new(custody_ph, get_prefix(testnet11)).encode()?,
            10000,
        )
        .await?;
        let p2 = StandardLayer::new(custody_pk);
        let inner_spend = Spend::new(ctx.alloc(&clvm_quote!(conds))?, NodePtr::NIL);
        let spend = p2.delegated_inner_spend(ctx, inner_spend)?;

        if ctx.tree_hash(spend.puzzle) != custody_ph.into() {
            return Err(CliError::Custom(
                "Payout puzzle hash does not match - address is using custom puzzle :(".to_string(),
            ));
        }

        ctx.spend(custody_coin, spend)?;

        hex_string_to_signature(
            &sage
                .sign_coin_spends(
                    vec![spend_to_coin_spend(ctx, custody_coin, spend)?],
                    false,
                    true,
                )
                .await?
                .spend_bundle
                .aggregated_signature,
        )?
    } else {
        sec_conds = sec_conds.extend(conds);

        Signature::default()
    };

    let security_coin_sig = spend_security_coin(
        ctx,
        security_coin,
        sec_conds,
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let spend_bundle = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig + &custody_sig,
    ));

    println!("Submitting transaction...");
    let resp = client.push_tx(spend_bundle).await?;
    if confirm_pushed_transaction(client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn submit_nft_collection_stake(
    client: &CoinsetClient,
    sage: &SageClient,
    ctx: &mut SpendContext,
    mut distributor: RewardDistributor,
    offer_str: &str,
    nft_launcher_id: Bytes32,
    nft_launcher_proof: NftLauncherProof,
    custody_puzzle_hash: Bytes32,
    existing_slot: Option<Slot<RewardDistributorEntrySlotValue>>,
    also_sync: bool,
    latest_timestamp: u64,
    testnet11: bool,
) -> Result<(), CliError> {
    let offer = Offer::from_spend_bundle(ctx, &decode_offer(offer_str)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(ctx, offer.offered_coins().xch[0])?;

    let mut sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    let payout_puzzle_hash = existing_slot
        .as_ref()
        .map(|slot| slot.info.value.payout_puzzle_hash);
    let current_nft = offer
        .offered_coins()
        .nfts
        .get(&nft_launcher_id)
        .ok_or(CliError::Custom("NFT not found in offer".to_string()))?;

    let (conds, notarized_payments, _created_nfts) = distributor
        .new_action::<RewardDistributorStakeAction>()
        .spend_for_collection_nft_mode(
            ctx,
            &mut distributor,
            std::slice::from_ref(current_nft),
            std::slice::from_ref(&nft_launcher_proof),
            custody_puzzle_hash,
            existing_slot,
        )?;

    let (_new_nft, nft_assert) = spend_settlement_nft_with_payment(
        ctx,
        &offer,
        nft_launcher_id,
        notarized_payments[0].nonce,
        notarized_payments[0].payments[0].clone(),
    )?;

    let (_new_distributor, pending_sig) = distributor.finish_spend(ctx, vec![])?;
    sec_conds = sec_conds.extend(nft_assert).reserve_fee(1);

    let custody_sig = if let Some(custody_ph) = payout_puzzle_hash {
        let custody_coin = Coin::new(security_coin.coin_id(), custody_ph, 0);
        sec_conds = sec_conds
            .create_coin(custody_ph, 0, Memos::None)
            .assert_concurrent_spend(custody_coin.coin_id());

        let custody_pk = get_coin_public_key(
            sage,
            &Address::new(custody_ph, get_prefix(testnet11)).encode()?,
            10000,
        )
        .await?;
        let p2 = StandardLayer::new(custody_pk);
        let inner_spend = Spend::new(ctx.alloc(&clvm_quote!(conds))?, NodePtr::NIL);
        let spend = p2.delegated_inner_spend(ctx, inner_spend)?;

        if ctx.tree_hash(spend.puzzle) != custody_ph.into() {
            return Err(CliError::Custom(
                "Payout puzzle hash does not match - address is using custom puzzle :(".to_string(),
            ));
        }

        ctx.spend(custody_coin, spend)?;

        hex_string_to_signature(
            &sage
                .sign_coin_spends(
                    vec![spend_to_coin_spend(ctx, custody_coin, spend)?],
                    false,
                    true,
                )
                .await?
                .spend_bundle
                .aggregated_signature,
        )?
    } else {
        sec_conds = sec_conds.extend(conds);
        Signature::default()
    };

    let security_coin_sig = spend_security_coin(
        ctx,
        security_coin,
        sec_conds,
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let spend_bundle = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig + &custody_sig,
    ));

    println!("Submitting transaction...");
    let resp = client.push_tx(spend_bundle).await?;
    if confirm_pushed_transaction(client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

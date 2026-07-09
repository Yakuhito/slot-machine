use chia_protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, Offer,
        RewardDistributorRefreshAction, RewardDistributorSyncAction, RewardDistributorType,
        SpendContext,
    },
    types::Conditions,
    utils::Address,
};

use crate::{
    assets_xch_only, confirm_pushed_transaction, curated_datastore_fields, delegated_puzzles,
    ensure_epoch_open, find_entry_slots, find_locked_nfts, get_coinset_client, get_constants,
    get_last_onchain_timestamp, hex_string_to_bytes32, load_csv_matching_root,
    merkle_proof_for_nft, no_assets, parse_amount, resolve_custody, spend_datastore_oracle,
    sync_datastore, sync_distributor, yes_no_prompt, CliError, Db, SageClient,
};

pub async fn reward_distributor_refresh(
    launcher_id_str: String,
    csv_path: String,
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

    let store_launcher_id = match distributor.info.constants.reward_distributor_type {
        RewardDistributorType::CuratedNft {
            store_launcher_id,
            refreshable: true,
        } => store_launcher_id,
        RewardDistributorType::CuratedNft {
            refreshable: false, ..
        } => {
            return Err(CliError::Custom(
                "This reward distributor is not refreshable".to_string(),
            ));
        }
        _ => {
            return Err(CliError::Custom(
                "Refresh is only supported for refreshable curated NFT reward distributors"
                    .to_string(),
            ));
        }
    };

    ensure_epoch_open(&client, &distributor).await?;

    let latest_timestamp = get_last_onchain_timestamp(&client).await?;
    let also_sync = distributor.info.state.round_time_info.last_update + 180 < latest_timestamp;
    if also_sync {
        println!(
            "Will also sync the distributor to timestamp {}",
            latest_timestamp
        );
    }

    println!("Syncing datastore...");
    let datastore =
        sync_datastore(&client, &mut ctx, store_launcher_id, &delegated_puzzles()).await?;
    let records = load_csv_matching_root(&csv_path, datastore.info.metadata.root_hash)?;

    let sage = SageClient::new()?;
    let custody = resolve_custody(&sage, custody_address).await?;
    println!(
        "Using the following address as custody: {}",
        Address::new(custody.puzzle_hash, crate::get_prefix(testnet11)).encode()?
    );

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

    let locked_nfts = find_locked_nfts(
        &mut ctx,
        &client,
        launcher_id,
        custody.puzzle_hash,
        distributor.info.constants.reward_distributor_type,
        entry_slot.info.value.shares,
    )
    .await?;

    if locked_nfts.is_empty() {
        return Err(CliError::Custom(
            "No locked NFTs found for refresh at this custody address".to_string(),
        ));
    }

    let mut refresh_nfts = Vec::new();
    let mut deltas = Vec::new();
    let mut new_shares = Vec::new();
    let mut proofs = Vec::new();

    for (nft, old_shares) in &locked_nfts {
        let merkle_entry = merkle_proof_for_nft(&records, nft.info.launcher_id)?;
        let delta = i64::try_from(merkle_entry.weight)
            .map_err(|_| CliError::Custom("CSV weight is too large".to_string()))?
            - i64::try_from(*old_shares)
                .map_err(|_| CliError::Custom("Locked share count is too large".to_string()))?;

        if delta == 0 {
            println!(
                "Skipping {} (weight unchanged at {})",
                Address::new(nft.info.launcher_id, "nft".to_string()).encode()?,
                old_shares
            );
            continue;
        }

        println!(
            "Refreshing {}: {} -> {} shares (delta {delta})",
            Address::new(nft.info.launcher_id, "nft".to_string()).encode()?,
            old_shares,
            merkle_entry.weight
        );
        refresh_nfts.push(*nft);
        deltas.push(delta);
        new_shares.push(merkle_entry.weight);
        proofs.push(merkle_entry.proof);
    }

    if refresh_nfts.is_empty() {
        return Err(CliError::Custom(
            "No locked NFTs need refreshing - CSV weights match current locked shares".to_string(),
        ));
    }

    println!("A one-sided offer will be created. It will contain:");
    println!("  - 1 mojo");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let dl_fields = curated_datastore_fields(&datastore, &mut ctx)?;
    let dl_spend = spend_datastore_oracle(&mut ctx, datastore, &delegated_puzzles())?;
    ctx.insert(dl_spend);

    let mut sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    let nft_slice = refresh_nfts.as_slice();
    let (refresh_conds, _new_locked_nfts) = distributor
        .new_action::<RewardDistributorRefreshAction>()
        .spend(
            &mut ctx,
            &mut distributor,
            vec![entry_slot],
            &[nft_slice],
            &[deltas.as_slice()],
            &[new_shares.as_slice()],
            &[proofs.as_slice()],
            dl_fields.dl_root_hash,
            dl_fields.dl_metadata_rest_hash,
            dl_fields.dl_metadata_updater_hash_hash,
            dl_fields.dl_inner_puzzle_hash,
        )?;
    sec_conds = sec_conds.extend(refresh_conds);

    let (_new_distributor, pending_sig) = distributor.finish_spend(&mut ctx, vec![])?;
    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        sec_conds.reserve_fee(1),
        &security_coin_sk,
        get_constants(testnet11),
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

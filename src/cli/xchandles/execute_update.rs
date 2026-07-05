use chia_protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, Asset, Offer, SingletonInfo,
        SpendContext, XchandlesExecuteUpdateAction,
    },
    types::{
        puzzles::{CompactCoinProof, XchandlesHandleSlotValue, XchandlesSlotNonce},
        Conditions,
    },
    utils::Address,
};
use clvm_utils::ToTreeHash;

use crate::{
    assets_xch_only, confirm_pushed_transaction, fetch_nft_from_wallet, find_xchandles_update_slot,
    get_coinset_client, get_constants, hex_string_to_bytes32, no_assets, parse_amount,
    quick_sync_xchandles, recreate_nft_in_wallet, sync_xchandles, yes_no_prompt, CliError, Db,
    SageClient, XchandlesApiClient,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_execute_update(
    launcher_id_str: String,
    handle: String,
    new_nft: String,
    testnet11: bool,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let new_nft_launcher_id = Address::decode(&new_nft)?.puzzle_hash;
    let fee = parse_amount(&fee_str, false)?;
    let handle_hash = handle.tree_hash().into();

    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);
    let sage = SageClient::new()?;

    print!("First, let's sync the registry... ");
    let mut db = Db::new(false).await?;
    let mut registry = if local {
        sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?
    } else {
        quick_sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?
    };
    println!("done.");

    print!("Fetching handle slot...");
    let handle_slot = if local {
        let slot_value_hash = db
            .get_xchandles_indexed_slot_value(launcher_id, handle.tree_hash().into())
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))?;
        db.get_slot::<XchandlesHandleSlotValue>(
            &mut ctx,
            launcher_id,
            XchandlesSlotNonce::HANDLE.to_u64(),
            slot_value_hash,
            0,
        )
        .await?
        .ok_or(CliError::SlotNotFound("Handle"))?
    } else {
        XchandlesApiClient::get(testnet11)
            .get_slot_value(launcher_id, handle.tree_hash().into())
            .await?
    };
    println!("done.");

    println!("Handle: {}", handle);
    let current_owner_nft =
        Address::new(handle_slot.info.value.owner_launcher_id, "nft".to_string()).encode()?;
    println!("Current owner NFT: {}", current_owner_nft);

    let (current_owner_nft, current_owner_p2_layer) =
        fetch_nft_from_wallet(&mut ctx, &sage, &cli, current_owner_nft).await?;
    let (new_owner_nft, new_owner_p2_layer) =
        fetch_nft_from_wallet(&mut ctx, &sage, &cli, new_nft).await?;

    print!("Fetching update slot...");
    let update_slot = find_xchandles_update_slot(
        &mut ctx,
        &cli,
        registry.info.constants,
        current_owner_nft.coin.parent_coin_info,
        handle_hash,
    )
    .await?;
    println!("done.");

    println!("A one-sided offer will be created; it will consume:");
    println!("  - 1 mojo");
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    println!("For security, your two NFTs (current owner and new owner) will be spent separately and re-created into your wallet.");

    yes_no_prompt("Continue with update execution?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    loop {
        let resp = cli.get_blockchain_state().await?;
        let Some(blockchain_state) = resp.blockchain_state else {
            eprintln!("Failed to get blockchain state - aborting...");
            return Ok(());
        };

        if blockchain_state.peak.height >= update_slot.info.value.min_height {
            break;
        }

        println!(
            "Latest block is #{}; waiting for {} more blocks...",
            blockchain_state.peak.height,
            update_slot.info.value.min_height - blockchain_state.peak.height
        );
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }

    let current_owner_proof = CompactCoinProof {
        parent_coin_info: current_owner_nft.coin.parent_coin_info,
        inner_puzzle_hash: current_owner_nft.info.inner_puzzle_hash().into(),
        amount: current_owner_nft.coin.amount,
    };
    let new_nft_inner_ph = new_owner_nft.info.inner_puzzle_hash().into();

    let (old_owner_conds, mut new_owner_conds, new_resolved_conds) = registry
        .new_action::<XchandlesExecuteUpdateAction>()
        .spend(
            &mut ctx,
            &mut registry,
            handle_slot,
            update_slot,
            new_nft_launcher_id,
            new_nft_launcher_id,
            current_owner_proof,
            new_nft_inner_ph,
            new_nft_inner_ph,
        )?;

    new_owner_conds = new_owner_conds.extend(new_resolved_conds);
    let current_owner_coin_id = current_owner_nft.coin_id();
    let new_owner_coin_id = new_owner_nft.coin_id();

    let current_owner_sig = recreate_nft_in_wallet(
        &mut ctx,
        &sage,
        current_owner_nft,
        current_owner_p2_layer,
        old_owner_conds,
    )
    .await?;

    let new_owner_sig = recreate_nft_in_wallet(
        &mut ctx,
        &sage,
        new_owner_nft,
        new_owner_p2_layer,
        new_owner_conds,
    )
    .await?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        Conditions::new()
            .assert_concurrent_spend(current_owner_coin_id)
            .assert_concurrent_spend(new_owner_coin_id),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &current_owner_sig + &new_owner_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    if confirm_pushed_transaction(&cli, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

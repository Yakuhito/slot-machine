use chia_protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, Offer, SingletonInfo,
        SpendContext, XchandlesInitiateUpdateAction,
    },
    types::{
        puzzles::{CompactCoinProof, XchandlesHandleSlotValue, XchandlesSlotNonce},
        Conditions,
    },
    utils::Address,
};
use clvm_utils::ToTreeHash;

use crate::{
    assets_xch_only, confirm_pushed_transaction, fetch_nft_from_wallet, get_coinset_client,
    get_constants, hex_string_to_bytes32, no_assets, parse_amount, quick_sync_xchandles,
    recreate_nft_in_wallet, sync_xchandles, yes_no_prompt, CliError, Db, SageClient,
    XchandlesApiClient,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_initiate_update(
    launcher_id_str: String,
    handle: String,
    new_nft: String,
    min_height: Option<u32>,
    testnet11: bool,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let new_nft_launcher_id = Address::decode(&new_nft)?.puzzle_hash;
    let fee = parse_amount(&fee_str, false)?;

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
    let slot = if local {
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

    let return_address = sage.get_derivations(false, 0, 1).await?.derivations[0]
        .clone()
        .address;

    let peak_height = cli
        .get_blockchain_state()
        .await?
        .blockchain_state
        .ok_or(CliError::Custom(
            "Could not fetch blockchain state".to_string(),
        ))?
        .peak
        .height;
    let min_height = min_height.unwrap_or(peak_height + 1);

    println!("Handle: {}", handle);
    let current_owner_nft =
        Address::new(slot.info.value.owner_launcher_id, "nft".to_string()).encode()?;
    println!("Current owner: {}", current_owner_nft);
    println!(
        "Current resolved launcher id: {}",
        Address::new(slot.info.value.resolved_launcher_id, "nft".to_string()).encode()?
    );
    println!("New owner and resolved launcher id: {}", new_nft);
    println!("Minimum height: {}", min_height);
    println!("NFT return address: {}", return_address);

    let (nft, p2_layer) = fetch_nft_from_wallet(&mut ctx, &sage, &cli, current_owner_nft).await?;

    println!("A one-sided offer will be created; it will consume:");
    println!("  - 1 mojo");
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    println!("For security, your NFT will be spent separately and re-created into your wallet.");

    yes_no_prompt("Continue?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let owner_proof = CompactCoinProof {
        parent_coin_info: nft.coin.parent_coin_info,
        inner_puzzle_hash: nft.info.inner_puzzle_hash().into(),
        amount: nft.coin.amount,
    };

    let initiate_update_conds = registry
        .new_action::<XchandlesInitiateUpdateAction>()
        .spend(
            &mut ctx,
            &mut registry,
            slot,
            new_nft_launcher_id,
            new_nft_launcher_id,
            owner_proof,
            min_height,
        )?;

    let nft_coin_id = nft.coin.coin_id();
    let nft_sig =
        recreate_nft_in_wallet(&mut ctx, &sage, nft, p2_layer, initiate_update_conds).await?;
    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        Conditions::new().assert_concurrent_spend(nft_coin_id),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &nft_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    if confirm_pushed_transaction(&cli, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed! Finish the update after the relative block height elapses.");
    }

    Ok(())
}

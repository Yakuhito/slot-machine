use chia_protocol::{Bytes32, SpendBundle};
use chia_puzzle_types::standard::StandardArgs;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, sign_standard_transaction, spend_security_coin,
        spend_settlement_nft, Offer, SingletonInfo, Spend, SpendContext, StandardLayer,
        XchandlesInitiateUpdateAction,
    },
    types::puzzles::{CompactCoinProof, XchandlesHandleSlotValue},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvm_utils::ToTreeHash;
use clvmr::NodePtr;

use crate::{
    assets_xch_and_nft, get_coinset_client, get_constants, hex_string_to_bytes32, no_assets,
    confirm_pushed_transaction, parse_amount, quick_sync_xchandles, sync_xchandles, yes_no_prompt,
    CliError, Db,
    SageClient, XchandlesApiClient,
};

pub(crate) fn encode_nft(nft_launcher_id: Bytes32) -> Result<String, CliError> {
    Address::new(nft_launcher_id, "nft".to_string())
        .encode()
        .map_err(CliError::from)
}

pub(crate) async fn fetch_handle_slot(
    launcher_id: Bytes32,
    handle: &str,
    local: bool,
    testnet11: bool,
    db: &mut Db,
    ctx: &mut SpendContext,
) -> Result<chia_wallet_sdk::driver::Slot<XchandlesHandleSlotValue>, CliError> {
    if local {
        let slot_value_hash = db
            .get_xchandles_indexed_slot_value(launcher_id, handle.tree_hash().into())
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))?;
        db.get_slot::<XchandlesHandleSlotValue>(ctx, launcher_id, 0, slot_value_hash, 0)
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))
    } else {
        XchandlesApiClient::get(testnet11)
            .get_slot_value(launcher_id, handle.tree_hash().into())
            .await
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_initiate_update(
    launcher_id_str: String,
    handle: String,
    new_owner_nft: Option<String>,
    new_resolved_nft: Option<String>,
    min_height: Option<u32>,
    testnet11: bool,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
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
    let slot = fetch_handle_slot(launcher_id, &handle, local, testnet11, &mut db, &mut ctx).await?;
    println!("done.");

    let new_owner_launcher_id = if let Some(new_owner_nft) = new_owner_nft {
        Address::decode(&new_owner_nft)?.puzzle_hash
    } else {
        slot.info.value.owner_launcher_id
    };
    let new_resolved_launcher_id = if let Some(new_resolved_nft) = new_resolved_nft {
        Address::decode(&new_resolved_nft)?.puzzle_hash
    } else {
        slot.info.value.resolved_launcher_id
    };

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
    println!(
        "Current owner: {}",
        encode_nft(slot.info.value.owner_launcher_id)?
    );
    println!(
        "Current resolved launcher id: {}",
        encode_nft(slot.info.value.resolved_launcher_id)?
    );
    println!("New owner: {}", encode_nft(new_owner_launcher_id)?);
    println!(
        "New resolved launcher id: {}",
        encode_nft(new_resolved_launcher_id)?
    );
    println!("Minimum height: {}", min_height);
    println!("NFT return address: {}", return_address);

    yes_no_prompt("Continue with update initiation?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_nft(1, encode_nft(slot.info.value.owner_launcher_id)?),
            fee,
            None,
            None,
            false,
        )
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;
    let pk = security_coin_sk.public_key();
    let nft_inner_ph: Bytes32 = StandardArgs::curry_tree_hash(pk).into();
    let (nft, security_conds) = spend_settlement_nft(
        &mut ctx,
        &offer,
        slot.info.value.owner_launcher_id,
        registry.coin.coin_id(),
        nft_inner_ph,
    )?;

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
            new_owner_launcher_id,
            new_resolved_launcher_id,
            owner_proof,
            min_height,
        )?;

    let nft_return_ph: Bytes32 = Address::decode(&return_address)?.puzzle_hash;
    let nft_inner_spend =
        initiate_update_conds.create_coin(nft_return_ph, 1, ctx.hint(nft_return_ph)?);
    let nft_inner_spend = ctx.alloc(&clvm_quote!(nft_inner_spend))?;
    let nft_inner_spend = StandardLayer::new(pk)
        .delegated_inner_spend(&mut ctx, Spend::new(nft_inner_spend, NodePtr::NIL))?;

    let nft_sig = sign_standard_transaction(
        &mut ctx,
        nft.coin,
        nft_inner_spend,
        &security_coin_sk,
        get_constants(testnet11),
    )?;
    let _new_nft = nft.spend(&mut ctx, nft_inner_spend)?;

    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        security_conds,
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

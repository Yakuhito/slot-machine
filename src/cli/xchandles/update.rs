use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, SpendBundle},
};
use chia_puzzle_types::{
    offer::{NotarizedPayment, Payment},
    standard::StandardArgs,
};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Offer, Spend, SpendContext, StandardLayer},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvmr::NodePtr;

use crate::{
    assets_xch_and_nft, get_coinset_client, get_constants, hex_string_to_bytes32, new_sk,
    no_assets, parse_amount, parse_one_sided_offer, quick_sync_xchandles,
    sign_standard_transaction, spend_security_coin, sync_xchandles, wait_for_coin, yes_no_prompt,
    CliError, Db, SageClient, XchandlesApiClient, XchandlesSlotValue, XchandlesUpdateAction,
};

fn encode_nft(nft_launcher_id: Bytes32) -> Result<String, CliError> {
    Address::new(nft_launcher_id, "nft".to_string())
        .encode()
        .map_err(CliError::from)
}

pub async fn xchandles_update(
    launcher_id_str: String,
    handle: String,
    new_owner_nft: Option<String>,
    new_resolved_nft: Option<String>,
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
    let slot = if local {
        let slot_value_hash = db
            .get_xchandles_indexed_slot_value(launcher_id, handle.tree_hash().into())
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))?;
        db.get_slot::<XchandlesSlotValue>(&mut ctx, launcher_id, 0, slot_value_hash, 0)
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))?
    } else {
        let xchandles_api_client = XchandlesApiClient::get(testnet11);
        xchandles_api_client
            .get_slot_value(launcher_id, handle.tree_hash().into())
            .await?
    };
    println!("done.");

    let new_owner_launcher_id = if let Some(new_owner_nft) = new_owner_nft {
        Address::decode(&new_owner_nft)?.puzzle_hash
    } else {
        slot.info.value.owner_launcher_id
    };
    let new_resolved_data = if let Some(new_resolved_nft) = new_resolved_nft {
        Address::decode(&new_resolved_nft)?.puzzle_hash.into()
    } else {
        slot.info.value.resolved_data.clone()
    };

    let return_address = sage.get_derivations(false, 0, 1).await?.derivations[0]
        .clone()
        .address;

    println!("Handle: {}", handle);
    println!(
        "Current owner: {}",
        encode_nft(slot.info.value.owner_launcher_id)?
    );
    println!(
        "Current resolved data: {}",
        if let Ok(resolved_data) = slot.info.value.resolved_data.clone().try_into() {
            encode_nft(resolved_data)?
        } else {
            hex::encode(slot.info.value.resolved_data.clone())
        }
    );
    println!("New owner: {}", encode_nft(new_owner_launcher_id)?);
    println!(
        "New resolved data: {}",
        if let Ok(resolved_data) = new_resolved_data.clone().try_into() {
            encode_nft(resolved_data)?
        } else {
            hex::encode(new_resolved_data.clone())
        }
    );
    println!("NFT return address: {}", return_address);

    yes_no_prompt("Continue with update?")?;

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

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let pk = security_coin_sk.public_key();
    let nft_inner_ph: Bytes32 = StandardArgs::curry_tree_hash(pk).into();
    let hint = ctx.hint(nft_inner_ph)?;
    let offer = parse_one_sided_offer(
        &mut ctx,
        offer,
        security_coin_sk.public_key(),
        None,
        Some(NotarizedPayment {
            nonce: registry.coin.coin_id(),
            payments: vec![Payment::new(nft_inner_ph, 1, hint)],
        }),
    )?;

    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let nft = offer.created_nft.unwrap();

    let nft_inner_conds = registry.new_action::<XchandlesUpdateAction>().spend(
        &mut ctx,
        &mut registry,
        slot,
        new_owner_launcher_id,
        new_resolved_data,
        nft.info.inner_puzzle_hash().into(),
    )?;

    let nft_return_ph: Bytes32 = Address::decode(&return_address)?.puzzle_hash;
    let nft_inner_spend = nft_inner_conds.create_coin(nft_return_ph, 1, ctx.hint(nft_return_ph)?);
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
    nft.spend(&mut ctx, nft_inner_spend)?;

    let _new_registry = registry.finish_spend(&mut ctx)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer.security_base_conditions,
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let sb = SpendBundle::new(
        ctx.take(),
        offer.aggregated_signature + &security_coin_sig + &nft_sig,
    );

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&cli, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

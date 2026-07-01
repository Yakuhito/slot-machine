use chia_protocol::{Bytes32, SpendBundle};
use chia_puzzle_types::standard::StandardArgs;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, sign_standard_transaction, spend_security_coin,
        spend_settlement_nft, Offer, SingletonInfo, Slot, Spend, SpendContext, StandardLayer,
        XchandlesExecuteUpdateAction,
    },
    types::puzzles::{CompactCoinProof, XchandlesSlotNonce, XchandlesUpdateSlotValue},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvm_utils::ToTreeHash;
use clvmr::NodePtr;

use crate::{
    assets_xch_and_nft, get_coinset_client, get_constants, hex_string_to_bytes32, no_assets,
    confirm_pushed_transaction, parse_amount, quick_sync_xchandles, sync_xchandles, yes_no_prompt,
    CliError, Db,
    SageClient,
};

use super::initiate_update::{encode_nft, fetch_handle_slot};

async fn find_update_slot(
    ctx: &mut SpendContext,
    client: &chia_wallet_sdk::coinset::CoinsetClient,
    db: &Db,
    launcher_id: Bytes32,
    handle_hash: Bytes32,
    local: bool,
) -> Result<Slot<XchandlesUpdateSlotValue>, CliError> {
    if local {
        let slots = db
            .get_slots::<XchandlesUpdateSlotValue>(
                ctx,
                launcher_id,
                XchandlesSlotNonce::UPDATE.to_u64(),
                0,
            )
            .await?;
        return slots
            .into_iter()
            .find(|slot| slot.info.value.handle_hash == handle_hash)
            .ok_or(CliError::SlotNotFound("Update"));
    }

    let constants = db
        .get_xchandles_configuration(ctx, launcher_id)
        .await?
        .ok_or(CliError::ConstantsNotSet)?;

    let records = client
        .get_coin_records_by_hint(launcher_id, None, None, Some(false), None)
        .await?
        .coin_records
        .unwrap_or_default();

    for record in records {
        if record.spent {
            continue;
        }

        let parent_spend = client
            .get_puzzle_and_solution(
                record.coin.parent_coin_info,
                Some(record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(record.coin.parent_coin_info))?;

        let Some(registry) = chia_wallet_sdk::driver::XchandlesRegistry::from_parent_spend(
            ctx,
            &parent_spend,
            constants,
        )?
        else {
            continue;
        };

        for update_slot_value in registry.pending_spend.created_update_slots.iter() {
            if update_slot_value.handle_hash != handle_hash {
                continue;
            }
            let slot = registry.created_update_slot_value_to_slot(*update_slot_value);
            if slot.coin == record.coin {
                return Ok(slot);
            }
        }
    }

    Err(CliError::SlotNotFound("Update"))
}

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_finish_update(
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
    let handle_slot =
        fetch_handle_slot(launcher_id, &handle, local, testnet11, &mut db, &mut ctx).await?;
    println!("done.");

    print!("Fetching update slot...");
    let update_slot =
        find_update_slot(&mut ctx, &cli, &db, launcher_id, handle_hash, local).await?;
    println!("done.");

    let new_owner_launcher_id = new_owner_nft
        .map(|nft| Address::decode(&nft).map(|a| a.puzzle_hash))
        .transpose()?
        .unwrap_or(update_slot.info.value.new_owner_launcher_id);
    let new_resolved_launcher_id = new_resolved_nft
        .map(|nft| Address::decode(&nft).map(|a| a.puzzle_hash))
        .transpose()?
        .unwrap_or(update_slot.info.value.new_resolved_launcher_id);

    let peak_height = cli
        .get_blockchain_state()
        .await?
        .blockchain_state
        .ok_or(CliError::Custom(
            "Could not fetch blockchain state".to_string(),
        ))?
        .peak
        .height;

    if peak_height < update_slot.info.value.min_height {
        return Err(CliError::Custom(format!(
            "Update cannot be executed yet - minimum height is {}",
            update_slot.info.value.min_height
        )));
    }

    let return_address = sage.get_derivations(false, 0, 1).await?.derivations[0]
        .clone()
        .address;

    println!("Handle: {}", handle);
    println!("New owner: {}", encode_nft(new_owner_launcher_id)?);
    println!(
        "New resolved launcher id: {}",
        encode_nft(new_resolved_launcher_id)?
    );
    println!("NFT return address: {}", return_address);

    yes_no_prompt("Continue with update execution?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_nft(1, encode_nft(handle_slot.info.value.owner_launcher_id)?),
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
        handle_slot.info.value.owner_launcher_id,
        registry.coin.coin_id(),
        nft_inner_ph,
    )?;

    let owner_proof = CompactCoinProof {
        parent_coin_info: nft.coin.parent_coin_info,
        inner_puzzle_hash: nft.info.inner_puzzle_hash().into(),
        amount: nft.coin.amount,
    };

    let new_owner_inner_puzzle_hash = nft_inner_ph;
    let new_resolved_inner_puzzle_hash = nft_inner_ph;
    let update_min_height = update_slot.info.value.min_height;

    let (old_owner_conds, _new_owner_conds, _new_resolved_conds) = registry
        .new_action::<XchandlesExecuteUpdateAction>()
        .spend(
            &mut ctx,
            &mut registry,
            handle_slot,
            update_slot,
            new_owner_launcher_id,
            new_resolved_launcher_id,
            owner_proof,
            update_min_height,
            new_owner_inner_puzzle_hash,
            new_resolved_inner_puzzle_hash,
        )?;

    let nft_return_ph: Bytes32 = Address::decode(&return_address)?.puzzle_hash;
    let nft_inner_spend = old_owner_conds.create_coin(nft_return_ph, 1, ctx.hint(nft_return_ph)?);
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
        println!("Confirmed!");
    }

    Ok(())
}

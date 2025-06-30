use chia::{clvm_utils::ToTreeHash, protocol::SpendBundle};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, Offer, SpendContext},
};

use crate::{
    assets_xch_and_cat, create_security_coin, get_coinset_client, get_constants,
    get_last_onchain_timestamp, hex_string_to_bytes32, no_assets, parse_amount,
    quick_sync_xchandles, spend_security_coin, spend_settlement_cats, sync_xchandles,
    wait_for_coin, yes_no_prompt, CliError, Db, DefaultCatMakerArgs, SageClient,
    XchandlesApiClient, XchandlesExtendAction, XchandlesFactorPricingPuzzleArgs,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_extend(
    launcher_id_str: String,
    handle: String,
    num_periods: u64,
    testnet11: bool,
    payment_asset_id_str: String,
    payment_cat_base_price_str: String,
    registration_period: u64,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let payment_asset_id = hex_string_to_bytes32(&payment_asset_id_str)?;
    let payment_cat_base_price = parse_amount(&payment_cat_base_price_str, true)?;
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

    if DefaultCatMakerArgs::curry_tree_hash(payment_asset_id.tree_hash().into())
        != registry.info.state.cat_maker_puzzle_hash.into()
        || registry.info.state.pricing_puzzle_hash
            != XchandlesFactorPricingPuzzleArgs::curry_tree_hash(
                payment_cat_base_price,
                registration_period,
            )
            .into()
    {
        return Err(CliError::Custom(
            "Given payment asset id & base price do not match the current registry's state."
                .to_string(),
        ));
    }

    let payment_cat_amount =
        XchandlesFactorPricingPuzzleArgs::get_price(payment_cat_base_price, &handle, num_periods);

    println!("Handle: {}", handle);
    println!(
        "Payment CAT amount: {:.3}",
        payment_cat_amount as f64 / 1000.0
    );
    println!("Fee: {} XCH", fee_str);

    let slot = if local {
        let slot_value_hash = db
            .get_xchandles_indexed_slot_value(launcher_id, handle.tree_hash().into())
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))?;
        db.get_slot(&mut ctx, launcher_id, 0, slot_value_hash, 0)
            .await?
            .ok_or(CliError::SlotNotFound("Handle"))?
    } else {
        let xchandles_api_client = XchandlesApiClient::get(testnet11);
        xchandles_api_client
            .get_slot_value(launcher_id, handle.tree_hash().into())
            .await?
    };
    println!("Current expiration: {}", slot.info.value.expiration);

    let start_time = get_last_onchain_timestamp(&cli).await? - 1;
    println!("Extension time: {}", start_time);

    let (sec_conds, notarized_payment) = registry.new_action::<XchandlesExtendAction>().spend(
        &mut ctx,
        &mut registry,
        handle,
        slot,
        payment_asset_id,
        payment_cat_base_price,
        registration_period,
        num_periods,
        start_time,
    )?;

    yes_no_prompt("Continue with extension?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_cat(1, payment_asset_id_str, payment_cat_amount),
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

    let (_cats, payment_assertion) = spend_settlement_cats(
        &mut ctx,
        &offer,
        payment_asset_id,
        notarized_payment.nonce,
        vec![(
            notarized_payment.payments[0].puzzle_hash,
            notarized_payment.payments[0].amount,
        )],
    )?;
    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        sec_conds.extend(payment_assertion),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&cli, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

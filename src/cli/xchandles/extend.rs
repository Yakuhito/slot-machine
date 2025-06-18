use chia::{clvm_utils::ToTreeHash, protocol::SpendBundle};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Offer, SpendContext},
};
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, new_sk, parse_amount,
    parse_one_sided_offer, quick_sync_xchandles, spend_security_coin, sync_xchandles,
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

    let (notarized_payment, sec_conds, _new_slot) =
        registry.new_action::<XchandlesExtendAction>().spend(
            &mut ctx,
            &mut registry,
            handle,
            slot,
            payment_asset_id,
            payment_cat_base_price,
            registration_period,
            num_periods,
        )?;

    yes_no_prompt("Continue with extension?")?;

    let offer_resp = sage
        .make_offer(MakeOffer {
            requested_assets: Assets {
                xch: Amount::u64(0),
                cats: vec![],
                nfts: vec![],
            },
            offered_assets: Assets {
                xch: Amount::u64(1),
                cats: vec![CatAmount {
                    asset_id: payment_asset_id_str,
                    amount: Amount::u64(payment_cat_amount),
                }],
                nfts: vec![],
            },
            fee: Amount::u64(fee),
            receive_address: None,
            expires_at_second: None,
            auto_import: false,
        })
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(
        &mut ctx,
        offer,
        security_coin_sk.public_key(),
        Some(notarized_payment),
        None,
    )?;

    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let _new_registry = registry.finish_spend(&mut ctx)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer.security_base_conditions.extend(sec_conds),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let sb = SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig);

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&cli, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

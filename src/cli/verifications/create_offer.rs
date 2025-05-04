use chia_wallet_sdk::{
    driver::{Offer, SpendContext},
    types::{MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};
use clvm_traits::clvm_list;
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, get_latest_data_for_asset_id, hex_string_to_bytes32, new_sk, parse_amount,
    parse_one_sided_offer, spend_security_coin, yes_no_prompt, CliError, SageClient,
};

pub async fn verifications_create_offer(
    launcher_id_str: String,
    asset_id_str: String,
    comment: String,
    payment_asset_id_str: String,
    payment_amount_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let asset_id = hex_string_to_bytes32(&asset_id_str)?;
    let payment_asset_id = hex_string_to_bytes32(&payment_asset_id_str)?;
    let payment_amount = parse_amount(&payment_amount_str, true)?;
    let fee = parse_amount(&fee_str, false)?;

    let mut ctx = SpendContext::new();
    let client = get_coinset_client(testnet11);

    println!("Syncing asset id data...");
    let latest_data = get_latest_data_for_asset_id(&mut ctx, &client, asset_id, testnet11).await?;

    println!("CAT NFT Metadata: ");
    latest_data.pretty_print("  ");
    println!("Note: Attestations cover the following: ticker, name, description, image hash, metadata hash, license hash.");

    println!(
        "A one-sided offer offering 1 mojo, {} payment CATs, and {} XCH ({} mojos) as fee will be generated and broadcast.",
        payment_amount_str,
        fee_str,
        fee
    );
    println!("The resulting offer will be used to create a verification request, which you can send to your chosen verifier.");
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
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
                    asset_id: hex::encode(payment_asset_id),
                    amount: Amount::u64(payment_amount),
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
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, false)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let mut security_coin_conditions = offer.security_base_conditions;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        security_coin_conditions,
        &security_coin_sk,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
    )?;

    let whole_sig = offer.aggregated_signature + &security_coin_sig;
    let data = clvm_list!(
        asset_id,
        comment,
        payment_asset_id,
        payment_amount,
        ctx.take(),
        whole_sig,
    );

    Ok(())
}

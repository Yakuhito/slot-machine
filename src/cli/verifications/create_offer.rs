use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32, Coin, CoinSpend, Program, SpendBundle},
};
use chia_puzzle_types::Memos;
use chia_wallet_sdk::{
    driver::{
        create_security_coin, decode_offer, encode_offer, spend_security_coin, Offer, SpendContext,
        VerificationAsserter, VerifiedData,
    },
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};
use clvm_traits::{clvm_list, clvm_quote};
use clvmr::serde::node_to_bytes;

use crate::{
    assets_xch_and_cat, get_coinset_client, get_latest_data_for_asset_id, hex_string_to_bytes32,
    no_assets, parse_amount, yes_no_prompt, CliError, SageClient,
};

pub async fn verifications_create_offer(
    launcher_id_str: String,
    asset_id_str: String,
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
        "A one-sided offer offering 1 mojo, {} payment CATs, and {} XCH ({} mojos) as fee will be generated.",
        payment_amount_str,
        fee_str,
        fee
    );
    println!("The resulting offer will be used to create a verification request, which you can send to your chosen verifier.");
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_and_cat(1, hex::encode(payment_asset_id), payment_amount),
            fee,
            None,
            None,
            true,
        )
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let verified_data = VerifiedData::from_cat_nft_metadata(asset_id, &latest_data, "".to_string());
    let verification_asserter = VerificationAsserter::from(
        launcher_id,
        verified_data.version,
        verified_data.asset_id.tree_hash(),
        verified_data.data_hash.tree_hash(),
    );
    let verification_asserter_puzzle_hash: Bytes32 = verification_asserter.tree_hash().into();

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let security_coin_conditions = Conditions::new()
        .reserve_fee(1)
        .create_coin(verification_asserter_puzzle_hash, 0, Memos::None)
        .assert_concurrent_puzzle(verification_asserter_puzzle_hash);

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        security_coin_conditions,
        &security_coin_sk,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
    )?;

    let latest_data_bytes = ctx.alloc(&clvm_quote!(latest_data))?;
    let latest_data_bytes: Bytes = node_to_bytes(&ctx, latest_data_bytes)?.into();
    let solution_bytes = ctx.alloc(&clvm_list!(security_coin.coin_id()))?;
    let solution_bytes: Bytes = node_to_bytes(&ctx, solution_bytes)?.into();
    ctx.insert(CoinSpend::new(
        Coin::new(Bytes32::new([1; 32]), asset_id, 0),
        Program::new(latest_data_bytes),
        Program::new(solution_bytes),
    ));
    let sb = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

    let verification_request = encode_offer(&sb)?;
    println!(
        "Verification request: {}",
        verification_request.replace("offer1", "verificationrequest1")
    );

    Ok(())
}

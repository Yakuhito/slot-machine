use bech32::{u5, Variant};
use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, SpendBundle},
    traits::Streamable,
};
use chia_puzzle_types::offer::{NotarizedPayment, Payment};
use chia_puzzles::SETTLEMENT_PAYMENT_HASH;
use chia_wallet_sdk::{
    driver::{compress_offer_bytes, Offer, SpendContext},
    types::{MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};
use clvm_traits::clvm_list;
use clvmr::serde::node_to_bytes;
use sage_api::{Amount, Assets, CatAmount, MakeOffer};

use crate::{
    get_coinset_client, get_latest_data_for_asset_id, hex_string_to_bytes32, new_sk, parse_amount,
    parse_one_sided_offer, spend_security_coin, yes_no_prompt, CliError, SageClient,
    VerificationAsserter, VerifiedData,
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
        "A one-sided offer offering 1 mojo, {} payment CATs, and {} XCH ({} mojos) as fee will be generated.",
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
            auto_import: true,
        })
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let verified_data =
        VerifiedData::from_cat_nft_metadata(asset_id, &latest_data, comment.clone());
    let verification_asserter = VerificationAsserter::from(
        launcher_id,
        verified_data.version,
        verified_data.asset_id.tree_hash(),
        verified_data.data_hash.tree_hash(),
    );
    let verification_asserter_puzzle_hash: Bytes32 = verification_asserter.tree_hash().into();

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(
        &mut ctx,
        offer,
        security_coin_sk.public_key(),
        Some(NotarizedPayment {
            nonce: Bytes32::default(),
            payments: vec![Payment::with_memos(
                SETTLEMENT_PAYMENT_HASH.into(),
                payment_amount,
                vec![SETTLEMENT_PAYMENT_HASH.to_vec().into()],
            )],
        }),
        None,
    )?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_conditions = offer
        .security_base_conditions
        .create_coin(SETTLEMENT_PAYMENT_HASH.into(), 1, None)
        .assert_concurrent_spend(offer.created_cat.unwrap().coin.coin_id())
        .create_coin(verification_asserter_puzzle_hash, 0, None)
        .assert_concurrent_puzzle(verification_asserter_puzzle_hash);

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
        SpendBundle::new(ctx.take(), whole_sig)
            .to_bytes()
            .map_err(|_| CliError::Custom(
                "Verification request serialization error 2".to_string()
            ))?,
    );
    let data = ctx.alloc(&data)?;

    let bytes = node_to_bytes(&ctx, data)?
        .to_bytes()
        .map_err(|_| CliError::Custom("Verification request serialization error 2".to_string()))?;
    let bytes = compress_offer_bytes(&bytes)?;
    let bytes = bech32::convert_bits(&bytes, 8, 5, true)?
        .into_iter()
        .map(u5::try_from_u8)
        .collect::<Result<Vec<_>, bech32::Error>>()?;
    let verification_request = bech32::encode("verificationrequest", bytes, Variant::Bech32m)?;

    println!("Verification request: {}", verification_request);

    Ok(())
}

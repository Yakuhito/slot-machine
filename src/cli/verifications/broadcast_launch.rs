use bech32::Variant;
use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32, Coin, SpendBundle},
    traits::Streamable,
};
use chia_puzzle_types::{
    cat::CatArgs,
    offer::{NotarizedPayment, Payment, SettlementPaymentsSolution},
    LineageProof,
};
use chia_puzzles::{CAT_PUZZLE_HASH, SETTLEMENT_PAYMENT_HASH};
use chia_wallet_sdk::{
    driver::{decompress_offer, Cat, CatInfo, CatSpend, HashedPtr, Launcher, Puzzle, Spend},
    types::{announcement_id, puzzles::SettlementPayment, Condition, Conditions},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32,
    multisig_broadcast_thing_finish, multisig_broadcast_thing_start, yes_no_prompt, CliError,
    MedievalVault, Verification, VerificationAsserter, VerificationLauncherKVList, VerifiedData,
};

#[allow(clippy::too_many_arguments)]
pub async fn verifications_broadcast_launch(
    launcher_id_str: String,
    asset_id_str: String,
    comment: String,
    request_offer: Option<String>,
    request_offer_recipient: Option<String>,
    signatures_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let asset_id = hex_string_to_bytes32(&asset_id_str)?;

    let (mut signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(signatures_str, launcher_id_str, testnet11).await?;

    println!(
        "\nFetching latest data for asset id {}... ",
        hex::encode(asset_id)
    );
    let latest_data = get_latest_data_for_asset_id(&mut ctx, &client, asset_id, testnet11).await?;

    println!("CAT NFT Metadata: ");
    latest_data.pretty_print("  ");
    println!("Note: Attestations cover the following: ticker, name, description, image hash, metadata hash, license hash.");
    println!("It is your responsibility to ensure the hashes are correct.");

    let verified_data =
        VerifiedData::from_cat_nft_metadata(asset_id, &latest_data, comment.clone());

    let launcher = Launcher::with_memos(
        medieval_vault.coin.coin_id(),
        0,
        ctx.hint(verified_data.get_hint())?,
    )
    .with_singleton_amount(1);
    let launcher_coin = launcher.coin();
    println!(
        "Verification launcher id: {}",
        hex::encode(launcher_coin.coin_id())
    );

    let (launch_conds, _coin) = launcher.spend(
        &mut ctx,
        Verification::inner_puzzle_hash(launcher_id, verified_data.clone()).into(),
        &VerificationLauncherKVList {
            revocation_singleton_launcher_id: launcher_id,
            verified_data: verified_data.clone(),
        },
    )?;

    let genesis_challenge = ctx.alloc(&get_constants(testnet11).genesis_challenge)?;
    let launch_conds_with_recreate = launch_conds.create_coin(
        medieval_vault.info.inner_puzzle_hash().into(),
        medieval_vault.coin.amount,
        ctx.hint(medieval_vault.info.launcher_id)?,
    );
    let delegated_puzzle = ctx.alloc(&clvm_quote!(MedievalVault::delegated_conditions(
        launch_conds_with_recreate,
        medieval_vault.coin.coin_id(),
        genesis_challenge,
    )))?;

    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    let verifier_proof = LineageProof {
        parent_parent_coin_info: medieval_vault.coin.parent_coin_info,
        parent_inner_puzzle_hash: medieval_vault.info.inner_puzzle_hash().into(),
        parent_amount: medieval_vault.coin.amount,
    };
    medieval_vault.spend_sunsafe(&mut ctx, &pubkeys, delegated_puzzle, NodePtr::NIL)?;

    let additional_conditions = if let Some(request_offer) = request_offer {
        // spend verification as well to make offer work
        let verification = Verification::after_mint(
            launcher_coin.parent_coin_info,
            launcher_id,
            verified_data.clone(),
        );
        verification.spend(&mut ctx, None)?;

        let (hrp, data, variant) = bech32::decode(&request_offer)?;
        if variant != Variant::Bech32m || hrp.as_str() != "verificationrequest" {
            return Err(CliError::Custom(
                "Invalid verification request offer provided".to_string(),
            ));
        }
        let bytes = bech32::convert_bits(&data, 5, 8, false)?;
        let decompressed = decompress_offer_bytes(&bytes)?;
        let ptr = node_from_bytes(&mut ctx, &decompressed)?;
        let (asset_id_verif, (spend_bundle_bytes, ())) =
            ctx.extract::<(Bytes32, (Bytes, ()))>(ptr)?;
        if asset_id_verif != asset_id {
            return Err(CliError::Custom(
                "Verification request offer made for another asset id :(".to_string(),
            ));
        }
        let spend_bundle = SpendBundle::from_bytes(&spend_bundle_bytes).unwrap();

        let verification_asserter = VerificationAsserter::from(
            launcher_id,
            verified_data.version,
            verified_data.asset_id.tree_hash(),
            verified_data.data_hash.tree_hash(),
        );
        let verification_asserter_puzzle_hash: Bytes32 = verification_asserter.tree_hash().into();

        let recipient_puzzle_hash = Address::decode(&request_offer_recipient.ok_or(
            CliError::Custom("Verification offer provided but recipient not specified".to_string()),
        )?)?
        .puzzle_hash;

        let mut payment_sent = false;
        let mut verification_asserter_spent = false;
        let mut conds = Conditions::new();
        for coin_spend in spend_bundle.coin_spends.into_iter() {
            let puzzle_ptr = ctx.alloc(&coin_spend.puzzle_reveal)?;
            let solution_ptr = ctx.alloc(&coin_spend.solution)?;
            let output = ctx.run(puzzle_ptr, solution_ptr)?;
            let output = ctx.extract::<Conditions>(output)?;

            let puzzle = Puzzle::parse(&ctx, puzzle_ptr);
            match puzzle {
                Puzzle::Curried(puzzle) => {
                    if puzzle.mod_hash == CAT_PUZZLE_HASH.into() {
                        let spent_cat_args = ctx.extract::<CatArgs<HashedPtr>>(puzzle.args)?;
                        let payment_cat_asset_id = spent_cat_args.asset_id;
                        let offer_puzzle_hash: Bytes32 = CatArgs::curry_tree_hash(
                            payment_cat_asset_id,
                            SETTLEMENT_PAYMENT_HASH.into(),
                        )
                        .into();

                        if let Some(cc) = output.iter().find_map(|c| match c {
                            Condition::CreateCoin(cc) => {
                                if cc.puzzle_hash == offer_puzzle_hash {
                                    Some(cc)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }) {
                            yes_no_prompt(format!("{} CAT mojos (asset id: {}) will be transferred to the specified recipient. Continue?", cc.amount, hex::encode(payment_cat_asset_id)).as_str())?;
                            let hint = ctx.hint(recipient_puzzle_hash)?;
                            let notarized_payment = NotarizedPayment {
                                nonce: coin_spend.coin.coin_id(),
                                payments: vec![Payment::new(
                                    recipient_puzzle_hash,
                                    cc.amount,
                                    hint,
                                )],
                            };

                            let offer_cat = Cat::new(
                                Coin::new(coin_spend.coin.coin_id(), offer_puzzle_hash, cc.amount),
                                Some(LineageProof {
                                    parent_parent_coin_info: coin_spend.coin.parent_coin_info,
                                    parent_inner_puzzle_hash: spent_cat_args
                                        .inner_puzzle
                                        .tree_hash()
                                        .into(),
                                    parent_amount: coin_spend.coin.amount,
                                }),
                                CatInfo::new(
                                    payment_cat_asset_id,
                                    None,
                                    SETTLEMENT_PAYMENT_HASH.into(),
                                ),
                            );

                            let offer_cat_inner_solution =
                                ctx.alloc(&SettlementPaymentsSolution {
                                    notarized_payments: vec![notarized_payment.clone()],
                                })?;

                            let offer_cat_spend = CatSpend::new(
                                offer_cat,
                                Spend::new(
                                    ctx.alloc_mod::<SettlementPayment>()?,
                                    offer_cat_inner_solution,
                                ),
                            );

                            Cat::spend_all(&mut ctx, &[offer_cat_spend])?;

                            payment_sent = true;
                            let notarized_payment_ptr = ctx.alloc(&notarized_payment)?;
                            let msg: Bytes32 = ctx.tree_hash(notarized_payment_ptr).into();
                            conds = conds.assert_puzzle_announcement(announcement_id(
                                offer_puzzle_hash,
                                msg,
                            ));
                        }
                    }
                }
                Puzzle::Raw(_puzzle) => {
                    if let Some(cc) = output.iter().find_map(|c| match c {
                        Condition::CreateCoin(cc) => {
                            if cc.puzzle_hash == verification_asserter_puzzle_hash {
                                Some(cc)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }) {
                        verification_asserter.spend(
                            &mut ctx,
                            Coin::new(coin_spend.coin.coin_id(), cc.puzzle_hash, cc.amount),
                            verifier_proof,
                            launcher_coin.amount,
                            comment.clone(),
                        )?;
                        verification_asserter_spent = true;
                    }
                }
            };

            ctx.insert(coin_spend);
        }

        if !payment_sent {
            return Err(CliError::Custom(
                "Payment in offer could not be found".to_string(),
            ));
        }
        if !verification_asserter_spent {
            return Err(CliError::Custom(
                "Verification asserter could not be found in offer - it is likely invalid"
                    .to_string(),
            ));
        }

        signature_from_signers += &spend_bundle.aggregated_signature;
        Some(conds)
    } else {
        None
    };

    multisig_broadcast_thing_finish(
        client,
        &mut ctx,
        signature_from_signers,
        fee_str,
        testnet11,
        medieval_vault_coin_id,
        additional_conditions,
    )
    .await
}

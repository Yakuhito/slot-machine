use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
};
use chia_puzzle_types::LineageProof;
use chia_wallet_sdk::{
    driver::{
        decode_offer, spend_settlement_cats, Launcher, MedievalVault, Offer, Verification,
        VerificationAsserter, VerificationLauncherKVList, VerifiedData,
    },
    types::{puzzles::CatNftMetadata, Conditions},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    get_constants, get_latest_data_for_asset_id, hex_string_to_bytes32,
    multisig_broadcast_thing_finish, multisig_broadcast_thing_start, yes_no_prompt, CliError,
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
        Verification::inner_puzzle_hash(launcher_id, &verified_data).into(),
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

        let offer = Offer::from_spend_bundle(
            &mut ctx,
            &decode_offer(&request_offer.replace("verificationrequest1", "offer1"))?,
        )?;

        let special_parent = Bytes32::from([1; 32]);
        let special_coin_spend = offer
            .spend_bundle()
            .coin_spends
            .iter()
            .find(|cs| cs.coin.parent_coin_info == special_parent)
            .ok_or(CliError::Custom("Special coin spend not found".to_string()))?;
        offer.spend_bundle().coin_spends.iter().for_each(|cs| {
            if cs.coin.parent_coin_info != special_parent {
                ctx.insert(cs.clone())
            };
        });
        signature_from_signers += &offer.spend_bundle().aggregated_signature;

        let asset_id_verif = special_coin_spend.coin.puzzle_hash;
        if asset_id_verif != asset_id {
            return Err(CliError::Custom(
                "Verification request offer made for another asset id :(".to_string(),
            ));
        }

        let special_puzzle = node_from_bytes(&mut ctx, &special_coin_spend.puzzle_reveal)?;
        let (_thing, nft_metadata_verif) = ctx.extract::<(u64, CatNftMetadata)>(special_puzzle)?;
        if latest_data != nft_metadata_verif {
            return Err(CliError::Custom(
                "Verification request offer made for a different version of metadata".to_string(),
            ));
        }

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

        let mut conds = Conditions::new();
        for (asset_id, cats) in offer.offered_coins().cats.iter() {
            let total_cat_amount = cats.iter().map(|c| c.coin.amount).sum::<u64>();
            println!(
                "Offer contains {} CAT mojos (asset id: {})",
                total_cat_amount,
                hex::encode(asset_id)
            );
            let (_new_cats, assert_cond) = spend_settlement_cats(
                &mut ctx,
                &offer,
                *asset_id,
                recipient_puzzle_hash,
                &[(recipient_puzzle_hash, total_cat_amount)],
            )?;
            conds = conds.extend(assert_cond);
        }

        let solution_ptr = node_from_bytes(&mut ctx, &special_coin_spend.solution)?;
        let (verification_asserter_parent, ()) = ctx.extract::<(Bytes32, ())>(solution_ptr)?;
        verification_asserter.spend(
            &mut ctx,
            Coin::new(
                verification_asserter_parent,
                verification_asserter_puzzle_hash,
                0,
            ),
            verifier_proof,
            launcher_coin.amount,
            comment.clone(),
        )?;

        yes_no_prompt("Accept the payments above?")?;
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

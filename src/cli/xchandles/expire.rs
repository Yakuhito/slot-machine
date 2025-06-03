use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, SpendBundle},
};
use chia_puzzle_types::{cat::CatArgs, singleton::SingletonStruct, LineageProof};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{CatLayer, DriverError, Layer, Offer, Puzzle, SpendContext},
    utils::Address,
};
use clvmr::{serde::node_from_bytes, NodePtr};
use sage_api::{Amount, Assets, GetDerivations, MakeOffer, SendCat};

use crate::{
    get_coinset_client, get_constants, get_last_onchain_timestamp, get_prefix,
    hex_string_to_bytes32, new_sk, parse_amount, parse_one_sided_offer, quick_sync_xchandles,
    spend_security_coin, sync_xchandles, wait_for_coin, yes_no_prompt, CliError, Db,
    DefaultCatMakerArgs, PrecommitCoin, PrecommitLayer, SageClient, Slot, XchandlesApiClient,
    XchandlesExpireAction, XchandlesExponentialPremiumRenewPuzzleArgs,
    XchandlesExponentialPremiumRenewPuzzleSolution, XchandlesFactorPricingPuzzleArgs,
    XchandlesFactorPricingSolution, XchandlesPrecommitValue, XchandlesRefundAction,
    XchandlesSlotValue,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_expire(
    launcher_id_str: String,
    handle: String,
    nft: String,
    num_years: u64,
    refund_address: Option<String>,
    secret: Option<String>,
    expire_time: Option<u64>,
    refund: bool,
    testnet11: bool,
    payment_asset_id_str: String,
    payment_cat_base_price_str: String,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    if refund {
        println!("Sorry to hear your re-registration attempt didn't go as planned :(");
    }
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let nft_launcher_id = Address::decode(&nft)?.puzzle_hash;
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
        || registry.info.state.expired_handle_pricing_puzzle_hash
            != XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(
                payment_cat_base_price,
                1000,
            )
            .into()
    {
        if !refund {
            return Err(CliError::Custom(
                "Given payment asset id & base price do not match the current registry's state."
                    .to_string(),
            ));
        }

        yes_no_prompt("Given payment asset id & base price do not match the current registry. Re-registration will NOT work unless the price singleton changes the registry's state. Continue at your own risk?")?;
    }

    print!("Fetching slot...");
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
    println!("done.");

    let expire_time = if let Some(et) = expire_time {
        et
    } else {
        get_last_onchain_timestamp(&cli).await?
            + registry.info.constants.relative_block_height as u64 * 18
    };
    println!("Using expire time: {}", expire_time);

    let pricing_puzzle = XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
        &mut ctx,
        payment_cat_base_price,
        1000,
    )?;
    let payment_cat_amount = pricing_puzzle.clone().get_price(
        &mut ctx,
        handle.clone(),
        slot.info.value.expiration,
        expire_time,
        num_years,
    )? as u64;

    println!("Handle: {}", handle);
    println!(
        "Payment CAT amount: {:.3}",
        payment_cat_amount as f64 / 1000.0
    );
    println!("Fee: {} XCH", fee_str);

    let secret = if let Some(s) = secret {
        hex_string_to_bytes32(&s)?
    } else {
        let mut s = [0u8; 32];
        getrandom::fill(&mut s).map_err(|_| {
            DriverError::Custom("Failed to generate new 32-byte secret".to_string())
        })?;
        let s: Bytes32 = s.into();

        println!("Registration secret (SAVE THIS): {}", hex::encode(s));
        s
    };

    let pricing_solution = XchandlesExponentialPremiumRenewPuzzleSolution {
        buy_time: expire_time,
        pricing_program_solution: XchandlesFactorPricingSolution {
            current_expiration: slot.info.value.expiration,
            handle: handle.clone(),
            num_years,
        },
    };
    let precommit_coin_value = XchandlesPrecommitValue::for_normal_registration(
        payment_asset_id.tree_hash(),
        XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(payment_cat_base_price, 1000),
        pricing_solution.tree_hash(),
        secret,
        handle.clone(),
        expire_time,
        nft_launcher_id,
        nft_launcher_id,
    );

    let refund_address = if let Some(provided_refund_address) = refund_address {
        provided_refund_address
    } else {
        let derivation_resp = sage
            .get_derivations(GetDerivations {
                hardened: false,
                offset: 0,
                limit: 1,
            })
            .await?;
        derivation_resp.derivations[0].address.clone()
    };
    let refund_puzzle_hash = Address::decode(&refund_address)?.puzzle_hash;
    println!("Refund address: {}", refund_address);

    let precommit_inner_puzzle_hash = PrecommitLayer::<()>::puzzle_hash(
        SingletonStruct::new(registry.info.constants.launcher_id)
            .tree_hash()
            .into(),
        registry.info.constants.relative_block_height,
        registry.info.constants.precommit_payout_puzzle_hash,
        refund_puzzle_hash,
        precommit_coin_value.tree_hash(),
    );

    let precomit_puzzle_hash =
        CatArgs::curry_tree_hash(payment_asset_id, precommit_inner_puzzle_hash);

    let Some(potential_precommit_coin_records) = cli
        .get_coin_records_by_hint(precommit_inner_puzzle_hash.into(), None, None, Some(false))
        .await?
        .coin_records
    else {
        return Err(CliError::Custom(
            "Could not check whether precommit coin exists".to_string(),
        ));
    };

    let precommit_coin_record = potential_precommit_coin_records.iter().find(|cr| {
        cr.coin.puzzle_hash == precomit_puzzle_hash.into() && cr.coin.amount == payment_cat_amount
    });

    if let Some(precommit_coin_record) = precommit_coin_record {
        let target_block_height = precommit_coin_record.confirmed_block_index
            + registry.info.constants.relative_block_height
            + registry.info.constants.relative_block_height / 4;
        println!(
        "Precommitment coin found! It was created at block #{}; target spendable block height is #{}",
        precommit_coin_record.confirmed_block_index, target_block_height
    );

        loop {
            let resp = cli.get_blockchain_state().await?;
            let Some(blockchain_state) = resp.blockchain_state else {
                eprintln!("Failed to get blockchain state - aborting...");
                return Ok(());
            };

            if blockchain_state.peak.height >= target_block_height {
                break;
            }

            println!(
                "Latest block is #{}; waiting for {} more blocks...",
                blockchain_state.peak.height,
                target_block_height - blockchain_state.peak.height
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }

        println!("Precommitment coin is now spendable! Fetching its lineage proof...");
        let Some(parent_coin_spend) = cli
            .get_puzzle_and_solution(
                precommit_coin_record.coin.parent_coin_info,
                Some(precommit_coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
        else {
            return Err(CliError::CoinNotSpent(
                precommit_coin_record.coin.parent_coin_info,
            ));
        };

        let parent_puzzle = node_from_bytes(&mut ctx, &parent_coin_spend.puzzle_reveal)?;
        let parent_cat_layer = Puzzle::parse(&ctx, parent_puzzle);
        let Some(parent_cat_layer) = CatLayer::<NodePtr>::parse_puzzle(&ctx, parent_cat_layer)?
        else {
            eprintln!(
                "Failed to parse CAT puzzle for coin {} - aborting...",
                hex::encode(precommit_coin_record.coin.coin_id())
            );
            return Ok(());
        };
        let parent_inner_puzzle_hash = ctx.tree_hash(parent_cat_layer.inner_puzzle);
        let lineage_proof = LineageProof {
            parent_parent_coin_info: parent_coin_spend.coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_inner_puzzle_hash.into(),
            parent_amount: parent_coin_spend.coin.amount,
        };

        let precommit_coin = PrecommitCoin::new(
            &mut ctx,
            precommit_coin_record.coin.parent_coin_info,
            lineage_proof,
            payment_asset_id,
            SingletonStruct::new(launcher_id).tree_hash().into(),
            registry.info.constants.relative_block_height,
            registry.info.constants.precommit_payout_puzzle_hash,
            refund_puzzle_hash,
            precommit_coin_value,
            payment_cat_amount,
        )?;

        println!("A one-sided offer will be created; it will consume:");
        println!("  - 1 mojo");
        println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
        yes_no_prompt("Proceed?")?;

        let offer_resp = sage
            .make_offer(MakeOffer {
                requested_assets: Assets {
                    xch: Amount::u64(0),
                    cats: vec![],
                    nfts: vec![],
                },
                offered_assets: Assets {
                    xch: Amount::u64(1),
                    cats: vec![],
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
        let offer =
            parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, None)?;
        offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

        let sec_conds = if refund {
            let slot: Option<Slot<XchandlesSlotValue>> =
                if DefaultCatMakerArgs::curry_tree_hash(payment_asset_id.tree_hash().into())
                    == registry.info.state.cat_maker_puzzle_hash.into()
                    && registry.info.state.pricing_puzzle_hash
                        == XchandlesFactorPricingPuzzleArgs::curry_tree_hash(payment_cat_base_price)
                            .into()
                    && registry.info.state.expired_handle_pricing_puzzle_hash
                        == XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(
                            payment_cat_base_price,
                            1000,
                        )
                        .into()
                {
                    Some(slot)
                } else {
                    None
                };

            let precommitted_pricing_puzzle = pricing_puzzle.get_puzzle(&mut ctx)?;
            let precommitted_pricing_solution = ctx.alloc(&pricing_solution)?;
            registry
                .new_action::<XchandlesRefundAction>()
                .spend(
                    &mut ctx,
                    &mut registry,
                    precommit_coin,
                    precommitted_pricing_puzzle,
                    precommitted_pricing_solution,
                    slot,
                )?
                .0
                .reserve_fee(1)
        } else {
            registry
                .new_action::<XchandlesExpireAction>()
                .spend(
                    &mut ctx,
                    &mut registry,
                    slot,
                    num_years,
                    payment_cat_base_price,
                    precommit_coin,
                )?
                .0
        };

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

        return Ok(());
    }
    if refund {
        return Err(CliError::Custom(
            "Precommitment coin not found but --refund was provided".to_string(),
        ));
    }

    println!(
        "Registered handle hash: {}",
        hex::encode(handle.tree_hash())
    );

    println!("The registration will be controlled by the following NFT:");
    println!("  {}", nft);

    println!("\nCONFIRM THE NFT IS CORRECT - HANDLE CANNOT BE RECOVERED AFTER REGISTRATION\n");

    println!(
        "Your wallet will send {} mojos of the payment asset with a fee of {} XCH ({} mojos)",
        payment_cat_amount, fee_str, fee
    );

    yes_no_prompt("Continue with registration?")?;

    let precommit_coin_address =
        Address::new(precommit_inner_puzzle_hash.into(), get_prefix(testnet11)).encode()?;
    let send_resp = sage
        .send_cat(SendCat {
            asset_id: hex::encode(payment_asset_id),
            address: precommit_coin_address,
            amount: Amount::Number(payment_cat_amount),
            fee: Amount::Number(fee),
            memos: None,
            auto_submit: true,
            include_hint: true,
        })
        .await?;
    println!("Transaction sent.");

    wait_for_coin(
        &cli,
        hex_string_to_bytes32(&send_resp.summary.inputs[0].coin_id)?,
        true,
    )
    .await?;
    println!("Confirmed!");

    println!(
        "To spend the precommitment coin, run the same command again with two more arguments:"
    );
    println!(
        "  --secret {} --expire-time {}",
        hex::encode(secret),
        expire_time
    );

    Ok(())
}

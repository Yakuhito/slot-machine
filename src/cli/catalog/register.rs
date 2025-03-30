use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, SpendBundle},
    puzzles::{cat::CatArgs, singleton::SingletonStruct, LineageProof},
};
use chia_wallet_sdk::{
    decode_address, encode_address, CatLayer, ChiaRpcClient, Layer, Offer, Puzzle, Spend,
    SpendContext,
};
use clvmr::{serde::node_from_bytes, NodePtr};
use sage_api::{Amount, Assets, GetDerivations, MakeOffer, SendCat};

use crate::{
    get_coinset_client, get_constants, get_prefix, hex_string_to_bytes, hex_string_to_bytes32,
    new_sk, parse_amount, parse_one_sided_offer, print_spend_bundle_to_file, spend_security_coin,
    sync_catalog, wait_for_coin, yes_no_prompt, CatNftMetadata, CatalogPrecommitValue,
    CatalogRefundAction, CatalogRegisterAction, CatalogRegistryConstants, CatalogSlotValue,
    CliError, Db, DefaultCatMakerArgs, PrecommitCoin, PrecommitLayer, SageClient, Slot,
};

#[allow(clippy::too_many_arguments)]
pub fn initial_metadata_from_arguments(
    ticker: String,
    name: String,
    description: String,
    precision: u8,
    image_uris_str: String,
    image_hash_str: String,
    metadata_uris_str: String,
    metadata_hash_str: Option<String>,
    license_uris_str: String,
    license_hash_str: Option<String>,
) -> Result<CatNftMetadata, CliError> {
    Ok(CatNftMetadata {
        ticker,
        name,
        description,
        precision,
        image_uris: image_uris_str.split(',').map(|s| s.to_string()).collect(),
        image_hash: hex_string_to_bytes32(&image_hash_str)?,
        metadata_uris: metadata_uris_str
            .split(',')
            .map(|s| s.to_string())
            .collect(),
        metadata_hash: if let Some(metadata_hash_str) = metadata_hash_str {
            Some(hex_string_to_bytes32(&metadata_hash_str)?)
        } else {
            None
        },
        license_uris: license_uris_str.split(',').map(|s| s.to_string()).collect(),
        license_hash: if let Some(license_hash_str) = license_hash_str {
            Some(hex_string_to_bytes32(&license_hash_str)?)
        } else {
            None
        },
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn catalog_register(
    tail_reveal_str: String,
    ticker: String,
    name: String,
    description: String,
    precision: u8,
    image_uris_str: String,
    image_hash_str: String,
    metadata_uris_str: String,
    metadata_hash_str: Option<String>,
    license_uris_str: String,
    license_hash_str: Option<String>,
    recipient_address: Option<String>,
    refund: bool,
    testnet11: bool,
    local: bool,
    payment_asset_id_str: String,
    payment_cat_amount_str: Option<String>,
    fee_str: String,
) -> Result<(), CliError> {
    if refund {
        println!(
            "Ouch - it sucks when things go wrong. Thankfully, the refund path is available to handle a lot of those cases :)"
        );
    } else {
        println!("Welcome to the CATalog registration process, issuer!");
    }

    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);
    let catalog_constants = CatalogRegistryConstants::get(testnet11);
    let sage = SageClient::new()?;

    let fee = parse_amount(fee_str.clone(), false)?;

    let initial_metadata = initial_metadata_from_arguments(
        ticker,
        name,
        description,
        precision,
        image_uris_str,
        image_hash_str,
        metadata_uris_str,
        metadata_hash_str,
        license_uris_str,
        license_hash_str,
    )?;

    let payment_asset_id = hex_string_to_bytes32(&payment_asset_id_str)?;

    print!("First, let's sync CATalog... ");
    if local {
        let mut db = Db::new(false).await?;
        sync_catalog(&cli, &mut db, &mut ctx, catalog_constants).await?
    } esle {
        quick_sync_catalog(&cli, &mut ctx, catalog_constants).await?
    }
    println!("done.");

    let recipient_address = if let Some(provided_recipient_address) = recipient_address {
        provided_recipient_address
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

    let tail_ptr = node_from_bytes(&mut ctx.allocator, &hex_string_to_bytes(&tail_reveal_str)?)?;
    let registered_asset_id: Bytes32 = ctx.tree_hash(tail_ptr).into();

    if !refund
        && DefaultCatMakerArgs::curry_tree_hash(payment_asset_id.tree_hash().into())
            != catalog.info.state.cat_maker_puzzle_hash.into()
    {
        yes_no_prompt("CAT maker puzzle hash doesn't correspond to the given payment asset ID. Registration will NOT work unless the price singleton changes the registry's state. Continue at your own risk?")?;
    }

    let mut payment_cat_amount = catalog.info.state.registration_price;
    if let Some(payment_cat_amount_str) = payment_cat_amount_str {
        let parsed_payment_cat_amount = parse_amount(payment_cat_amount_str, true)?;
        if parsed_payment_cat_amount != payment_cat_amount {
            if !refund {
                yes_no_prompt("Payment CAT amount is different from the specified registration price. Registration will likely fail. Continue at your own risk?")?;
            }
            payment_cat_amount = parsed_payment_cat_amount;
        }
    }

    let recipient_puzzle_hash = Bytes32::new(decode_address(&recipient_address)?.0);

    let initial_nft_puzzle_ptr = CatalogPrecommitValue::<()>::initial_inner_puzzle(
        &mut ctx,
        recipient_puzzle_hash,
        initial_metadata.clone(),
    )?;

    let precommit_value = CatalogPrecommitValue::with_default_cat_maker(
        payment_asset_id.tree_hash(),
        ctx.tree_hash(initial_nft_puzzle_ptr).into(),
        tail_ptr,
    );
    let precommit_value_ptr = ctx.alloc(&precommit_value)?;

    let precommit_inner_puzzle_hash = PrecommitLayer::<()>::puzzle_hash(
        SingletonStruct::new(catalog_constants.launcher_id)
            .tree_hash()
            .into(),
        catalog_constants.relative_block_height,
        catalog_constants.precommit_payout_puzzle_hash,
        recipient_puzzle_hash,
        ctx.tree_hash(precommit_value_ptr),
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
            + catalog_constants.relative_block_height
            + 7;
        println!(
            "Precommitment coin found! It was created at block #{}; target spendable block height is #{}",
            target_block_height, target_block_height
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

        let parent_puzzle = node_from_bytes(&mut ctx.allocator, &parent_coin_spend.puzzle_reveal)?;
        let Some(parent_cat_layer) = CatLayer::<NodePtr>::parse_puzzle(
            &ctx.allocator,
            Puzzle::parse(&ctx.allocator, parent_puzzle),
        )?
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
            SingletonStruct::new(catalog_constants.launcher_id)
                .tree_hash()
                .into(),
            catalog_constants.relative_block_height,
            catalog_constants.precommit_payout_puzzle_hash,
            recipient_puzzle_hash,
            precommit_value,
            payment_cat_amount,
        )?;

        println!("A one-sided offer will be created; it will consume:");
        if refund {
            println!("  - 1 mojo");
        } else {
            println!("  - 1 mojo for minting the CAT NFT");
        }
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
            parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, false)?;
        offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

        let sec_conds = if refund {
            let slot: Option<Slot<CatalogSlotValue>> = if DefaultCatMakerArgs::curry_tree_hash(
                payment_asset_id.tree_hash().into(),
            ) == catalog
                .info
                .state
                .cat_maker_puzzle_hash
                .into()
                && payment_cat_amount == catalog.info.state.registration_price
            {
                let Some(slot_value_hash) = db
                    .get_catalog_indexed_slot_value(registered_asset_id)
                    .await?
                else {
                    return Err(CliError::Custom(
                                "Refund not available - precommit uses right CAT & amount & tries to register a new CAT".to_string(),
                            ));
                };

                Some(
                    db.get_slot::<CatalogSlotValue>(
                        &mut ctx.allocator,
                        catalog_constants.launcher_id,
                        0,
                        slot_value_hash,
                        0,
                    )
                    .await?
                    .unwrap(),
                )
            } else {
                None
            };

            catalog
                .new_action::<CatalogRefundAction>()
                .spend(
                    &mut ctx,
                    &mut catalog,
                    registered_asset_id,
                    if let Some(slot) = slot {
                        slot.info.value.neighbors.tree_hash().into()
                    } else {
                        Bytes32::default()
                    },
                    precommit_coin,
                    slot,
                )?
                .reserve_fee(1)
        } else {
            let (left_slot, right_slot) = db
                .get_catalog_neighbors(
                    &mut ctx.allocator,
                    catalog_constants.launcher_id,
                    registered_asset_id,
                )
                .await?;

            catalog
                .new_action::<CatalogRegisterAction>()
                .spend(
                    &mut ctx,
                    &mut catalog,
                    registered_asset_id,
                    left_slot,
                    right_slot,
                    precommit_coin,
                    Spend {
                        puzzle: initial_nft_puzzle_ptr,
                        solution: NodePtr::NIL,
                    },
                )?
                .0
        };

        let _new_catalog = catalog.finish_spend(&mut ctx)?;

        let security_coin_sig = spend_security_coin(
            &mut ctx,
            offer.security_coin,
            offer.security_base_conditions.extend(sec_conds),
            &security_coin_sk,
            get_constants(testnet11),
        )?;

        let sb = SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig);

        println!("Submitting transaction...");
        print_spend_bundle_to_file(
            sb.coin_spends.clone(),
            sb.aggregated_signature.clone(),
            "sb.debug",
        );
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

    println!("Registered asset ID: {}", hex::encode(registered_asset_id));

    println!("Have one last look at the initial metadata:");
    initial_metadata.pretty_print("  ");

    println!("The NFT will be minted to the following address:");
    println!("  {}", recipient_address);

    println!("\nCONFIRM THE ADDRESS IS CORRECT - NFT CANNOT BE RECOVERED AFTER REGISTRATION\n");

    println!(
        "Your wallet will send {} mojos of the payment asset with a fee of {} XCH ({} mojos)",
        payment_cat_amount, fee_str, fee
    );

    yes_no_prompt("Continue with registration?")?;

    let precommit_coin_address =
        encode_address(precommit_inner_puzzle_hash.into(), &get_prefix(testnet11))?;
    let send_resp = sage
        .send_cat(SendCat {
            asset_id: hex::encode(payment_asset_id),
            address: precommit_coin_address,
            amount: Amount::Number(payment_cat_amount),
            fee: Amount::Number(fee),
            memos: vec![],
            auto_submit: true,
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

    println!("To spend the precommitment coin, run the same command again");

    Ok(())
}

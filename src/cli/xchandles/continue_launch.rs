use std::collections::{HashMap, HashSet};

use chia_protocol::{Bytes32, Coin, SpendBundle};
use chia_puzzle_types::{
    cat::CatArgs, nft::NftMetadata, singleton::SingletonStruct, CoinProof, EveProof, LineageProof,
    Proof,
};
use chia_puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        create_security_coin, decode_offer, spend_security_coin, spend_settlement_cats, CatLayer,
        CatalogPrecommitValue, Launcher, Layer, Nft, NftInfo, Offer, PrecommitCoin, PrecommitLayer,
        Puzzle, SingleCatSpend, SingletonInfo, Spend, SpendContext, XchandlesPrecommitValue,
        XchandlesRegisterAction,
    },
    types::{
        puzzles::{
            XchandlesFactorPricingPuzzleArgs, XchandlesPricingSolution, ANY_METADATA_UPDATER_HASH,
        },
        Conditions, Mod, MAINNET_CONSTANTS, TESTNET11_CONSTANTS,
    },
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvm_utils::ToTreeHash;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    assets_xch_and_cat, assets_xch_only, confirm_pushed_transaction, encode_nft,
    get_last_onchain_timestamp, get_prefix, hex_string_to_bytes32, load_xchandles_premine_csv,
    no_assets, parse_amount, sync_xchandles, yes_no_prompt, CliError, Db, SageClient,
    XchandlesPremineRecord,
};

fn precommit_value_for_handle(
    handle: &XchandlesPremineRecord,
    handle_nft_launcher_id: Bytes32,
    payment_asset_id: Bytes32,
    start_time: u64,
    registration_period: u64,
) -> Result<XchandlesPrecommitValue, CliError> {
    Ok(XchandlesPrecommitValue::for_normal_registration(
        payment_asset_id.tree_hash(),
        XchandlesFactorPricingPuzzleArgs {
            base_price: 1,
            registration_period,
        }
        .curry_tree_hash(),
        &XchandlesPricingSolution {
            buy_time: start_time,
            current_expiration: 0,
            handle: handle.handle.clone(),
            num_periods: 1,
        },
        handle.handle.clone(),
        Bytes32::default(),
        handle_nft_launcher_id,
        handle_nft_launcher_id,
    ))
}

fn metadata_for_handle_nft(
    handle_info: XchandlesPremineRecord,
    edition_number: u64,
    edition_total: u64,
) -> NftMetadata {
    NftMetadata {
        edition_number,
        edition_total,
        data_uris: handle_info.image_uris,
        data_hash: Some(handle_info.image_hash),
        metadata_uris: handle_info.metadata_uris,
        metadata_hash: Some(handle_info.metadata_hash),
        license_uris: handle_info.license_uris,
        license_hash: Some(handle_info.license_hash),
    }
}

#[allow(clippy::too_many_arguments)]
async fn eve_nft_for_handle(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    registry_launcher_id: Bytes32,
    handle: &XchandlesPremineRecord,
    handle_index: u64,
    total_handles: u64,
    royalty_puzzle_hash: Bytes32,
    royalty_basis_points: u16,
    eve_nft_temp_inner_ph: Bytes32,
    include_spent_coins: bool,
) -> Result<Option<Nft>, CliError> {
    let hint = (registry_launcher_id, handle.handle.tree_hash())
        .tree_hash()
        .into();

    let metadata = metadata_for_handle_nft(handle.clone(), handle_index + 1, total_handles);
    let metadata = ctx.alloc_hashed(&metadata)?;

    let Some(mut possible_launcher_records) = client
        .get_coin_records_by_hint(hint, None, None, Some(true), None)
        .await?
        .coin_records
    else {
        return Err(CliError::Custom(
            "Failed to get possible launchers - aborting...".to_string(),
        ));
    };

    possible_launcher_records.retain(|cr| {
        cr.coin.amount % 2 == 0 && cr.coin.puzzle_hash == SINGLETON_LAUNCHER_HASH.into()
    });

    let expected_coins: Vec<(NftInfo, Coin)> = possible_launcher_records
        .iter()
        .map(|possible_launcher_record| {
            let nft_info = NftInfo::new(
                possible_launcher_record.coin.coin_id(),
                metadata,
                ANY_METADATA_UPDATER_HASH.into(),
                None,
                royalty_puzzle_hash,
                royalty_basis_points,
                eve_nft_temp_inner_ph,
            );
            let eve_nft_ph = nft_info.puzzle_hash();

            (
                nft_info,
                Coin::new(
                    possible_launcher_record.coin.coin_id(),
                    eve_nft_ph.into(),
                    1,
                ),
            )
        })
        .collect();

    let expected_coin_ids: Vec<Bytes32> = expected_coins.iter().map(|c| c.1.coin_id()).collect();

    let Some(coin_records) = client
        .get_coin_records_by_names(
            expected_coin_ids,
            None,
            None,
            Some(include_spent_coins),
            None,
        )
        .await?
        .coin_records
    else {
        return Ok(None);
    };

    if coin_records.is_empty() {
        return Ok(None);
    }

    let Some(found_index) = expected_coins
        .iter()
        .position(|(_, coin)| *coin == coin_records[0].coin)
    else {
        return Ok(None);
    };

    let proof = Proof::Eve(EveProof {
        parent_parent_coin_info: possible_launcher_records[found_index].coin.parent_coin_info,
        parent_amount: possible_launcher_records[found_index].coin.amount,
    });

    Ok(Some(Nft::new(
        coin_records[0].coin,
        proof,
        expected_coins[found_index].0,
    )))
}

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_continue_launch(
    launcher_id_str: String,
    payment_asset_id_str: String,
    royalty_address: String,
    royalty_basis_points: u16,
    handles_per_spend: usize,
    start_time: Option<u64>,
    registration_period: u64,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let royalty_puzzle_hash = Address::decode(&royalty_address)?.puzzle_hash;
    println!("Time to unroll an XCHandles registry! Yee-haw!");

    let premine_csv_filename = if testnet11 {
        "xchandles_premine_testnet11.csv"
    } else {
        "xchandles_premine_mainnet.csv"
    };

    println!("Loading premine data from '{}'...", premine_csv_filename);
    let handles_to_launch = load_xchandles_premine_csv(premine_csv_filename)?;

    println!("Initializing Chia RPC client...");
    let client = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };

    println!("Opening database...");
    let mut db = Db::new(false).await?;
    let mut ctx = SpendContext::new();

    println!("Syncing XCHandles registry...");

    let mut registry = sync_xchandles(&client, &mut db, &mut ctx, launcher_id).await?;
    println!(
        "Latest XCHandles registry coin id: {}",
        registry.coin.coin_id()
    );

    println!("Finding last registered handle from list...");
    let mut i = 0;
    while i < handles_to_launch.len() {
        let handle = &handles_to_launch[i];
        let resp = db
            .get_xchandles_indexed_slot_value(launcher_id, handle.handle.tree_hash().into())
            .await?;
        if resp.is_none() {
            break;
        }

        i += 1;
    }

    if i == handles_to_launch.len() {
        eprintln!("All handles have already been registered - nothing to do!");
        return Ok(());
    }

    let payment_asset_id = Bytes32::new(hex_string_to_bytes32(&payment_asset_id_str)?.into());

    let sage = SageClient::new()?;
    let fee = parse_amount(&fee_str, false)?;

    let derivation_resp = sage.get_derivations(false, 0, 1).await?;
    println!(
        "Active wallet address: {})",
        derivation_resp.derivations[0].address
    );

    let eve_nft_temp_inner_ph =
        Address::decode(&derivation_resp.derivations[0].address)?.puzzle_hash;

    // Make sure this is always rounded down to a day
    let constants = registry.info.constants;
    let start_time = if let Some(st) = start_time {
        st
    } else {
        get_last_onchain_timestamp(&client).await? / 8640 * 8640
    };
    println!("Using start time: {}", start_time);

    if i == 0 {
        println!("No handles registered yet - looking for precommitment coins...");

        let mut i = 0;
        while i < handles_to_launch.len() {
            let Some(_eve_nft) = eve_nft_for_handle(
                &mut ctx,
                &client,
                registry.info.constants.launcher_id,
                &handles_to_launch[i],
                (i) as u64,
                handles_to_launch.len() as u64,
                royalty_puzzle_hash,
                royalty_basis_points,
                eve_nft_temp_inner_ph,
                false,
            )
            .await?
            else {
                break;
            };

            i += 1;
        }

        if i != handles_to_launch.len() {
            // there are unlaunched precommitment coins, launch those first and exit

            println!(
                "Some precommitment coins were not launched yet - they correspond to these handles:"
            );

            let mut j = i;
            while j < handles_to_launch.len() && j - i < handles_per_spend {
                println!(
                    "  handle: {:}, recipient: {:}, image_uris: {:?}",
                    handles_to_launch[j].handle,
                    Address::new(handles_to_launch[j].recipient, get_prefix(testnet11)).encode()?,
                    handles_to_launch[j].image_uris.join("|")
                );
                j += 1;
            }

            // (inner puzzle hash, amount)
            let mut handles_payment_total = 0;
            let mut handle_infos = Vec::with_capacity(handles_per_spend);
            j = i;
            while j < handles_to_launch.len() && j - i < handles_per_spend {
                let handle_reg_price =
                    XchandlesFactorPricingPuzzleArgs::get_price(1, &handles_to_launch[j].handle, 1);

                handles_payment_total += handle_reg_price;
                handle_infos.push(handles_to_launch[j].clone());

                j += 1;
            }

            println!(
                "NFTs will be minted with royalty address: {}",
                royalty_address
            );
            println!("Royalty basis points: {}", royalty_basis_points);

            println!("A one-sided offer will be created; it will consume:");
            println!(
                "  - {} payment CAT mojos for creating precommitment coins",
                handles_payment_total,
            );
            println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
            println!(
                "  - {} mojo(s) to mint the eve NFTs for handles",
                handle_infos.len()
            );
            println!("Eve NFTs will be temporarily stored in your wallet before being transferred to the final recipient.");
            yes_no_prompt("Proceed?")?;

            let offer_resp = sage
                .make_offer(
                    no_assets(),
                    assets_xch_and_cat(
                        handle_infos.len() as u64,
                        payment_asset_id_str,
                        handles_payment_total,
                    ),
                    fee,
                    None,
                    None,
                    false,
                )
                .await?;
            println!("Offer with id {} generated.", offer_resp.offer_id);

            // Parse one-sided offer
            let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
            let (security_coin_sk, security_coin) =
                create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

            let mut security_coin_conditions = Conditions::new();
            let mut cat_creator_conds = Conditions::new();

            for (index, handle_info) in handle_infos.into_iter().enumerate() {
                // Launch eve NFT
                let launcher_amount = (index * 2) as u64;
                let launcher = Launcher::new(security_coin.coin_id(), launcher_amount);
                let launcher_id = launcher.coin().coin_id();

                println!(
                    "  Handle {} will be represented by NFT {}",
                    handle_info.handle,
                    encode_nft(launcher.coin().coin_id())?
                );

                let metadata = metadata_for_handle_nft(
                    handle_info.clone(),
                    (i + index) as u64,
                    handles_to_launch.len() as u64,
                );
                let metadata = ctx.alloc_hashed(&metadata)?;

                let (sec_conditions, _eve_nft) = launcher.mint_eve_nft(
                    &mut ctx,
                    eve_nft_temp_inner_ph,
                    metadata,
                    ANY_METADATA_UPDATER_HASH.into(),
                    royalty_puzzle_hash,
                    royalty_basis_points,
                )?;

                // unique hint per deployment to minimize confusions
                let hint: Bytes32 = (
                    registry.info.constants.launcher_id,
                    handle_info.handle.tree_hash(),
                )
                    .tree_hash()
                    .into();
                let hint = ctx.hint(hint)?;

                security_coin_conditions = security_coin_conditions
                    .create_coin(SINGLETON_LAUNCHER_HASH.into(), launcher_amount, hint)
                    .extend(sec_conditions);

                // Create precommitment coin
                let handle_reg_price =
                    XchandlesFactorPricingPuzzleArgs::get_price(1, &handle_info.handle, 1);

                let precommit_value = precommit_value_for_handle(
                    &handle_info,
                    launcher_id,
                    payment_asset_id,
                    start_time,
                    registration_period,
                )?;
                let precommit_value_ptr = ctx.alloc(&precommit_value)?;
                let precommit_value_hash = ctx.tree_hash(precommit_value_ptr);

                let inner_puzzle_hash = PrecommitLayer::<XchandlesPrecommitValue>::puzzle_hash(
                    SingletonStruct::new(constants.launcher_id)
                        .tree_hash()
                        .into(),
                    constants.relative_block_height,
                    constants.precommit_payout_puzzle_hash,
                    Bytes32::default(),
                    precommit_value_hash,
                );

                cat_creator_conds = cat_creator_conds.create_coin(
                    inner_puzzle_hash.into(),
                    handle_reg_price,
                    ctx.hint(inner_puzzle_hash.into())?,
                );
            }

            // Spend offered CAT
            let cat_destination_puzzle = ctx.alloc_hashed(&clvm_quote!(cat_creator_conds))?;
            let cat_destination_puzzle_hash: Bytes32 = cat_destination_puzzle.tree_hash().into();

            let (created_cats, cat_assert) = spend_settlement_cats(
                &mut ctx,
                &offer,
                payment_asset_id,
                launcher_id,
                &[(cat_destination_puzzle_hash, handles_payment_total)],
            )?;

            let created_cat = created_cats[0];
            security_coin_conditions =
                cat_assert.assert_concurrent_spend(created_cat.coin.coin_id());

            // Spend security coin
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

            // Spend CAT
            created_cat.spend(
                &mut ctx,
                SingleCatSpend {
                    next_coin_proof: CoinProof {
                        parent_coin_info: created_cat.coin.parent_coin_info,
                        inner_puzzle_hash: created_cat.info.p2_puzzle_hash,
                        amount: created_cat.coin.amount,
                    },
                    prev_coin_id: created_cat.coin.coin_id(),
                    prev_subtotal: 0,
                    extra_delta: 0,
                    p2_spend: Spend::new(cat_destination_puzzle.ptr(), NodePtr::NIL),
                    revoke: false,
                },
            )?;

            // Build spend bundle
            let sb = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

            println!("Submitting transaction...");
            let resp = client.push_tx(sb).await?;

            if confirm_pushed_transaction(&client, &resp, security_coin.coin_id(), true).await? {
                println!("Confirmed!");
            }

            return Ok(());
        } else {
            println!("All precommitment coins have already been created :)");
        }
    }

    let mut handles = Vec::with_capacity(handles_per_spend);
    while i < handles_to_launch.len() && handles.len() < handles_per_spend {
        handles.push(handles_to_launch[i].clone());
        i += 1;
    }

    println!(
        "These handles will be launched (total number={}):",
        handles.len()
    );
    for handle in &handles {
        println!(
            "  handle: {:}, recipient: {:}, image_uris: {:?}",
            handle.handle,
            Address::new(handle.recipient, get_prefix(testnet11)).encode()?,
            handle.image_uris.join("|")
        );
    }

    println!("Fetching eve NFTs for handles...");

    let mut eve_nfts = Vec::with_capacity(handles.len());

    for (index, handle) in handles.iter().enumerate() {
        let Some(eve_nft) = eve_nft_for_handle(
            &mut ctx,
            &client,
            registry.info.constants.launcher_id,
            handle,
            (i + index) as u64,
            handles.len() as u64,
            royalty_puzzle_hash,
            royalty_basis_points,
            eve_nft_temp_inner_ph,
            false,
        )
        .await?
        else {
            return Err(CliError::Custom(format!(
                "No valid eve NFT found for handle {} - aborting...",
                handle.handle,
            )));
        };

        eve_nfts.push(eve_nft);
    }

    // check if precommitment coins are available and have the appropriate age
    println!("Checking precommitment coins...");
    let precommit_values = handles
        .iter()
        .zip(eve_nfts.iter())
        .map(|(handle, eve_nft)| {
            precommit_value_for_handle(
                handle,
                eve_nft.info.launcher_id,
                payment_asset_id,
                start_time,
                registration_period,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let precommit_puzzle_hashes = precommit_values
        .iter()
        .map(|pv| {
            let precommit_value_ptr = ctx.alloc(pv)?;
            let precommit_value_hash = ctx.tree_hash(precommit_value_ptr);
            let inner_ph = PrecommitLayer::<CatalogPrecommitValue>::puzzle_hash(
                SingletonStruct::new(constants.launcher_id)
                    .tree_hash()
                    .into(),
                constants.relative_block_height,
                constants.precommit_payout_puzzle_hash,
                Bytes32::default(),
                precommit_value_hash,
            );

            Ok::<Bytes32, CliError>(CatArgs::curry_tree_hash(payment_asset_id, inner_ph).into())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let expected_records = precommit_puzzle_hashes.len();
    let phes_resp = client
        .get_coin_records_by_puzzle_hashes(
            precommit_puzzle_hashes.clone(),
            None,
            None,
            Some(false),
            None,
        )
        .await?;
    let Some(precommit_coin_records) = phes_resp.coin_records else {
        eprintln!("Failed to get precommitment coin records - aborting...");
        return Ok(());
    };
    if precommit_coin_records.len() < expected_records {
        eprintln!("Received too few records - aborting...");
        return Ok(());
    }

    let max_confirmed_block_index = precommit_coin_records
        .iter()
        .map(|cr| cr.confirmed_block_index)
        .max()
        .unwrap_or(0);

    let target_block_height = max_confirmed_block_index
        + constants.relative_block_height
        + constants.relative_block_height / 4;
    println!(
        "Last precommitment coin created at block #{}; target spendable block height is #{}",
        max_confirmed_block_index, target_block_height
    );

    loop {
        let resp = client.get_blockchain_state().await?;
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

    println!("Precommitment coins are now spendable!");

    print!("Fetching parent records for lineage proofs... ");

    let parent_ids = precommit_coin_records
        .iter()
        .map(|cr| cr.coin.parent_coin_info)
        .collect::<HashSet<Bytes32>>();

    let expected_records = parent_ids.len();
    let parent_records_resp = client
        .get_coin_records_by_names(
            parent_ids.into_iter().collect(),
            None,
            None,
            Some(true),
            None,
        )
        .await?;
    let Some(parent_records) = parent_records_resp.coin_records else {
        eprintln!("Failed to get parent records - aborting...");
        return Ok(());
    };
    if parent_records.len() < expected_records {
        eprintln!("Received too few records - aborting...");
        return Ok(());
    }

    let mut lineage_proofs: HashMap<Bytes32, LineageProof> = HashMap::new();
    for record in parent_records {
        let puzzle_and_solution_resp = client
            .get_puzzle_and_solution(record.coin.coin_id(), Some(record.spent_block_index))
            .await?;
        let Some(coin_spend) = puzzle_and_solution_resp.coin_solution else {
            eprintln!(
                "Failed to get puzzle and solution for coin {} - aborting...",
                hex::encode(record.coin.coin_id())
            );
            return Ok(());
        };

        let puzzle = node_from_bytes(&mut ctx, &coin_spend.puzzle_reveal)?;
        let layer = Puzzle::parse(&ctx, puzzle);
        let Some(layer) = CatLayer::<NodePtr>::parse_puzzle(&ctx, layer)? else {
            eprintln!(
                "Failed to parse CAT puzzle for coin {} - aborting...",
                hex::encode(record.coin.coin_id())
            );
            return Ok(());
        };
        let inner_puzzle_hash = ctx.tree_hash(layer.inner_puzzle);
        lineage_proofs.insert(
            record.coin.coin_id(),
            LineageProof {
                parent_parent_coin_info: record.coin.parent_coin_info,
                parent_inner_puzzle_hash: inner_puzzle_hash.into(),
                parent_amount: record.coin.amount,
            },
        );
    }

    println!("Done!");

    println!("A one-sided offer will be created; it will consume:");
    println!("  - 1 mojo for the sake of it");
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let mut security_coin_conditions = Conditions::new().reserve_fee(1);

    for (i, precommit_value) in precommit_values.iter().enumerate() {
        let precommit_ph = precommit_puzzle_hashes[i];
        let precommit_coin_record = precommit_coin_records
            .iter()
            .find(|cr| cr.coin.puzzle_hash == precommit_ph)
            .unwrap();

        let lineage_proof = lineage_proofs
            .get(&precommit_coin_record.coin.parent_coin_info)
            .unwrap();

        let handle_hash = precommit_value.handle.clone().tree_hash().into();

        let precommit_coin = PrecommitCoin::new(
            &mut ctx,
            precommit_coin_record.coin.parent_coin_info,
            *lineage_proof,
            payment_asset_id,
            SingletonStruct::new(constants.launcher_id)
                .tree_hash()
                .into(),
            constants.relative_block_height,
            constants.precommit_payout_puzzle_hash,
            Bytes32::default(),
            precommit_value.clone(),
            XchandlesFactorPricingPuzzleArgs::get_price(1, &precommit_value.handle, 1),
        )?;

        let (left_slot, right_slot) = db
            .get_xchandles_neighbors(&mut ctx, constants.launcher_id, handle_hash)
            .await?;

        let (left_slot, right_slot) = registry.actual_neigbors(handle_hash, left_slot, right_slot);

        let (register_conds, owner_message_conds, resolved_message_conds) =
            registry.new_action::<XchandlesRegisterAction>().spend(
                &mut ctx,
                &mut registry,
                left_slot,
                right_slot,
                &precommit_coin,
                1,
                registration_period,
                start_time,
                Bytes32::default(),
                Bytes32::default(),
            )?;

        let mut nft_conds = owner_message_conds;
        if let Some(resolved_message_conds) = resolved_message_conds {
            nft_conds = nft_conds.extend(resolved_message_conds);
        }

        todo!("Transfer eve NFT to intended owner, sign");
        // eve_nfts[i].transfer(&mut ctx, inner, p2_puzzle_hash, nft_conds);

        security_coin_conditions = security_coin_conditions.extend(register_conds);
    }

    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

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

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let resp = client.push_tx(sb).await?;

    if confirm_pushed_transaction(&client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

use std::collections::{HashMap, HashSet};

use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes32, SpendBundle},
    puzzles::{cat::CatArgs, singleton::SingletonStruct, CoinProof, LineageProof},
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        decode_offer, CatLayer, DriverError, Layer, Offer, Puzzle, SingleCatSpend, Spend,
        SpendContext,
    },
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};
use clvm_traits::clvm_quote;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    assets_xch_and_cat, assets_xch_only, create_security_coin, hex_string_to_bytes32,
    load_catalog_premine_csv, no_assets, parse_amount, spend_security_coin, spend_settlement_cats,
    sync_catalog, wait_for_coin, yes_no_prompt, CatNftMetadata, CatalogPrecommitValue,
    CatalogPremineRecord, CatalogRegisterAction, CatalogRegistryConstants, CliError, Db,
    PrecommitCoin, PrecommitLayer, SageClient,
};

pub fn initial_cat_inner_puzzle_ptr(
    ctx: &mut SpendContext,
    cat: &CatalogPremineRecord,
) -> Result<NodePtr, DriverError> {
    CatalogPrecommitValue::<()>::initial_inner_puzzle(
        ctx,
        cat.owner,
        CatNftMetadata {
            ticker: cat.code.clone(),
            name: cat.name.clone(),
            description: "".to_string(),
            precision: cat.precision,
            image_uris: cat.image_uris.clone(),
            image_hash: cat.image_hash,
            metadata_uris: vec![],
            metadata_hash: None,
            license_uris: vec![],
            license_hash: None,
        },
    )
}

fn precommit_value_for_cat(
    ctx: &mut SpendContext,
    cat: &CatalogPremineRecord,
    payment_asset_id: Bytes32,
) -> Result<CatalogPrecommitValue, CliError> {
    let tail_ptr = node_from_bytes(ctx, &cat.tail)?;
    let tail_hash = ctx.tree_hash(tail_ptr);
    if tail_hash != cat.asset_id.into() {
        eprintln!("CAT {} has a tail hash mismatch - aborting", cat.asset_id);
        return Err(CliError::Custom("TAIL hash mismatch".to_string()));
    }

    let initial_inner_puzzle_ptr = initial_cat_inner_puzzle_ptr(ctx, cat)?;

    Ok(CatalogPrecommitValue::with_default_cat_maker(
        payment_asset_id.tree_hash(),
        ctx.tree_hash(initial_inner_puzzle_ptr).into(),
        tail_ptr,
    ))
}

pub async fn catalog_continue_launch(
    payment_asset_id_str: String,
    cats_per_spend: usize,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    println!("Time to unroll a CATalog! Yee-haw!");

    let premine_csv_filename = if testnet11 {
        "catalog_premine_testnet11.csv"
    } else {
        "catalog_premine_mainnet.csv"
    };

    println!("Loading premine data from '{}'...", premine_csv_filename);
    let cats_to_launch = load_catalog_premine_csv(premine_csv_filename)?;

    println!("Initializing Chia RPC client...");
    let client = if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    };

    println!("Opening database...");
    let mut db = Db::new(false).await?;

    let constants = CatalogRegistryConstants::get(testnet11);
    if constants.price_singleton_launcher_id == Bytes32::default()
        || constants.launcher_id == Bytes32::default()
    {
        return Err(CliError::ConstantsNotSet);
    }

    println!("Syncing CATalog...");
    let mut ctx = SpendContext::new();

    let mut catalog = sync_catalog(&client, &mut db, &mut ctx, constants).await?;
    println!("Latest catalog coin id: {}", catalog.coin.coin_id());

    println!("Finding last registered CAT from list...");
    let mut i = 0;
    while i < cats_to_launch.len() {
        let cat = &cats_to_launch[i];
        let resp = db.get_catalog_indexed_slot_value(cat.asset_id).await?;
        if resp.is_none() {
            break;
        }

        i += 1;
    }

    if i == cats_to_launch.len() {
        eprintln!("All CATs have already been registered - nothing to do!");
        return Ok(());
    }

    let payment_asset_id = Bytes32::new(hex_string_to_bytes32(&payment_asset_id_str)?.into());

    let sage = SageClient::new()?;
    let fee = parse_amount(&fee_str, false)?;

    if i == 0 {
        println!("No CATs registered yet - looking for precommitment coins...");

        let inner_puzzle_hashes = cats_to_launch
            .iter()
            .map(|cat| {
                let precommit_value = precommit_value_for_cat(&mut ctx, cat, payment_asset_id)?;
                let precommit_value_ptr = ctx.alloc(&precommit_value)?;
                let precommit_value_hash = ctx.tree_hash(precommit_value_ptr);

                Ok::<TreeHash, CliError>(PrecommitLayer::<CatalogPrecommitValue>::puzzle_hash(
                    SingletonStruct::new(constants.launcher_id)
                        .tree_hash()
                        .into(),
                    constants.relative_block_height,
                    constants.precommit_payout_puzzle_hash,
                    Bytes32::default(),
                    precommit_value_hash,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut i = 0;
        while i < cats_to_launch.len() {
            let precommit_inner_puzzle = inner_puzzle_hashes[i];

            let precommit_puzzle =
                CatArgs::curry_tree_hash(payment_asset_id, precommit_inner_puzzle);

            let records_resp = client
                .get_coin_records_by_puzzle_hash(precommit_puzzle.into(), None, None, Some(true))
                .await?;
            let Some(records) = records_resp.coin_records else {
                break;
            };

            if records.is_empty() {
                break;
            }

            i += 1;
        }

        if i != cats_to_launch.len() {
            // there are unlaunched precommitment coins, launch those first and exit

            println!(
                "Some precommitment coins were not launched yet - they correspond to these CATs:"
            );

            let mut j = i;
            while j < cats_to_launch.len() && j - i < cats_per_spend {
                println!(
                    "  code: {:?}, name: {:?}",
                    cats_to_launch[j].code, cats_to_launch[j].name
                );
                j += 1;
            }

            let mut precommitment_inner_puzzle_hashes_to_launch =
                Vec::with_capacity(cats_per_spend);
            j = i;
            while j < cats_to_launch.len() && j - i < cats_per_spend {
                precommitment_inner_puzzle_hashes_to_launch.push(inner_puzzle_hashes[j]);
                j += 1;
            }

            let cat_amount = precommitment_inner_puzzle_hashes_to_launch.len() as u64;

            println!("A one-sided offer will be created; it will consume:");
            println!(
                "  - {} payment CAT mojos for creating precommitment coins",
                cat_amount
            );
            println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
            println!("  - 1 mojo for the sake of it");
            yes_no_prompt("Proceed?")?;

            let offer_resp = sage
                .make_offer(
                    no_assets(),
                    assets_xch_and_cat(1, payment_asset_id_str, cat_amount),
                    fee,
                    None,
                    None,
                    false,
                )
                .await?;
            println!("Offer with id {} generated.", offer_resp.offer_id);

            let mut cat_creator_conds = Conditions::new();
            for inner_ph in precommitment_inner_puzzle_hashes_to_launch {
                cat_creator_conds =
                    cat_creator_conds.create_coin(inner_ph.into(), 1, ctx.hint(inner_ph.into())?);
            }
            let cat_destination_puzzle_ptr = ctx.alloc(&clvm_quote!(cat_creator_conds))?;
            let cat_destination_puzzle_hash: Bytes32 =
                ctx.tree_hash(cat_destination_puzzle_ptr).into();

            // Parse one-sided offer
            let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
            let (security_coin_sk, security_coin) =
                create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;
            let (created_cats, cat_assert) = spend_settlement_cats(
                &mut ctx,
                &offer,
                payment_asset_id,
                constants.launcher_id,
                vec![(cat_destination_puzzle_hash, cat_amount)],
            )?;

            let created_cat = created_cats[0];
            let security_coin_conditions = cat_assert
                .assert_concurrent_spend(created_cat.coin.coin_id())
                .reserve_fee(1);

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
                    p2_spend: Spend::new(cat_destination_puzzle_ptr, NodePtr::NIL),
                    revoke: false,
                },
            )?;

            // Build spend bundle
            let sb = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

            println!("Submitting transaction...");
            let resp = client.push_tx(sb).await?;

            println!("Transaction submitted; status='{}'", resp.status);

            wait_for_coin(&client, security_coin.coin_id(), true).await?;
            println!("Confirmed!");

            return Ok(());
        } else {
            println!("All precommitment coins have already been created :)");
        }
    }

    let mut cats = Vec::with_capacity(cats_per_spend);
    while i < cats_to_launch.len() && cats.len() < cats_per_spend {
        cats.push(cats_to_launch[i].clone());
        i += 1;
    }

    println!("These cats will be launched (total number={}):", cats.len());
    for cat in &cats {
        println!("  code: {:?}, name: {:?}", cat.code, cat.name);
    }

    // check if precommitment coins are available and have the appropriate age
    println!("Checking precommitment coins...");
    let precommit_values = cats
        .iter()
        .map(|cat| precommit_value_for_cat(&mut ctx, cat, payment_asset_id))
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
        .get_coin_records_by_puzzle_hashes(precommit_puzzle_hashes.clone(), None, None, Some(false))
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

    let target_block_height = max_confirmed_block_index + constants.relative_block_height + 7;
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
        .get_coin_records_by_names(parent_ids.into_iter().collect(), None, None, Some(true))
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
    println!("  - {} mojos for minting CAT NFTs", cats.len());
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(
            no_assets(),
            assets_xch_only(cats.len() as u64),
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

    let mut security_coin_conditions = Conditions::new();

    for (i, precommit_value) in precommit_values.iter().enumerate() {
        let precommit_ph = precommit_puzzle_hashes[i];
        let precommit_coin_record = precommit_coin_records
            .iter()
            .find(|cr| cr.coin.puzzle_hash == precommit_ph)
            .unwrap();

        let lineage_proof = lineage_proofs
            .get(&precommit_coin_record.coin.parent_coin_info)
            .unwrap();

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
            *precommit_value,
            1,
        )?;

        let tail_hash: Bytes32 = ctx.tree_hash(precommit_value.tail_reveal).into();

        let (left_slot, right_slot) = db
            .get_catalog_neighbors(&mut ctx, constants.launcher_id, tail_hash)
            .await?;

        let (left_slot, right_slot) = catalog.actual_neigbors(tail_hash, left_slot, right_slot);

        let eve_nft_inner_puzzle = initial_cat_inner_puzzle_ptr(&mut ctx, &cats[i])?;

        let sec_conds = catalog.new_action::<CatalogRegisterAction>().spend(
            &mut ctx,
            &mut catalog,
            tail_hash,
            left_slot,
            right_slot,
            precommit_coin,
            Spend {
                puzzle: eve_nft_inner_puzzle,
                solution: NodePtr::NIL,
            },
        )?;

        security_coin_conditions = security_coin_conditions.extend(sec_conds);
    }

    let (_new_catalog, pending_sig) = catalog.finish_spend(&mut ctx)?;

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

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&client, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

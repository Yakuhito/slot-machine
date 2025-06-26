use std::collections::{HashMap, HashSet};

use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes32, SpendBundle},
    puzzles::{cat::CatArgs, singleton::SingletonStruct, CoinProof, LineageProof},
};
use chia_puzzle_types::offer::{NotarizedPayment, Payment};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{CatLayer, Layer, Offer, Puzzle, SingleCatSpend, Spend, SpendContext},
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    assets_xch_and_cat, assets_xch_only, get_last_onchain_timestamp, hex_string_to_bytes32,
    load_xchandles_premine_csv, new_sk, no_assets, parse_amount, parse_one_sided_offer,
    spend_security_coin, sync_xchandles, wait_for_coin, yes_no_prompt, CatalogPrecommitValue,
    CliError, Db, PrecommitCoin, PrecommitLayer, SageClient, XchandlesFactorPricingPuzzleArgs,
    XchandlesFactorPricingSolution, XchandlesPrecommitValue, XchandlesPremineRecord,
    XchandlesRegisterAction,
};

fn precommit_value_for_handle(
    handle: &XchandlesPremineRecord,
    payment_asset_id: Bytes32,
    start_time: u64,
    registration_period: u64,
) -> Result<XchandlesPrecommitValue, CliError> {
    let owner_nft_launcher_id = Address::decode(&handle.owner_nft)?.puzzle_hash;

    Ok(XchandlesPrecommitValue::for_normal_registration(
        payment_asset_id.tree_hash(),
        XchandlesFactorPricingPuzzleArgs::curry_tree_hash(1, registration_period),
        XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: handle.handle.clone(),
            num_periods: 1,
        }
        .tree_hash(),
        handle.handle.clone(),
        Bytes32::default(),
        start_time,
        owner_nft_launcher_id,
        owner_nft_launcher_id.into(),
    ))
}

pub async fn xchandles_continue_launch(
    launcher_id_str: String,
    payment_asset_id_str: String,
    handles_per_spend: usize,
    start_time: Option<u64>,
    registration_period: u64,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
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

        let inner_puzzle_hashes = handles_to_launch
            .iter()
            .map(|handle| {
                let precommit_value = precommit_value_for_handle(
                    handle,
                    payment_asset_id,
                    start_time,
                    registration_period,
                )?;
                let precommit_value_ptr = ctx.alloc(&precommit_value)?;
                let precommit_value_hash = ctx.tree_hash(precommit_value_ptr);

                Ok::<TreeHash, CliError>(PrecommitLayer::<XchandlesPrecommitValue>::puzzle_hash(
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
        while i < handles_to_launch.len() {
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

        if i != handles_to_launch.len() {
            // there are unlaunched precommitment coins, launch those first and exit

            println!(
                "Some precommitment coins were not launched yet - they correspond to these handles:"
            );

            let mut j = i;
            while j < handles_to_launch.len() && j - i < handles_per_spend {
                println!(
                    "  handle: {:}, owner NFT: {:}",
                    handles_to_launch[j].handle, handles_to_launch[j].owner_nft
                );
                j += 1;
            }

            // (inner puzzle hash, amount)
            let mut handles_payment_total = 0;
            let mut precommitment_info_to_launch = Vec::with_capacity(handles_per_spend);
            j = i;
            while j < handles_to_launch.len() && j - i < handles_per_spend {
                let handle_reg_price =
                    XchandlesFactorPricingPuzzleArgs::get_price(1, &handles_to_launch[j].handle, 1);

                precommitment_info_to_launch.push((inner_puzzle_hashes[j], handle_reg_price));
                handles_payment_total += handle_reg_price;

                j += 1;
            }

            println!("A one-sided offer will be created; it will consume:");
            println!(
                "  - {} payment CAT mojos for creating precommitment coins",
                handles_payment_total,
            );
            println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
            println!("  - 1 mojo for the sake of it");
            yes_no_prompt("Proceed?")?;

            let offer_resp = sage
                .make_offer(
                    no_assets(),
                    assets_xch_and_cat(1, payment_asset_id_str, handles_payment_total),
                    fee,
                    None,
                    None,
                    false,
                )
                .await?;
            println!("Offer with id {} generated.", offer_resp.offer_id);

            let mut cat_creator_conds = Conditions::new();
            for (inner_ph, amount) in precommitment_info_to_launch {
                cat_creator_conds = cat_creator_conds.create_coin(
                    inner_ph.into(),
                    amount,
                    Some(ctx.hint(inner_ph.into())?),
                );
            }
            let cat_destination_puzzle_ptr = ctx.alloc(&clvm_quote!(cat_creator_conds))?;
            let cat_destination_puzzle_hash: Bytes32 =
                ctx.tree_hash(cat_destination_puzzle_ptr).into();

            let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
            let security_coin_sk = new_sk()?;

            // Parse one-sided offer
            let one_sided_offer = parse_one_sided_offer(
                &mut ctx,
                offer,
                security_coin_sk.public_key(),
                Some(NotarizedPayment {
                    nonce: launcher_id,
                    payments: vec![Payment::with_memos(
                        cat_destination_puzzle_hash,
                        handles_payment_total,
                        vec![cat_destination_puzzle_hash.into()],
                    )],
                }),
                None,
            )?;

            let Some(created_cat) = one_sided_offer.created_cat else {
                eprintln!("No CAT was created in one-sided offer - aborting...");
                return Ok(());
            };
            one_sided_offer
                .coin_spends
                .into_iter()
                .for_each(|cs| ctx.insert(cs));

            let security_coin_conditions = one_sided_offer
                .security_base_conditions
                .assert_concurrent_spend(created_cat.coin.coin_id())
                .reserve_fee(1);

            // Spend security coin
            let security_coin_sig = spend_security_coin(
                &mut ctx,
                one_sided_offer.security_coin,
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
                        inner_puzzle_hash: created_cat.p2_puzzle_hash,
                        amount: created_cat.coin.amount,
                    },
                    prev_coin_id: created_cat.coin.coin_id(),
                    prev_subtotal: 0,
                    extra_delta: 0,
                    inner_spend: Spend::new(cat_destination_puzzle_ptr, NodePtr::NIL),
                },
            )?;

            // Build spend bundle
            let sb = SpendBundle::new(
                ctx.take(),
                one_sided_offer.aggregated_signature + &security_coin_sig,
            );

            println!("Submitting transaction...");
            let resp = client.push_tx(sb).await?;

            println!("Transaction submitted; status='{}'", resp.status);

            wait_for_coin(&client, one_sided_offer.security_coin.coin_id(), true).await?;
            println!("Confirmed!");

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
            "  handle: {:}, owner NFT: {:}",
            handle.handle, handle.owner_nft
        );
    }

    // check if precommitment coins are available and have the appropriate age
    println!("Checking precommitment coins...");
    let precommit_values = handles
        .iter()
        .map(|handle| {
            precommit_value_for_handle(handle, payment_asset_id, start_time, registration_period)
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
    println!("  - 1 mojo for the sake of it");
    println!("  - {} XCH for fees ({} mojos)", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let mut security_coin_conditions = offer.security_base_conditions.reserve_fee(1);

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

        let sec_conds = registry.new_action::<XchandlesRegisterAction>().spend(
            &mut ctx,
            &mut registry,
            left_slot,
            right_slot,
            precommit_coin,
            1,
            registration_period,
        )?;

        security_coin_conditions = security_coin_conditions.extend(sec_conds);
    }

    let _new_registry = registry.finish_spend(&mut ctx)?;

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

    let sb = SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig);

    println!("Submitting transaction...");
    let resp = client.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

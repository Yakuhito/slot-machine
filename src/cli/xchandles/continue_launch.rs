use std::collections::{HashMap, HashSet};

use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use chia_puzzle_types::{
    cat::CatArgs,
    nft::{NftOwnershipLayerSolution, NftStateLayerSolution},
    singleton::{SingletonSolution, SingletonStruct},
    standard::StandardSolution,
    CoinProof, EveProof, LineageProof, Proof,
};
use chia_puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        create_security_coin, decode_offer, spend_security_coin, spend_settlement_cats, CatLayer,
        CatalogPrecommitValue, Launcher, Layer, Nft, NftInfo, Offer, PrecommitCoin, PrecommitLayer,
        Puzzle, SingleCatSpend, SingletonInfo, Spend, SpendContext, StandardLayer,
        XchandlesPrecommitValue, XchandlesRegisterAction,
    },
    types::{
        puzzles::{
            HandleNftMetadata, XchandlesFactorPricingPuzzleArgs, XchandlesPricingSolution,
            ANY_METADATA_UPDATER_HASH,
        },
        Conditions, Mod, MAINNET_CONSTANTS, TESTNET11_CONSTANTS,
    },
    utils::Address,
};
use clvm_traits::clvm_quote;
use clvm_utils::ToTreeHash;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    assets_xch_and_cat, assets_xch_only, clear_pending_batch_spend, confirm_pushed_transaction,
    decide_batch_retry, default_mainnet_bundle_path, default_mainnet_plan_path,
    default_pending_batch_spend_path, default_testnet11_bundle_path, default_testnet11_plan_path,
    emit_pre_broadcast_plan, finality_reached, get_prefix, hex_string_to_bytes32,
    hex_string_to_pubkey, hex_string_to_signature, launch_handles_from_bundle,
    load_pending_batch_spend, load_premine_launch_bundle, new_pending_batch_spend, no_assets,
    parse_amount, sync_xchandles, verify_premine_set_against_bundle, write_pending_batch_spend,
    yes_no_prompt, BatchRetryDecision, CliError, Db, InputCoinState, LaunchHandle, SageClient,
    VerificationPhase, PREMINE_FINALITY_DEPTH,
};
use std::collections::BTreeMap;
use std::path::Path;

fn precommit_value_for_handle(
    handle: &LaunchHandle,
    handle_nft_launcher_id: Bytes32,
    payment_asset_id: Bytes32,
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
            buy_time: handle.buy_time,
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

pub fn metadata_for_handle_nft(handle_info: &LaunchHandle) -> HandleNftMetadata {
    HandleNftMetadata {
        display_name: Some(handle_info.handle.clone()),
        image_uris: handle_info.image_uris.clone(),
        image_hash: Some(handle_info.image_hash),
        metadata_uris: handle_info.metadata_uris.clone(),
        metadata_hash: Some(handle_info.metadata_hash),
        license_uris: handle_info.license_uris.clone(),
        license_hash: Some(handle_info.license_hash),
    }
}

async fn input_coin_states_for_pending(
    client: &CoinsetClient,
    pending: &crate::PendingBatchSpendRecord,
) -> Result<BTreeMap<String, InputCoinState>, CliError> {
    let mut states = BTreeMap::new();

    for coin_hex in &pending.input_coin_ids {
        let coin_id = hex_string_to_bytes32(coin_hex)?;
        let Some(record) = client.get_coin_record_by_name(coin_id).await?.coin_record else {
            // Missing record — treat as conflicting so we do not blind-retry.
            states.insert(coin_hex.clone(), InputCoinState::SpentConflicting);
            continue;
        };
        if !record.spent {
            states.insert(coin_hex.clone(), InputCoinState::Unspent);
        } else {
            // Input of our pending spend is spent: treat as applied by that pending.
            // A mixed unspent+spent set still conflicts via decide_batch_retry.
            states.insert(coin_hex.clone(), InputCoinState::SpentByPending);
        }
    }
    Ok(states)
}

async fn maybe_reuse_pending_batch_spend(
    client: &CoinsetClient,
    batch_id: u32,
    phase: &str,
) -> Result<Option<chia_protocol::SpendBundle>, CliError> {
    let path = default_pending_batch_spend_path(batch_id, phase);
    let Some(pending) = load_pending_batch_spend(&path)? else {
        return Ok(None);
    };
    let states = input_coin_states_for_pending(client, &pending).await?;
    match decide_batch_retry(Some(&pending), &states) {
        BatchRetryDecision::ReuseIdentical(record) => {
            println!(
                "Reusing identical pending {} spend for batch {} (inputs still unspent).",
                phase, batch_id
            );
            Ok(Some(record.spend_bundle))
        }
        BatchRetryDecision::AlreadyApplied(_) => {
            println!(
                "Pending {} spend for batch {} already applied; clearing pending record.",
                phase, batch_id
            );
            clear_pending_batch_spend(&path)?;
            Ok(None)
        }
        BatchRetryDecision::Conflict(report) => {
            let json = serde_json::to_string_pretty(&report).map_err(|e| {
                CliError::Custom(format!("conflict report serialize failed: {e}"))
            })?;
            Err(CliError::Custom(format!(
                "spent/conflicting input stops blind retry for batch {batch_id} phase {phase}:\n{json}"
            )))
        }
        BatchRetryDecision::ConstructFresh => Ok(None),
    }
}

async fn persist_and_push_batch_spend(
    client: &CoinsetClient,
    registry_launcher_id: Bytes32,
    batch_id: u32,
    phase: &str,
    handles: &[LaunchHandle],
    input_coin_ids: Vec<Bytes32>,
    sb: SpendBundle,
    confirm_coin_id: Bytes32,
) -> Result<(), CliError> {
    let path = default_pending_batch_spend_path(batch_id, phase);
    let record = new_pending_batch_spend(
        registry_launcher_id,
        batch_id,
        phase,
        handles.iter().map(|h| h.handle.clone()).collect(),
        input_coin_ids,
        sb.clone(),
    );
    write_pending_batch_spend(&path, &record)?;
    println!("Persisted identical-retry spend to {path}");

    println!("Submitting transaction...");
    let resp = client.push_tx(sb).await?;

    if confirm_pushed_transaction(client, &resp, confirm_coin_id, true).await? {
        println!("Confirmed!");
        clear_pending_batch_spend(&path)?;
    }
    Ok(())
}

async fn verify_registered_batches_or_stop(
    client: &CoinsetClient,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    bundle: &crate::PremineLaunchBundle,
    through_batch_id: u32,
) -> Result<(), CliError> {
    println!(
        "Running canonical Premine verification through batch {through_batch_id}..."
    );
    let (canonical, confirm_height) = verify_premine_set_against_bundle(
        client,
        ctx,
        launcher_id,
        bundle,
        Some(through_batch_id),
        VerificationPhase::Canonical,
    )
    .await?;
    println!("{}", canonical.to_machine_readable_json()?);
    canonical.gate_later_batches()?;

    let Some(confirm_height) = confirm_height else {
        return Err(CliError::Custom(
            "canonical verification succeeded but confirmation height is unknown".to_string(),
        ));
    };

    println!(
        "Waiting for {PREMINE_FINALITY_DEPTH}-block finality above height {confirm_height} before allowing later batches..."
    );
    loop {
        let resp = client.get_blockchain_state().await?;
        let Some(state) = resp.blockchain_state else {
            return Err(CliError::Custom(
                "Failed to get blockchain state while waiting for batch finality".to_string(),
            ));
        };
        if finality_reached(confirm_height, state.peak.height) {
            break;
        }
        println!(
            "Peak #{}; need {} more blocks...",
            state.peak.height,
            PREMINE_FINALITY_DEPTH.saturating_sub(state.peak.height.saturating_sub(confirm_height))
        );
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }

    println!("Re-running final Premine verification through batch {through_batch_id}...");
    let (final_report, _) = verify_premine_set_against_bundle(
        client,
        ctx,
        launcher_id,
        bundle,
        Some(through_batch_id),
        VerificationPhase::Final,
    )
    .await?;
    println!("{}", final_report.to_machine_readable_json()?);
    final_report.gate_later_batches()?;
    println!("Batch {through_batch_id} final verification OK; later batches may proceed.");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn eve_nft_for_handle(
    ctx: &mut SpendContext,
    client: &CoinsetClient,
    registry_launcher_id: Bytes32,
    handle: &LaunchHandle,
    royalty_puzzle_hash: Bytes32,
    royalty_basis_points: u16,
    eve_nft_temp_inner_ph: Bytes32,
    include_spent_coins: bool,
) -> Result<Option<Nft>, CliError> {
    let hint = (registry_launcher_id, handle.handle.tree_hash())
        .tree_hash()
        .into();

    let metadata = metadata_for_handle_nft(handle);
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
            let launcher_id = possible_launcher_record.coin.coin_id();
            let nft_info = NftInfo::new(
                launcher_id,
                metadata,
                ANY_METADATA_UPDATER_HASH.into(),
                None,
                royalty_puzzle_hash,
                royalty_basis_points,
                eve_nft_temp_inner_ph,
            );
            let eve_nft_ph = nft_info.puzzle_hash();

            (nft_info, Coin::new(launcher_id, eve_nft_ph.into(), 1))
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
    skip: usize,
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
    if !testnet11 {
        if royalty_puzzle_hash != crate::ROYALTY_PUZZLE_HASH
            || royalty_basis_points != crate::ROYALTY_BASIS_POINTS
            || royalty_address != crate::ROYALTY_ADDRESS
        {
            return Err(CliError::Custom(format!(
                "Mainnet royalty must be {} at {} BPS (puzzle hash {})",
                crate::ROYALTY_ADDRESS,
                crate::ROYALTY_BASIS_POINTS,
                hex::encode(crate::ROYALTY_PUZZLE_HASH)
            )));
        }
        if registration_period != crate::REGISTRATION_PERIOD {
            return Err(CliError::Custom(format!(
                "Mainnet registration period must be exactly {} seconds",
                crate::REGISTRATION_PERIOD
            )));
        }
    }
    println!("Time to unroll an XCHandles registry! Yee-haw!");

    let bundle_path = if testnet11 {
        default_testnet11_bundle_path()
    } else {
        default_mainnet_bundle_path()
    };

    println!("Loading Premine Launch Bundle from '{}'...", bundle_path);
    let bundle = load_premine_launch_bundle(bundle_path)?;
    if handles_per_spend != bundle.handles_per_batch {
        return Err(CliError::Custom(format!(
            "handles_per_spend ({handles_per_spend}) must match bundle handles_per_batch ({})",
            bundle.handles_per_batch
        )));
    }
    if registration_period != bundle.registration_period {
        return Err(CliError::Custom(format!(
            "registration_period ({registration_period}) must match bundle ({})",
            bundle.registration_period
        )));
    }
    if start_time.is_some() {
        println!(
            "Ignoring --start-time; Premine Launch Bundle rows carry per-handle buy_time/expiration."
        );
    }
    let handles_to_launch = launch_handles_from_bundle(&bundle)?;
    println!(
        "Loaded {} handles from launch bundle (handles_per_batch={}).",
        handles_to_launch.len(),
        bundle.handles_per_batch
    );

    let plan_path = if testnet11 {
        Path::new(default_testnet11_plan_path())
    } else {
        Path::new(default_mainnet_plan_path())
    };
    let plan = emit_pre_broadcast_plan(&bundle, Some(plan_path))?;
    println!(
        "Pre-broadcast plan ready: {} rows in {} batches.",
        plan.total_rows,
        plan.batches.len()
    );

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

        if i == 0 && skip > 1 {
            i = skip;
        } else {
            i += 1;
        }
    }

    if i == handles_to_launch.len() {
        eprintln!("All handles have already been registered - nothing to do!");
        return Ok(());
    }

    let current_batch_id = handles_to_launch[i].batch_id;
    if current_batch_id > 0 {
        // Prior batches must pass final verification before constructing the next.
        verify_registered_batches_or_stop(
            &client,
            &mut ctx,
            launcher_id,
            &bundle,
            current_batch_id - 1,
        )
        .await?;
    }

    // Prefer identical pending spend for this batch before constructing a fresh one.
    if let Some(sb) = maybe_reuse_pending_batch_spend(&client, current_batch_id, "mint_precommit")
        .await?
    {
        let confirm_coin = sb
            .coin_spends
            .first()
            .map(|cs| cs.coin.coin_id())
            .ok_or_else(|| CliError::Custom("pending mint spend has no coin spends".to_string()))?;
        println!("Submitting reused mint_precommit spend for batch {current_batch_id}...");
        let resp = client.push_tx(sb).await?;
        if confirm_pushed_transaction(&client, &resp, confirm_coin, true).await? {
            println!("Confirmed reused mint_precommit!");
            clear_pending_batch_spend(default_pending_batch_spend_path(
                current_batch_id,
                "mint_precommit",
            ))?;
        }
        return Ok(());
    }
    if let Some(sb) =
        maybe_reuse_pending_batch_spend(&client, current_batch_id, "register").await?
    {
        let confirm_coin = sb
            .coin_spends
            .first()
            .map(|cs| cs.coin.coin_id())
            .ok_or_else(|| {
                CliError::Custom("pending register spend has no coin spends".to_string())
            })?;
        println!("Submitting reused register spend for batch {current_batch_id}...");
        let resp = client.push_tx(sb).await?;
        if confirm_pushed_transaction(&client, &resp, confirm_coin, true).await? {
            println!("Confirmed reused register!");
            clear_pending_batch_spend(default_pending_batch_spend_path(
                current_batch_id,
                "register",
            ))?;
            verify_registered_batches_or_stop(
                &client,
                &mut ctx,
                launcher_id,
                &bundle,
                current_batch_id,
            )
            .await?;
        }
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

    let constants = registry.info.constants;

    if i == 0 {
        println!("No handles registered yet - looking for precommitment coins...");

        let mut i = 0;
        while i < handles_to_launch.len() {
            let Some(_eve_nft) = eve_nft_for_handle(
                &mut ctx,
                &client,
                registry.info.constants.launcher_id,
                &handles_to_launch[i],
                royalty_puzzle_hash,
                royalty_basis_points,
                eve_nft_temp_inner_ph,
                false,
            )
            .await?
            else {
                break;
            };

            if i == 0 && skip > 1 {
                i = skip;
            } else {
                i += 1;
            }
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
                // unique hint per deployment to minimize confusions
                let hint: Bytes32 = (
                    registry.info.constants.launcher_id,
                    handle_info.handle.tree_hash(),
                )
                    .tree_hash()
                    .into();
                let hint = ctx.hint(hint)?;

                // Launch eve NFT
                let launcher_amount = (index * 2) as u64;
                let launcher = Launcher::with_memos(security_coin.coin_id(), launcher_amount, hint)
                    .with_singleton_amount(1);
                let launcher_id = launcher.coin().coin_id();

                println!(
                    "Handle {} will be represented by NFT {}",
                    handle_info.handle,
                    Address::new(launcher_id, "nft".to_string()).encode()?
                );

                let metadata = metadata_for_handle_nft(&handle_info);
                let metadata = ctx.alloc_hashed(&metadata)?;

                let (sec_conditions, _eve_nft) = launcher.mint_eve_nft(
                    &mut ctx,
                    eve_nft_temp_inner_ph,
                    metadata,
                    ANY_METADATA_UPDATER_HASH.into(),
                    royalty_puzzle_hash,
                    royalty_basis_points,
                )?;

                security_coin_conditions = security_coin_conditions.extend(sec_conditions);

                // Create precommitment coin
                let handle_reg_price =
                    XchandlesFactorPricingPuzzleArgs::get_price(1, &handle_info.handle, 1);

                let precommit_value = precommit_value_for_handle(
                    &handle_info,
                    launcher_id,
                    payment_asset_id,
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
            security_coin_conditions = security_coin_conditions
                .extend(cat_assert)
                .assert_concurrent_spend(created_cat.coin.coin_id());

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
            let mint_handles = &handles_to_launch[i..j];
            persist_and_push_batch_spend(
                &client,
                registry.info.constants.launcher_id,
                current_batch_id,
                "mint_precommit",
                mint_handles,
                vec![security_coin.coin_id()],
                sb,
                security_coin.coin_id(),
            )
            .await?;

            return Ok(());
        } else {
            println!("All precommitment coins have already been created :)");
        }
    }

    let _i_offset = i; // used to map indices from 'handles' to 'handles_to_launch'

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
    let mut destination_puzzle_hashes = Vec::with_capacity(handles.len());

    for (_index, handle) in handles.iter().enumerate() {
        let Some(eve_nft) = eve_nft_for_handle(
            &mut ctx,
            &client,
            registry.info.constants.launcher_id,
            handle,
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
        destination_puzzle_hashes.push(handle.recipient);
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

    let eve_nft_temp_pubkey = hex_string_to_pubkey(&derivation_resp.derivations[0].public_key)?;
    let eve_nft_temp_p2 = StandardLayer::new(eve_nft_temp_pubkey);

    let mut security_coin_conditions = Conditions::new().reserve_fee(1);

    let mut nft_coin_spends = Vec::with_capacity(precommit_values.len());

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

        let eve_nft = eve_nfts[i];
        let eve_nft_layers = eve_nft.info.into_layers(eve_nft_temp_p2);
        let eve_nft_inner_puzzle_hash = eve_nft.info.inner_puzzle_hash().into();

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
                handles[i].buy_time,
                eve_nft_inner_puzzle_hash,
                eve_nft_inner_puzzle_hash,
            )?;

        let mut nft_conds = owner_message_conds;
        if let Some(resolved_message_conds) = resolved_message_conds {
            nft_conds = nft_conds.extend(resolved_message_conds);
        }

        let hint = ctx.hint(destination_puzzle_hashes[i])?;
        let delegated_puzzle = ctx.alloc(&clvm_quote!(nft_conds.create_coin(
            destination_puzzle_hashes[i],
            1,
            hint
        )))?;

        let eve_nft_spend = eve_nft_layers.construct_spend(
            &mut ctx,
            SingletonSolution {
                lineage_proof: eve_nft.proof,
                amount: eve_nft.coin.amount,
                inner_solution: NftStateLayerSolution {
                    inner_solution: NftOwnershipLayerSolution {
                        inner_solution: StandardSolution {
                            original_public_key: None,
                            delegated_puzzle,
                            solution: NodePtr::NIL,
                        },
                    },
                },
            },
        )?;

        let eve_nft_spend_puzzle = ctx.serialize(&eve_nft_spend.puzzle)?;
        let eve_nft_spend_solution = ctx.serialize(&eve_nft_spend.solution)?;
        let eve_nft_coin_spend =
            CoinSpend::new(eve_nft.coin, eve_nft_spend_puzzle, eve_nft_spend_solution);

        ctx.insert(eve_nft_coin_spend.clone());
        nft_coin_spends.push(eve_nft_coin_spend);

        // no need to assert NFT being spent as register won't go through without the NFT
        //  approving registration via message
        security_coin_conditions = security_coin_conditions.extend(register_conds);
    }

    // Get signature required to spend eve NFTs
    let nft_sig = hex_string_to_signature(
        &sage
            .sign_coin_spends(nft_coin_spends, false, true)
            .await?
            .spend_bundle
            .aggregated_signature,
    )?;

    let registry_input_coin_id = registry.coin.coin_id();
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
        security_coin_sig + &pending_sig + &nft_sig,
    ));

    let register_batch_id = handles
        .first()
        .map(|h| h.batch_id)
        .unwrap_or(current_batch_id);
    persist_and_push_batch_spend(
        &client,
        launcher_id,
        register_batch_id,
        "register",
        &handles,
        vec![security_coin.coin_id(), registry_input_coin_id],
        sb,
        security_coin.coin_id(),
    )
    .await?;

    verify_registered_batches_or_stop(
        &client,
        &mut ctx,
        launcher_id,
        &bundle,
        register_batch_id,
    )
    .await?;

    Ok(())
}

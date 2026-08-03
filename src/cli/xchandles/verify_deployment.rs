use std::collections::BTreeMap;

use chia_protocol::{Bytes32, Coin};
use chia_puzzle_types::singleton::{LauncherSolution, SingletonSolution};
use chia_puzzle_types::Memos;
use chia_wallet_sdk::driver::{
    HashedPtr, Layer, MedievalVaultInfo, Nft, Puzzle, SingletonInfo, XchandlesExpirePricingPuzzle,
    XchandlesRegistry, XchandlesRegistryReceivedMessagePrefix, XchandlesRegistryState,
};
use chia_wallet_sdk::types::puzzles::{
    DefaultCatMakerArgs, HandleNftMetadata, XchandlesFactorPricingPuzzleArgs,
    XchandlesPricingSolution, XchandlesRegisterActionSolution,
};
use chia_wallet_sdk::types::{Condition, Conditions, Mod};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{ActionLayer, SpendContext},
};
use clvm_traits::clvm_list;
use clvm_utils::ToTreeHash;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    compare_premine_set, controller_matches_configured, default_mainnet_bundle_path,
    default_testnet11_bundle_path, expected_observations_from_bundle_rows, finality_reached,
    get_coinset_client, handle_nft_metadata_clvm_hex, hex_string_to_bytes32,
    launch_handles_from_bundle, load_premine_launch_bundle, load_xchandles_state_schedule_csv,
    price_singleton_public_keys, print_medieval_vault_configuration,
    reorganization_invalidates_finality, CliError, MultisigSingleton, ObservedPremineHandle,
    PremineLaunchBundle, PremineVerificationReport, VerificationPhase,
    XchandlesStateScheduleRecord, OWNER_RESOLVED_RELATIONSHIP, PREMINE_FINALITY_DEPTH,
    PRICE_SINGLETON_M, ROYALTY_BASIS_POINTS, ROYALTY_PUZZLE_HASH,
};

use crate::sync_multisig_singleton;

/// Compares a trusted CSV schedule to the on-chain scheduler schedule end-to-end.
pub fn xchandles_trusted_schedule_matches(
    trusted: &[XchandlesStateScheduleRecord],
    on_chain: &[(u64, XchandlesRegistryState)],
) -> bool {
    if trusted.len() != on_chain.len() {
        return false;
    }

    for (record, (timestamp, state)) in trusted.iter().zip(on_chain.iter()) {
        let pricing_puzzle_hash = XchandlesFactorPricingPuzzleArgs {
            base_price: record.registration_price,
            registration_period: record.registration_period,
        }
        .curry_tree_hash();
        let expired_handle_pricing_puzzle_hash = XchandlesExpirePricingPuzzle::curry_tree_hash(
            record.registration_price,
            record.registration_period,
        );
        let cat_maker_puzzle_hash =
            DefaultCatMakerArgs::new(record.asset_id.tree_hash().into()).curry_tree_hash();

        if record.timestamp != *timestamp
            || state.pricing_puzzle_hash != pricing_puzzle_hash.into()
            || state.expired_handle_pricing_puzzle_hash != expired_handle_pricing_puzzle_hash.into()
            || state.cat_maker_puzzle_hash != cat_maker_puzzle_hash.into()
        {
            return false;
        }
    }

    true
}

/// Reconstruct registered Premine Handles from the registry spend chain and
/// compare complete set equality against the launch bundle (through optional
/// batch ceiling). Read-only Coinset RPC — does not broadcast.
pub async fn verify_premine_set_against_bundle(
    cli: &CoinsetClient,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    bundle: &PremineLaunchBundle,
    through_batch_id: Option<u32>,
    phase: VerificationPhase,
) -> Result<(PremineVerificationReport, Option<u32>), CliError> {
    let rows: Vec<_> = match through_batch_id {
        Some(max_batch) => bundle
            .rows
            .iter()
            .filter(|r| r.batch_id <= max_batch)
            .cloned()
            .collect(),
        None => bundle.rows.clone(),
    };
    let handles_by_hash: BTreeMap<Bytes32, &crate::PremineLaunchBundleRow> = rows
        .iter()
        .map(|r| (r.handle.tree_hash().into(), r))
        .collect();

    let Some(launcher_coin_record) = cli.get_coin_record_by_name(launcher_id).await?.coin_record
    else {
        return Err(CliError::CoinNotFound(launcher_id));
    };

    let Some(launcher_coin_solution) = cli
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?
        .coin_solution
    else {
        return Err(CliError::CoinNotSpent(launcher_id));
    };

    let launcher_solution_ptr = node_from_bytes(ctx, &launcher_coin_solution.solution)?;
    let Some((mut registry, _initial_slots, _asset_id, _base_price)) =
        XchandlesRegistry::from_launcher_solution(
            ctx,
            launcher_coin_record.coin,
            launcher_solution_ptr,
        )?
    else {
        return Err(CliError::Custom(
            "XCHandles registry was not launched correctly.".to_string(),
        ));
    };

    let mut observed: Vec<ObservedPremineHandle> = Vec::new();
    let mut nft_launcher_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut last_confirmation_height: Option<u32> = None;

    loop {
        let Some(coin_record) = cli
            .get_coin_record_by_name(registry.coin.coin_id())
            .await?
            .coin_record
        else {
            return Err(CliError::CoinNotFound(registry.coin.coin_id()));
        };

        let Some(coin_spend) = cli
            .get_puzzle_and_solution(registry.coin.coin_id(), Some(coin_record.spent_block_index))
            .await?
            .coin_solution
        else {
            break;
        };

        last_confirmation_height = Some(coin_record.spent_block_index);

        let solution = node_from_bytes(ctx, &coin_spend.solution)?;
        let parsed_solution = ctx.extract::<SingletonSolution<NodePtr>>(solution)?;
        let inner_solution = ActionLayer::<XchandlesRegistryState, NodePtr>::parse_solution(
            ctx,
            parsed_solution.inner_solution,
        )?;

        for action_spend in inner_solution.action_spends {
            let action_solution = ctx.extract::<XchandlesRegisterActionSolution<
                NodePtr,
                NodePtr,
                NodePtr,
                NodePtr,
                NodePtr,
            >>(action_spend.solution)?;

            let Some(bundle_row) = handles_by_hash.get(&action_solution.handle_hash) else {
                observed.push(ObservedPremineHandle {
                    handle: format!("unknown_hash_{}", hex::encode(action_solution.handle_hash)),
                    recipient_puzzle_hash: String::new(),
                    expiration: 0,
                    owner_resolved_relationship: String::new(),
                    owner_launcher_id: hex::encode(
                        action_solution
                            .other_precommit_data
                            .launcher_ids
                            .owner_launcher_id,
                    ),
                    resolved_launcher_id: hex::encode(
                        action_solution
                            .other_precommit_data
                            .launcher_ids
                            .resolved_launcher_id,
                    ),
                    nft_launcher_id: hex::encode(
                        action_solution
                            .other_precommit_data
                            .launcher_ids
                            .owner_launcher_id,
                    ),
                    display_name: String::new(),
                    image_uri: String::new(),
                    image_hash: String::new(),
                    metadata_uri: String::new(),
                    metadata_hash: String::new(),
                    license_uri: String::new(),
                    license_hash: String::new(),
                    handle_nft_metadata_clvm_hex: String::new(),
                    updater_hash: String::new(),
                    royalty_puzzle_hash: String::new(),
                    royalty_basis_points: 0,
                    batch_id: u32::MAX,
                    row_index: u32::MAX,
                });
                continue;
            };

            let nft_launcher_id = action_solution
                .other_precommit_data
                .launcher_ids
                .owner_launcher_id;
            let resolved_launcher_id = action_solution
                .other_precommit_data
                .launcher_ids
                .resolved_launcher_id;

            let pricing = ctx.extract::<XchandlesPricingSolution>(
                action_solution.pricing_puzzle_and_solution.solution,
            )?;
            let expiration = pricing.buy_time + bundle.registration_period * pricing.num_periods;

            let Some(launcher_coin_record) = cli
                .get_coin_record_by_name(nft_launcher_id)
                .await?
                .coin_record
            else {
                return Err(CliError::Custom(format!(
                    "Could not fetch record for launcher coin {}",
                    hex::encode(nft_launcher_id)
                )));
            };

            let Some(launcher_spend) = cli
                .get_puzzle_and_solution(
                    nft_launcher_id,
                    Some(launcher_coin_record.spent_block_index),
                )
                .await?
                .coin_solution
            else {
                return Err(CliError::Custom(format!(
                    "Could not fetch coin spend for launcher coin {}",
                    hex::encode(nft_launcher_id)
                )));
            };

            let launcher_solution = ctx.alloc(&launcher_spend.solution)?;
            let launcher_solution = ctx.extract::<LauncherSolution<NodePtr>>(launcher_solution)?;
            let eve_nft_coin =
                Coin::new(nft_launcher_id, launcher_solution.singleton_puzzle_hash, 1);

            let Some(eve_nft_record) = cli
                .get_coin_record_by_name(eve_nft_coin.coin_id())
                .await?
                .coin_record
            else {
                return Err(CliError::Custom(format!(
                    "Could not fetch record for eve nft coin {}",
                    hex::encode(eve_nft_coin.coin_id())
                )));
            };

            let Some(eve_nft_spend) = cli
                .get_puzzle_and_solution(
                    eve_nft_coin.coin_id(),
                    Some(eve_nft_record.spent_block_index),
                )
                .await?
                .coin_solution
            else {
                return Err(CliError::Custom(format!(
                    "Could not fetch coin spend for eve nft coin {}",
                    hex::encode(eve_nft_coin.coin_id())
                )));
            };

            let puzzle_ptr = ctx.alloc(&eve_nft_spend.puzzle_reveal)?;
            let puzzle = Puzzle::parse(ctx, puzzle_ptr);
            let solution_ptr = ctx.alloc(&eve_nft_spend.solution)?;
            let Some((eve_nft, inner_puzzle, inner_solution_ptr)) =
                Nft::parse(ctx, eve_nft_coin, puzzle, solution_ptr)?
            else {
                return Err(CliError::Custom(format!(
                    "Could not parse eve nft coin {}",
                    hex::encode(eve_nft_coin.coin_id())
                )));
            };

            let metadata: HandleNftMetadata = ctx.extract(eve_nft.info.metadata.ptr())?;
            let metadata_clvm_hex = handle_nft_metadata_clvm_hex(&metadata)?;

            let p2_output = ctx.run(inner_puzzle.ptr(), inner_solution_ptr)?;
            let p2_output = ctx.extract::<Conditions<HashedPtr>>(p2_output)?;
            let p2_output = p2_output.into_vec();
            if p2_output.len() != 3 {
                return Err(CliError::Custom(format!(
                    "P2 output contains {} conditions, expected 3 for handle {}",
                    p2_output.len(),
                    bundle_row.handle
                )));
            }
            let (
                Condition::<HashedPtr>::AggSigMe(ref _cond),
                Condition::<HashedPtr>::SendMessage(ref send_message),
                Condition::<HashedPtr>::CreateCoin(ref create_coin),
            ) = (&p2_output[0], &p2_output[1], &p2_output[2])
            else {
                return Err(CliError::Custom(format!(
                    "P2 output does not contain the correct conditions for handle {}",
                    bundle_row.handle
                )));
            };

            let Memos::Some(memos) = create_coin.memos else {
                return Err(CliError::Custom(format!(
                    "P2 output missing memo for handle {}",
                    bundle_row.handle
                )));
            };
            if create_coin.amount != 1
                || memos.tree_hash() != clvm_list!(create_coin.puzzle_hash).tree_hash()
            {
                return Err(CliError::Custom(format!(
                    "P2 recreation condition invalid for handle {}",
                    bundle_row.handle
                )));
            }
            if send_message.mode != 18
                || send_message.message[0]
                    != XchandlesRegistryReceivedMessagePrefix::RegisterOwner as u8
            {
                return Err(CliError::Custom(format!(
                    "P2 send message invalid for handle {}",
                    bundle_row.handle
                )));
            }

            let relationship = if nft_launcher_id == resolved_launcher_id {
                OWNER_RESOLVED_RELATIONSHIP.to_string()
            } else {
                "distinct_singletons".to_string()
            };

            nft_launcher_ids.insert(bundle_row.handle.clone(), hex::encode(nft_launcher_id));

            observed.push(ObservedPremineHandle {
                handle: bundle_row.handle.clone(),
                recipient_puzzle_hash: hex::encode(create_coin.puzzle_hash),
                expiration,
                owner_resolved_relationship: relationship,
                owner_launcher_id: hex::encode(nft_launcher_id),
                resolved_launcher_id: hex::encode(resolved_launcher_id),
                nft_launcher_id: hex::encode(nft_launcher_id),
                display_name: metadata.display_name.clone().unwrap_or_default(),
                image_uri: metadata.image_uris.first().cloned().unwrap_or_default(),
                image_hash: metadata.image_hash.map(hex::encode).unwrap_or_default(),
                metadata_uri: metadata.metadata_uris.first().cloned().unwrap_or_default(),
                metadata_hash: metadata.metadata_hash.map(hex::encode).unwrap_or_default(),
                license_uri: metadata.license_uris.first().cloned().unwrap_or_default(),
                license_hash: metadata.license_hash.map(hex::encode).unwrap_or_default(),
                handle_nft_metadata_clvm_hex: metadata_clvm_hex,
                updater_hash: hex::encode(eve_nft.info.metadata_updater_puzzle_hash),
                royalty_puzzle_hash: hex::encode(eve_nft.info.royalty_puzzle_hash),
                royalty_basis_points: eve_nft.info.royalty_basis_points,
                batch_id: bundle_row.batch_id,
                row_index: bundle_row.row_index,
            });
        }

        registry = registry.child(registry.pending_spend.latest_state.1);
    }

    let expected = expected_observations_from_bundle_rows(&rows, &nft_launcher_ids)?;
    let report = compare_premine_set(&expected, &observed, phase);
    Ok((report, last_confirmation_height))
}

pub async fn xchandles_verify_deployment(
    launcher_id_str: String,
    testnet11: bool,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);

    let price_schedule_csv_filename = if testnet11 {
        "xchandles_price_schedule_testnet11.csv"
    } else {
        "xchandles_price_schedule_mainnet.csv"
    };

    let bundle_path = if testnet11 {
        default_testnet11_bundle_path()
    } else {
        default_mainnet_bundle_path()
    };

    println!("Verifying XCHandles deployment (testnet: {})...", testnet11);

    println!("Let's start with the XCHandles registry.");
    println!(
        "It should also have a premine that matches the Premine Launch Bundle '{}' (TRUSTED SOURCE).",
        bundle_path
    );

    let bundle = load_premine_launch_bundle(bundle_path)?;
    let handles_to_launch = launch_handles_from_bundle(&bundle)?;

    let Some(launcher_coin_record) = cli.get_coin_record_by_name(launcher_id).await?.coin_record
    else {
        return Err(CliError::CoinNotFound(launcher_id));
    };

    let Some(launcher_coin_solution) = cli
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?
        .coin_solution
    else {
        return Err(CliError::CoinNotSpent(launcher_id));
    };

    let launcher_solution_ptr = node_from_bytes(&mut ctx, &launcher_coin_solution.solution)?;

    let Some((registry, _initial_slots, initial_registration_asset_id, initial_base_price)) =
        XchandlesRegistry::from_launcher_solution(
            &mut ctx,
            launcher_coin_record.coin,
            launcher_solution_ptr,
        )?
    else {
        return Err(CliError::Custom(
            "XCHandles registry was not launched correctly.".to_string(),
        ));
    };

    if initial_base_price != 1 {
        return Err(CliError::Custom(
            "XCHandles registry was not launched with a base price of 1.".to_string(),
        ));
    }

    println!(
        "Registry launched at height {} with a premine registration CAT asset id of {}.",
        launcher_coin_record.spent_block_index,
        hex::encode(initial_registration_asset_id)
    );

    println!(
        "Running canonical Premine set-equality verification over {} bundle rows...",
        handles_to_launch.len()
    );
    let (canonical_report, last_height) = verify_premine_set_against_bundle(
        &cli,
        &mut ctx,
        launcher_id,
        &bundle,
        None,
        VerificationPhase::Canonical,
    )
    .await?;
    println!("{}", canonical_report.to_machine_readable_json()?);
    canonical_report.gate_later_batches()?;
    println!("Canonical Premine set equality OK.");

    let Some(mut anchor_height) = last_height else {
        return Err(CliError::Custom(
            "Could not determine last Premine registration confirmation height".to_string(),
        ));
    };

    loop {
        println!(
            "Waiting for {PREMINE_FINALITY_DEPTH}-block finality above confirmation height {anchor_height}..."
        );
        loop {
            let resp = cli.get_blockchain_state().await?;
            let Some(state) = resp.blockchain_state else {
                return Err(CliError::Custom(
                    "Failed to get blockchain state while waiting for Premine finality".to_string(),
                ));
            };
            if finality_reached(anchor_height, state.peak.height) {
                println!(
                    "Peak height {} reaches finality over confirmation {}.",
                    state.peak.height, anchor_height
                );
                break;
            }
            println!(
                "Peak #{}; need {} more blocks for finality...",
                state.peak.height,
                PREMINE_FINALITY_DEPTH
                    .saturating_sub(state.peak.height.saturating_sub(anchor_height))
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }

        println!("Re-running Premine set-equality verification at finality...");
        let (final_report, current_confirm_height) = verify_premine_set_against_bundle(
            &cli,
            &mut ctx,
            launcher_id,
            &bundle,
            None,
            VerificationPhase::Final,
        )
        .await?;
        println!("{}", final_report.to_machine_readable_json()?);
        final_report.gate_later_batches()?;

        let resp = cli.get_blockchain_state().await?;
        let Some(state) = resp.blockchain_state else {
            return Err(CliError::Custom(
                "Failed to get blockchain state after final Premine verification".to_string(),
            ));
        };
        if !reorganization_invalidates_finality(
            anchor_height,
            current_confirm_height,
            state.peak.height,
        ) {
            println!("Final Premine set equality OK.");
            break;
        }

        let Some(reanchored) = current_confirm_height else {
            return Err(CliError::Custom(
                "reorganization orphaned Premine registrations before finality".to_string(),
            ));
        };
        println!(
            "Reorganization invalidated finality (anchor={anchor_height}, current={reanchored}, peak={}); re-anchoring.",
            state.peak.height
        );
        anchor_height = reanchored;
    }

    if !testnet11 {
        println!(
            "Mainnet royalty constants are enforced via set equality (expected ph={} bps={}).",
            hex::encode(ROYALTY_PUZZLE_HASH),
            ROYALTY_BASIS_POINTS
        );
    }

    println!("Now let's analyze the price singleton.");
    let (multisig_singleton, Some(state_scheduler_info)) =
        sync_multisig_singleton::<XchandlesRegistryState>(
            &cli,
            &mut ctx,
            registry.info.constants.price_singleton_launcher_id,
            None,
        )
        .await?
    else {
        return Err(CliError::Custom(
            "Price singleton was not created correctly.".to_string(),
        ));
    };

    print!(
        "Checking executed price schedule against '{}' (TRUSTED SOURCE)... ",
        price_schedule_csv_filename
    );

    let price_schedule = load_xchandles_state_schedule_csv(price_schedule_csv_filename)?;
    let price_schedule_ok =
        xchandles_trusted_schedule_matches(&price_schedule, &state_scheduler_info.state_schedule);

    if price_schedule_ok {
        println!("OK");
    } else {
        println!("FAILED");
        return Err(CliError::Custom(
            "Price schedule does not match the one defined in the csv.".to_string(),
        ));
    }

    if !testnet11 {
        let expected_controller = MedievalVaultInfo::new(
            registry.info.constants.price_singleton_launcher_id,
            PRICE_SINGLETON_M,
            price_singleton_public_keys()?,
        );
        if Bytes32::from(expected_controller.inner_puzzle_hash())
            != state_scheduler_info.final_puzzle_hash
        {
            return Err(CliError::Custom(
                "Price singleton final controller puzzle hash does not match the ordered configured 6-of-10 validator key set"
                    .to_string(),
            ));
        }
        println!(
            "Committed post-schedule controller matches typed launch configuration ({}-of-{}).",
            PRICE_SINGLETON_M,
            price_singleton_public_keys()?.len()
        );
    }

    match multisig_singleton {
        MultisigSingleton::Vault(vault) => {
            println!("Current (latest unspent) vault info:");
            print_medieval_vault_configuration(vault.info.m, &vault.info.public_key_list)?;
            if !testnet11
                && !controller_matches_configured(vault.info.m, &vault.info.public_key_list)?
            {
                return Err(CliError::Custom(
                    "Live Price Singleton vault controller does not match the ordered configured 6-of-10 validator key set"
                        .to_string(),
                ));
            }
        }
        MultisigSingleton::StateScheduler(state_scheduler) => {
            if state_scheduler.info.generation != 0 {
                println!(
                    "Price singleton is still a price scheduler of generation {}.",
                    state_scheduler.info.generation
                );
            } else {
                return Err(CliError::Custom(
                    "Price singleton has not been unrolled even once.".to_string(),
                ));
            }
        }
    }

    println!("\nEverything seems OK");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chia_protocol::Bytes32;
    use hex_literal::hex;

    fn sample_asset_id() -> Bytes32 {
        Bytes32::new(hex!(
            "d82dd03f8a9ad2f84353cd953c4de6b21dbaaf7de3ba3f4ddd9abe31ecba80ad"
        ))
    }

    fn record(timestamp: u64, price: u64) -> XchandlesStateScheduleRecord {
        XchandlesStateScheduleRecord {
            timestamp,
            asset_id: sample_asset_id(),
            registration_price: price,
            registration_period: 31_557_600,
        }
    }

    fn state(price: u64) -> XchandlesRegistryState {
        XchandlesRegistryState::from(sample_asset_id().tree_hash().into(), price, 31_557_600)
    }

    #[test]
    fn trusted_schedule_matches_full_length_timestamps_and_states() {
        let trusted = vec![record(1, 3), record(2, 2), record(3, 1)];
        let on_chain = vec![(1, state(3)), (2, state(2)), (3, state(1))];
        assert!(xchandles_trusted_schedule_matches(&trusted, &on_chain));
    }

    #[test]
    fn trusted_schedule_rejects_length_mismatch() {
        let trusted = vec![record(1, 3), record(2, 2)];
        let on_chain = vec![(1, state(3)), (2, state(2)), (3, state(1))];
        assert!(!xchandles_trusted_schedule_matches(&trusted, &on_chain));
    }

    #[test]
    fn trusted_schedule_rejects_timestamp_mismatch() {
        let trusted = vec![record(1, 3), record(2, 2), record(3, 1)];
        let on_chain = vec![(1, state(3)), (9, state(2)), (3, state(1))];
        assert!(!xchandles_trusted_schedule_matches(&trusted, &on_chain));
    }

    #[test]
    fn trusted_schedule_rejects_state_mismatch() {
        let trusted = vec![record(1, 3), record(2, 2), record(3, 1)];
        let on_chain = vec![(1, state(3)), (2, state(99)), (3, state(1))];
        assert!(!xchandles_trusted_schedule_matches(&trusted, &on_chain));
    }
}

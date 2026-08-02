use chia_protocol::{Bytes32, Coin};
use chia_puzzle_types::singleton::{LauncherSolution, SingletonSolution};
use chia_puzzle_types::Memos;
use chia_wallet_sdk::driver::{
    HashedPtr, Layer, MedievalVaultInfo, Nft, Puzzle, SingletonInfo, XchandlesExpirePricingPuzzle,
    XchandlesRegistry, XchandlesRegistryReceivedMessagePrefix, XchandlesRegistryState,
};
use chia_wallet_sdk::types::puzzles::{
    DefaultCatMakerArgs, XchandlesFactorPricingPuzzleArgs, XchandlesRegisterActionSolution,
};
use chia_wallet_sdk::types::{Condition, Conditions, Mod};
use chia_wallet_sdk::utils::Address;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{ActionLayer, SpendContext},
};
use clvm_traits::clvm_list;
use clvm_utils::ToTreeHash;
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    controller_matches_configured, get_coinset_client, get_prefix, hex_string_to_bytes32,
    load_xchandles_premine_csv, load_xchandles_state_schedule_csv, metadata_for_handle_nft,
    price_singleton_public_keys, print_medieval_vault_configuration, CliError, MultisigSingleton,
    XchandlesStateScheduleRecord, PRICE_SINGLETON_M, ROYALTY_BASIS_POINTS, ROYALTY_PUZZLE_HASH,
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

    let premine_csv_filename = if testnet11 {
        "xchandles_premine_testnet11.csv"
    } else {
        "xchandles_premine_mainnet.csv"
    };

    println!("Verifying XCHandles deployment (testnet: {})...", testnet11);

    println!("Let's start with the XCHandles registry.");
    println!(
        "It should also have a premine that matches the one defined in '{}'(TRUSTED SOURCE).",
        premine_csv_filename
    );

    let handles_to_launch = load_xchandles_premine_csv(premine_csv_filename)?;

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

    let Some((mut registry, _initial_slots, initial_registration_asset_id, initial_base_price)) =
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

    let mut handle_index = 0;
    let mut royalty_puzzle_hash: Option<Bytes32> = None;
    let mut royalty_basis_points: Option<u16> = None;
    let mut eve_nft_temp_inner_ph: Option<Bytes32> = None;

    while handle_index < handles_to_launch.len() {
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

        let solution = node_from_bytes(&mut ctx, &coin_spend.solution)?;
        let parsed_solution = ctx.extract::<SingletonSolution<NodePtr>>(solution)?;
        let inner_solution = ActionLayer::<XchandlesRegistryState, NodePtr>::parse_solution(
            &ctx,
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

            let nft_launcher_id = action_solution
                .other_precommit_data
                .launcher_ids
                .owner_launcher_id;
            if action_solution.handle_hash
                != handles_to_launch[handle_index].handle.tree_hash().into()
                || action_solution
                    .other_precommit_data
                    .launcher_ids
                    .resolved_launcher_id
                    != nft_launcher_id
            {
                return Err(CliError::Custom(format!(
                    "Wrong handle registered at index {}",
                    handle_index
                )));
            }

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
            let puzzle = Puzzle::parse(&ctx, puzzle_ptr);
            let solution_ptr = ctx.alloc(&eve_nft_spend.solution)?;

            let Some((eve_nft, inner_puzzle, inner_solution_ptr)) =
                Nft::parse(&ctx, eve_nft_coin, puzzle, solution_ptr)?
            else {
                return Err(CliError::Custom(format!(
                    "Could not parse eve nft coin {}",
                    hex::encode(eve_nft_coin.coin_id())
                )));
            };

            // First, verify NFT info, starting from constants 'deduced' from the first NFT.

            if royalty_puzzle_hash.is_none() {
                println!(
                    " Royalty address: {}",
                    Address::new(eve_nft.info.royalty_puzzle_hash, get_prefix(testnet11))
                        .encode()?
                );
                royalty_puzzle_hash = Some(eve_nft.info.royalty_puzzle_hash);
            } else if royalty_puzzle_hash != Some(eve_nft.info.royalty_puzzle_hash) {
                return Err(CliError::Custom(format!(
                    "Royalty puzzle hash mismatch for handle #{}",
                    handle_index
                )));
            }

            if royalty_basis_points.is_none() {
                println!(
                    " Royalty basis points: {} BPS",
                    eve_nft.info.royalty_basis_points
                );
                royalty_basis_points = Some(eve_nft.info.royalty_basis_points);
            } else if royalty_basis_points != Some(eve_nft.info.royalty_basis_points) {
                return Err(CliError::Custom(format!(
                    "Royalty basis points mismatch for handle #{}",
                    handle_index
                )));
            }

            let inner_puzzle_hash: Bytes32 = inner_puzzle.tree_hash().into();
            if eve_nft_temp_inner_ph.is_none() {
                println!(
                    " Temporary eve NFT address: {}",
                    Address::new(inner_puzzle_hash, get_prefix(testnet11)).encode()?
                );
                eve_nft_temp_inner_ph = Some(inner_puzzle_hash);
            } else if eve_nft_temp_inner_ph != Some(inner_puzzle_hash) {
                return Err(CliError::Custom(format!(
                    "Inner puzzle hash mismatch for handle #{}",
                    handle_index
                )));
            }

            // Then, check metadata.
            let expected_metadata_hash = metadata_for_handle_nft(handles_to_launch[handle_index].clone())
                .tree_hash();
            if expected_metadata_hash != eve_nft.info.metadata.tree_hash() {
                return Err(CliError::Custom(format!(
                    "Metadata hash mismatch for handle #{}",
                    handle_index
                )));
            }

            // Lastly, check p2 output only contains the SEND_MESSAGE to the registry
            //   as well as the correct re-creation condition (and the sig check)
            let p2_output = ctx.run(inner_puzzle.ptr(), inner_solution_ptr)?;
            let p2_output = ctx.extract::<Conditions<HashedPtr>>(p2_output)?;
            let p2_output = p2_output.into_vec();

            if p2_output.len() != 3 {
                return Err(CliError::Custom(format!(
                    "P2 output contains {} conditions, expected 3 for handle #{}",
                    p2_output.len(),
                    handle_index
                )));
            }

            let (
                Condition::<HashedPtr>::AggSigMe(ref _cond),
                Condition::<HashedPtr>::SendMessage(ref send_message),
                Condition::<HashedPtr>::CreateCoin(ref create_coin),
            ) = (&p2_output[0], &p2_output[1], &p2_output[2])
            else {
                return Err(CliError::Custom(format!(
                    "P2 output does not contain the correct conditions for handle #{}",
                    handle_index
                )));
            };

            let target_ph = handles_to_launch[handle_index].recipient;
            let Memos::Some(memos) = create_coin.memos else {
                return Err(CliError::Custom(format!(
                    "P2 output does not contain the correct memo for handle #{}",
                    handle_index
                )));
            };
            if create_coin.puzzle_hash != target_ph
                || create_coin.amount != 1
                || memos.tree_hash() != clvm_list!(target_ph).tree_hash()
            {
                return Err(CliError::Custom(format!(
                    "P2 output does not contain the correct recreation condition for handle #{}",
                    handle_index
                )));
            }

            if send_message.mode != 18
                || send_message.message[0]
                    != XchandlesRegistryReceivedMessagePrefix::RegisterOwner as u8
                || send_message.data.len() != 1
                || send_message.data[0].tree_hash() != registry.coin.puzzle_hash.tree_hash()
            {
                return Err(CliError::Custom(format!(
                    "P2 send message mode mismatch for handle #{}",
                    handle_index
                )));
            }

            handle_index += 1;
        }

        registry = registry.child(registry.pending_spend.latest_state.1);
    }

    if handle_index < handles_to_launch.len() {
        return Err(CliError::Custom(
            "XCHandles registry not completely unrolled".to_string(),
        ));
    } else {
        println!("All premined handles were registered correctly.");
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
    let price_schedule_ok = xchandles_trusted_schedule_matches(
        &price_schedule,
        &state_scheduler_info.state_schedule,
    );

    if price_schedule_ok {
        println!("OK");
    } else {
        println!("FAILED");
        return Err(CliError::Custom(
            "Price schedule does not match the one defined in the csv.".to_string(),
        ));
    }

    if !testnet11 {
        let Some(royalty_ph) = royalty_puzzle_hash else {
            return Err(CliError::Custom(
                "Could not determine royalty puzzle hash from premine NFTs".to_string(),
            ));
        };
        let Some(royalty_bps) = royalty_basis_points else {
            return Err(CliError::Custom(
                "Could not determine royalty basis points from premine NFTs".to_string(),
            ));
        };
        if royalty_ph != ROYALTY_PUZZLE_HASH || royalty_bps != ROYALTY_BASIS_POINTS {
            return Err(CliError::Custom(format!(
                "Mainnet royalty mismatch: expected ph={} bps={}, got ph={} bps={}",
                hex::encode(ROYALTY_PUZZLE_HASH),
                ROYALTY_BASIS_POINTS,
                hex::encode(royalty_ph),
                royalty_bps
            )));
        }
        println!(
            "Mainnet royalty constants match typed launch configuration ({} BPS).",
            ROYALTY_BASIS_POINTS
        );
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

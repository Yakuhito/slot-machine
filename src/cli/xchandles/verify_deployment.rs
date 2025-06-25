use chia::clvm_utils::ToTreeHash;
use chia_puzzle_types::singleton::SingletonSolution;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{Layer, SpendContext},
    utils::Address,
};
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    get_coinset_client, hex_string_to_bytes32, load_xchandles_premine_csv,
    load_xchandles_state_schedule_csv, print_medieval_vault_configuration, ActionLayer, CliError,
    DefaultCatMakerArgs, MultisigSingleton, XchandlesExponentialPremiumRenewPuzzleArgs,
    XchandlesFactorPricingPuzzleArgs, XchandlesRegisterActionSolution, XchandlesRegistry,
    XchandlesRegistryState,
};

use crate::sync_multisig_singleton;

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

            let nft_launcher_id =
                Address::decode(&handles_to_launch[handle_index].owner_nft)?.puzzle_hash;
            if action_solution.handle_hash
                != handles_to_launch[handle_index].handle.tree_hash().into()
                || action_solution.data.owner_launcher_id != nft_launcher_id
                || action_solution.data.resolved_data != nft_launcher_id.into()
            {
                return Err(CliError::Custom(format!(
                    "Wrong handle registered at index {}",
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
    let mut price_schedule_ok = true;
    for (i, record) in price_schedule.iter().enumerate() {
        let (block, state) = state_scheduler_info.state_schedule[i];
        if record.block_height != block
            || state.pricing_puzzle_hash
                != XchandlesFactorPricingPuzzleArgs::curry_tree_hash(
                    record.registration_price,
                    record.registration_period,
                )
                .into()
            || state.expired_handle_pricing_puzzle_hash
                != XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(
                    record.registration_price,
                    record.registration_period,
                    1000,
                )
                .into()
            || state.cat_maker_puzzle_hash
                != DefaultCatMakerArgs::curry_tree_hash(record.asset_id.tree_hash().into()).into()
        {
            price_schedule_ok = false;
            break;
        }
    }

    if price_schedule_ok {
        println!("OK");
    } else {
        println!("FAILED");
        return Err(CliError::Custom(
            "Price schedule does not match the one defined in the csv.".to_string(),
        ));
    }

    match multisig_singleton {
        MultisigSingleton::Vault(vault) => {
            println!("Current (latest unspent) vault info:");
            print_medieval_vault_configuration(vault.info.m, &vault.info.public_key_list)?;
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

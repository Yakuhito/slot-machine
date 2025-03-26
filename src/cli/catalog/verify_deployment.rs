use chia::clvm_utils::ToTreeHash;
use chia::protocol::{Bytes32, Coin};
use chia::puzzles::singleton::{LauncherSolution, SINGLETON_LAUNCHER_PUZZLE_HASH};
use chia_wallet_sdk::{ChiaRpcClient, Condition, Conditions, DriverError, SpendContext};
use clvm_traits::FromClvm;
use clvmr::serde::node_from_bytes;
use clvmr::NodePtr;

use crate::{
    get_coinset_client, load_catalog_state_schedule_csv, print_medieval_vault_configuration,
    CatalogRegistryConstants, CatalogRegistryState, CliError, DefaultCatMakerArgs,
    MultisigSingleton,
};

use crate::sync_multisig_singleton;

pub async fn catalog_verify_deployment(testnet11: bool) -> Result<(), CliError> {
    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);
    let catalog_constants = CatalogRegistryConstants::get(testnet11);

    println!("Verifying CATalog deployment (testnet: {})...", testnet11);

    let premine_csv_filename = if testnet11 {
        "catalog_premine_testnet11.csv"
    } else {
        "catalog_premine_mainnet.csv"
    };

    println!("Let's start with the CATalog registry.");
    println!("It should have the right constants (defined in this lib; TRUSTED SOURCE).");
    println!(
        "It should also have a premine that matches the one defined in '{}'(TRUSTED SOURCE).",
        premine_csv_filename
    );

    let Some(launcher_coin_record) = cli
        .get_coin_record_by_name(catalog_constants.launcher_id)
        .await?
        .coin_record
    else {
        return Err(CliError::CoinNotFound(catalog_constants.launcher_id));
    };

    let Some(launcher_coin_solution) = cli
        .get_puzzle_and_solution(
            catalog_constants.launcher_id,
            Some(launcher_coin_record.spent_block_index),
        )
        .await?
        .coin_solution
    else {
        return Err(CliError::CoinNotSpent(catalog_constants.launcher_id));
    };

    let launcher_puzzle_ptr =
        node_from_bytes(&mut ctx.allocator, &launcher_coin_solution.puzzle_reveal)?;
    let launcher_solution_ptr =
        node_from_bytes(&mut ctx.allocator, &launcher_coin_solution.solution)?;

    let output_conds = ctx.run(launcher_puzzle_ptr, launcher_solution_ptr)?;
    let output_conds = Conditions::<NodePtr>::from_clvm(&mut ctx.allocator, output_conds)
        .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;
    let create_coin_cond = output_conds
        .into_iter()
        .find_map(|cond| {
            if let Condition::CreateCoin(create_coin_cond) = cond {
                if create_coin_cond.amount == 1 {
                    Some(create_coin_cond)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap();

    let eve_catalog_coin = Coin::new(
        catalog_constants.launcher_id,
        create_coin_cond.puzzle_hash,
        1,
    );

    let Some(eve_coin_record) = cli
        .get_coin_record_by_name(eve_catalog_coin.coin_id())
        .await?
        .coin_record
    else {
        return Err(CliError::CoinNotFound(eve_catalog_coin.coin_id()));
    };

    let Some(eve_coin_spend) = cli
        .get_puzzle_and_solution(
            eve_catalog_coin.coin_id(),
            Some(eve_coin_record.spent_block_index),
        )
        .await?
        .coin_solution
    else {
        return Err(CliError::CoinNotSpent(eve_catalog_coin.coin_id()));
    };

    let eve_puzzle_ptr = node_from_bytes(&mut ctx.allocator, &eve_coin_spend.puzzle_reveal)?;
    let (_, conditions) = <(u64, Conditions<NodePtr>)>::from_clvm(&ctx.allocator, eve_puzzle_ptr)
        .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;

    // let (hinted_launcher_id, (initial_registration_asset_id, (initial_state, ()))) =
    //     launcher_solution.key_value_list;
    // if hinted_launcher_id != catalog_constants.launcher_id
    //     || initial_state.registration_price != 1
    //     || initial_state.cat_maker_puzzle_hash || launcher_coin_record.coin.puzzle_hash != SINGLETON_LAUNCHER_PUZZLE_HASH.into()
    //         != DefaultCatMakerArgs::curry_tree_hash(
    //             initial_registration_asset_id.tree_hash().into(),
    //         )
    //         .into()
    // {
    //     return Err(CliError::Custom(
    //         "Launcher key_vaule_list not ok".to_string(),
    //     ));
    // }

    // println!(
    //     "Registry launched at height {} with a premine registration CAT asset id of {}.",
    //     launcher_coin_record.spent_block_index,
    //     hex::encode(initial_registration_asset_id)
    // );

    println!("Now let's analyze the price singleton.");
    let (MultisigSingleton::Vault(my_vault), Some(state_scheduler_info)) =
        sync_multisig_singleton::<CatalogRegistryState>(
            &cli,
            &mut ctx,
            catalog_constants.launcher_id,
            None,
        )
        .await?
    else {
        return Err(CliError::Custom(
            "Price singleton was not created correctly or is still in its state scheduler phase."
                .to_string(),
        ));
    };

    let price_schedule_csv_filename = if testnet11 {
        "catalog_price_schedule_testnet11.csv"
    } else {
        "catalog_price_schedule_mainnet.csv"
    };
    print!(
        "Checking executed price schedule against '{}' (TRUSTED SOURCE)... ",
        price_schedule_csv_filename
    );

    let price_schedule = load_catalog_state_schedule_csv(price_schedule_csv_filename)?;
    let mut price_schedule_ok = true;
    for (i, record) in price_schedule.iter().enumerate() {
        let (block, state) = state_scheduler_info.state_schedule[i];
        if record.block_height != block
            || record.registration_price != state.registration_price
            || record.asset_id
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

    println!("Current (latest unspent) vault info:");
    print_medieval_vault_configuration(my_vault.info.m, &my_vault.info.public_key_list)?;

    Ok(())
}

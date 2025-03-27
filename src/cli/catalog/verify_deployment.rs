use chia::clvm_utils::{CurriedProgram, ToTreeHash};
use chia::protocol::{Bytes32, Coin};
use chia::puzzles::nft::{
    NftOwnershipLayerArgs, NftRoyaltyTransferPuzzleArgs, NftStateLayerArgs,
    NFT_STATE_LAYER_PUZZLE_HASH,
};
use chia::puzzles::singleton::{SingletonArgs, SINGLETON_LAUNCHER_PUZZLE_HASH};
use chia::puzzles::{LineageProof, Proof};
use chia_wallet_sdk::{
    ChiaRpcClient, Condition, Conditions, DriverError, Layer, Puzzle, SingletonLayer, SpendContext,
};
use clvm_traits::FromClvm;
use clvmr::serde::node_from_bytes;
use clvmr::NodePtr;

use crate::{
    get_coinset_client, initial_cat_inner_puzzle_ptr, load_catalog_premine_csv,
    load_catalog_state_schedule_csv, print_medieval_vault_configuration, CatalogRegistry,
    CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState, CatalogSlotValue,
    CliError, DefaultCatMakerArgs, MultisigSingleton, Slot, SlotInfo, UniquenessPrelauncher,
    ANY_METADATA_UPDATER_HASH, SLOT32_MAX_VALUE, SLOT32_MIN_VALUE,
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

    let cats_to_launch = load_catalog_premine_csv(premine_csv_filename)?;

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
    let output_conds = Conditions::<NodePtr>::from_clvm(&ctx.allocator, output_conds)
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

    let eve_puzzle = node_from_bytes(&mut ctx.allocator, &eve_coin_spend.puzzle_reveal)?;
    let eve_singleton_layer = SingletonLayer::<NodePtr>::parse_puzzle(
        &ctx.allocator,
        Puzzle::parse(&ctx.allocator, eve_puzzle),
    )?
    .unwrap();
    let (_, conditions) =
        <(u64, Conditions<NodePtr>)>::from_clvm(&ctx.allocator, eve_singleton_layer.inner_puzzle)
            .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;

    let conditions: Vec<Condition<NodePtr>> = conditions.into_iter().collect();
    let [Condition::CreateCoin(left_slot_cc), Condition::CreateCoin(right_slot_cc), Condition::CreateCoin(catalog_cc)] =
        conditions.as_slice()
    else {
        return Err(CliError::Custom(
            "Eve puzzle does not have the right conditions".to_string(),
        ));
    };

    let left_slot_info = SlotInfo::from_value(
        catalog_constants.launcher_id,
        0,
        CatalogSlotValue::left_end(SLOT32_MAX_VALUE.into()),
    );
    let right_slot_info = SlotInfo::from_value(
        catalog_constants.launcher_id,
        0,
        CatalogSlotValue::right_end(SLOT32_MIN_VALUE.into()),
    );

    if left_slot_cc.puzzle_hash != Slot::puzzle_hash(&left_slot_info).into()
        || right_slot_cc.puzzle_hash != Slot::puzzle_hash(&right_slot_info).into()
    {
        return Err(CliError::Custom(
            "Left/right end slot puzzle hashes do not match expected values".to_string(),
        ));
    }

    let (hinted_launcher_id, (initial_registration_asset_id, (initial_state, ()))) =
        <(Bytes32, (Bytes32, (CatalogRegistryState, ())))>::from_clvm(
            &ctx.allocator,
            catalog_cc.memos.unwrap().value,
        )
        .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;

    let catalog_info = CatalogRegistryInfo::new(initial_state, catalog_constants);
    let catalog_full_ph = SingletonArgs::curry_tree_hash(
        catalog_constants.launcher_id,
        catalog_info.inner_puzzle_hash(),
    );
    let mut catalog = CatalogRegistry::new(
        Coin::new(eve_catalog_coin.coin_id(), catalog_full_ph.into(), 1),
        Proof::Lineage(LineageProof {
            parent_parent_coin_info: eve_catalog_coin.parent_coin_info,
            parent_inner_puzzle_hash: ctx.tree_hash(eve_singleton_layer.inner_puzzle).into(),
            parent_amount: 1,
        }),
        catalog_info,
    );

    if hinted_launcher_id != catalog_constants.launcher_id
        || initial_state.registration_price != 1
        || initial_state.cat_maker_puzzle_hash
            != DefaultCatMakerArgs::curry_tree_hash(
                initial_registration_asset_id.tree_hash().into(),
            )
            .into()
        || launcher_coin_record.coin.puzzle_hash != SINGLETON_LAUNCHER_PUZZLE_HASH.into()
        || catalog_cc.amount != 1
        || catalog_cc.puzzle_hash != catalog_info.inner_puzzle_hash().into()
    {
        return Err(CliError::Custom(
            "Eve coin CATalog create_coin not ok".to_string(),
        ));
    }

    println!(
        "Registry launched at height {} with a premine registration CAT asset id of {}.",
        launcher_coin_record.spent_block_index,
        hex::encode(initial_registration_asset_id)
    );

    let mut cat_index = 0;

    while cat_index < cats_to_launch.len() {
        let Some(coin_record) = cli
            .get_coin_record_by_name(catalog.coin.coin_id())
            .await?
            .coin_record
        else {
            return Err(CliError::CoinNotFound(catalog.coin.coin_id()));
        };

        let Some(coin_spend) = cli
            .get_puzzle_and_solution(catalog.coin.coin_id(), Some(coin_record.spent_block_index))
            .await?
            .coin_solution
        else {
            break;
        };

        let solution = node_from_bytes(&mut ctx.allocator, &coin_spend.solution)?;
        let new_slots = catalog.get_new_slots_from_spend(&mut ctx, solution)?;

        while cat_index < cats_to_launch.len() {
            let top_cat = &cats_to_launch[cat_index];
            let found = new_slots
                .iter()
                .find(|slot| slot.info.value.unwrap().asset_id == top_cat.asset_id);
            if found.is_some() {
                cat_index += 1;

                let eve_nft_inner_puzzle = initial_cat_inner_puzzle_ptr(&mut ctx, top_cat)?;
                let eve_nft_inner_puzzle_hash = ctx.tree_hash(eve_nft_inner_puzzle);

                let uniqueness_prelauncher_coin = Coin::new(
                    catalog.coin.coin_id(),
                    UniquenessPrelauncher::<()>::puzzle_hash(top_cat.asset_id.tree_hash()).into(),
                    0,
                );
                let cat_nft_launcher = Coin::new(
                    uniqueness_prelauncher_coin.coin_id(),
                    SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
                    1,
                );
                let cat_nft_launcher_id = cat_nft_launcher.coin_id();

                let cat_nft_puzzle_hash = SingletonArgs::curry_tree_hash(
                    cat_nft_launcher_id,
                    CurriedProgram {
                        program: NFT_STATE_LAYER_PUZZLE_HASH,
                        args: NftStateLayerArgs {
                            mod_hash: NFT_STATE_LAYER_PUZZLE_HASH.into(),
                            metadata: (),
                            metadata_updater_puzzle_hash: ANY_METADATA_UPDATER_HASH.into(),
                            inner_puzzle: NftOwnershipLayerArgs::curry_tree_hash(
                                None,
                                NftRoyaltyTransferPuzzleArgs::curry_tree_hash(
                                    cat_nft_launcher_id,
                                    catalog_constants.royalty_address,
                                    catalog_constants.royalty_ten_thousandths,
                                ),
                                eve_nft_inner_puzzle_hash,
                            ),
                        },
                    }
                    .tree_hash(),
                );

                let eve_cat_nft_coin =
                    Coin::new(cat_nft_launcher_id, cat_nft_puzzle_hash.into(), 1);
                let Some(eve_cat_nft_record) = cli
                    .get_coin_record_by_name(eve_cat_nft_coin.coin_id())
                    .await?
                    .coin_record
                else {
                    return Err(CliError::CoinNotFound(eve_cat_nft_coin.coin_id()));
                };
                if !eve_cat_nft_record.spent {
                    return Err(CliError::CoinNotSpent(eve_cat_nft_coin.coin_id()));
                }
            } else {
                break;
            }
        }

        let puzzle_ptr = node_from_bytes(&mut ctx.allocator, &coin_spend.puzzle_reveal)?;
        let parent_puzzle = Puzzle::parse(&ctx.allocator, puzzle_ptr);
        catalog = CatalogRegistry::from_parent_spend(
            &mut ctx.allocator,
            catalog.coin,
            parent_puzzle,
            solution,
            catalog.info.constants,
        )?
        .unwrap();
    }

    if cat_index < cats_to_launch.len() {
        return Err(CliError::Custom(
            "CATalog not completely unrolled".to_string(),
        ));
    } else {
        println!("All premine CATs were distributed correctly.");
    }

    println!("Now let's analyze the price singleton.");
    let (MultisigSingleton::Vault(my_vault), Some(state_scheduler_info)) =
        sync_multisig_singleton::<CatalogRegistryState>(
            &cli,
            &mut ctx,
            catalog_constants.price_singleton_launcher_id,
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

    println!("\nEverything seems OK");

    Ok(())
}

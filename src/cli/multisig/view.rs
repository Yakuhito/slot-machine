use chia::puzzles::singleton::LauncherSolution;
use chia_wallet_sdk::{ChiaRpcClient, DriverError, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    get_alias_map, get_coinset_client, hex_string_to_bytes32, CatalogRegistryState, CliError,
    MedievalVaultHint, MedievalVaultInfo, StateScheduler, StateSchedulerInfo,
    XchandlesRegistryState,
};

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(transparent)]
pub enum StateSchedulerHintedState {
    Catalog(CatalogRegistryState),
    Xchandles(XchandlesRegistryState),
}

pub async fn multisig_view(launcher_id_str: String, testnet11: bool) -> Result<(), CliError> {
    let mut ctx = SpendContext::new();
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let cli = get_coinset_client(testnet11);

    println!("Viewing vault...");
    let launcher_coin_record = cli.get_coin_record_by_name(launcher_id).await?;
    let Some(launcher_coin_record) = launcher_coin_record.coin_record else {
        return Err(CliError::CoinNotFound(launcher_id));
    };
    if !launcher_coin_record.spent {
        return Err(CliError::CoinNotSpent(launcher_id));
    }

    let launcher_spend = cli
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?;
    let Some(launcher_spend) = launcher_spend.coin_solution else {
        return Err(CliError::CoinNotSpent(launcher_id));
    };

    let launcher_solution_ptr = node_from_bytes(&mut ctx.allocator, &launcher_spend.solution)?;
    let launcher_solution =
        LauncherSolution::<NodePtr>::from_clvm(&ctx.allocator, launcher_solution_ptr)
            .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;

    let Some((state_scheduler_info, medieval_vault_hint)) = StateSchedulerInfo::<
        StateSchedulerHintedState,
    >::from_launcher_solution::<
        MedievalVaultHint,
    >(
        &mut ctx.allocator,
        launcher_solution,
    )?
    else {
        return Err(CliError::Custom(
            "Vault not launched as a state scheduler singleton".to_string(),
        ));
    };

    let target_vault_info = MedievalVaultInfo::from_hint(medieval_vault_hint);
    let target_puzzle_hash = target_vault_info.inner_puzzle_hash();
    if target_puzzle_hash != state_scheduler_info.final_puzzle_hash.into() {
        return Err(CliError::Custom("Singleton hinted incorrectly".to_string()));
    }

    println!("Vault launched as a state scheduler first. Schedule: ");
    for (block, state) in state_scheduler_info.state_schedule {
        match state {
            StateSchedulerHintedState::Catalog(catalog_state) => {
                println!(
                    "  After block {}, price will be {} mojos with a CAT maker puzzle hash of {}.",
                    block,
                    catalog_state.registration_price,
                    hex::encode(catalog_state.cat_maker_puzzle_hash),
                );
            }
            StateSchedulerHintedState::Xchandles(xchandles_state) => {
                println!(
                    "  After block {}, the CAT maker puzzle hash will be {}, the pricing puzzle hash will be {}, and the expired handle pricing puzzle hash will be {}.",
                    block,
                    hex::encode(xchandles_state.cat_maker_puzzle_hash),
                    hex::encode(xchandles_state.pricing_puzzle_hash),
                    hex::encode(xchandles_state.expired_handle_pricing_puzzle_hash),
                );
            }
        }
    }

    println!("\nInitial medieval vault configuration: ");

    let alias_map = get_alias_map()?;

    println!("  Public Key List:");
    for pubkey in target_vault_info.public_key_list.iter() {
        println!(
            "    - {}",
            alias_map
                .get(pubkey)
                .unwrap_or(&format!("0x{}", hex::encode(pubkey.to_bytes())))
        );
    }
    println!("  Signature Threshold: {}", target_vault_info.m);

    println!("\nFollowing coin on-chain...");

    let Some(mut state_scheduler) =
        StateScheduler::<StateSchedulerHintedState>::from_launcher_spend(&mut ctx, launcher_spend)?
    else {
        return Err(CliError::Custom(
            "Failed to parse state scheduler".to_string(),
        ));
    };

    loop {
        let coin_record = cli
            .get_coin_record_by_name(state_scheduler.coin.coin_id())
            .await?;

        let Some(coin_record) = coin_record.coin_record else {
            return Err(CliError::CoinNotFound(state_scheduler.coin.coin_id()));
        };

        if !coin_record.spent {
            println!("Latest state scheduler coin not spent.");
            break;
        }

        if let Some(child) = state_scheduler.child() {
            state_scheduler = child;
            println!(
                "State scheduler spent to update state to one after block {}.",
                state_scheduler.info.state_schedule[state_scheduler.info.generation - 1].0
            );
        } else {
            println!("State scheduler phase finished - next coin will be a vault.");
            break;
        }
    }

    Ok(())
}

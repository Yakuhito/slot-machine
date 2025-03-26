use chia::puzzles::singleton::LauncherSolution;
use chia_wallet_sdk::{ChiaRpcClient, DriverError, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{serde::node_from_bytes, NodePtr};

use crate::{
    get_coinset_client, hex_string_to_bytes32, CatalogRegistryState, CliError, MedievalVaultHint,
    MedievalVaultInfo, StateSchedulerInfo, XchandlesRegistryState,
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

    println!("All good so far.");

    Ok(())
}

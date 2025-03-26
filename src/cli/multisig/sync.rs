use chia::{
    bls::PublicKey, clvm_utils::ToTreeHash, protocol::Bytes32, puzzles::singleton::LauncherSolution,
};
use chia_wallet_sdk::{ChiaRpcClient, CoinsetClient, DriverError, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{serde::node_from_bytes, Allocator, NodePtr};

use crate::{
    get_alias_map, CliError, MedievalVault, MedievalVaultHint, MedievalVaultInfo, StateScheduler,
    StateSchedulerInfo,
};

pub enum MultisigSingleton<S>
where
    S: Clone + ToClvm<Allocator> + FromClvm<Allocator> + ToTreeHash,
{
    StateScheduler(StateScheduler<S>),
    Vault(MedievalVault),
}

pub fn print_medieval_vault_configuration(m: usize, pubkeys: &[PublicKey]) -> Result<(), CliError> {
    let alias_map = get_alias_map()?;

    println!("  Public Key List:");
    for pubkey in pubkeys.iter() {
        println!(
            "    - {}",
            alias_map
                .get(pubkey)
                .unwrap_or(&format!("0x{}", hex::encode(pubkey.to_bytes())))
        );
    }
    println!("  Signature Threshold: {}", m);

    Ok(())
}

// returns object representing last coin, which is either a StateScheduler or a MedievalVault
// second object will contain verified state scheduler info *IF* the multisig had an initial state scheduler phase
//  note that the state scheduler info will be returned even if the state scheduler phase is over
//  (i.e., the last coin is a vault)
pub async fn sync_multisig_singleton<S>(
    client: &CoinsetClient,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    print_state_info: Option<fn(u32, &S) -> Result<(), CliError>>,
) -> Result<(MultisigSingleton<S>, Option<StateSchedulerInfo<S>>), CliError>
where
    S: Clone + ToClvm<Allocator> + FromClvm<Allocator> + ToTreeHash,
{
    let print_sync = print_state_info.is_some();

    let launcher_coin_record = client.get_coin_record_by_name(launcher_id).await?;
    let Some(launcher_coin_record) = launcher_coin_record.coin_record else {
        return Err(CliError::CoinNotFound(launcher_id));
    };
    if !launcher_coin_record.spent {
        return Err(CliError::CoinNotSpent(launcher_id));
    }

    let launcher_spend = client
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?;
    let Some(launcher_spend) = launcher_spend.coin_solution else {
        return Err(CliError::CoinNotSpent(launcher_id));
    };

    let launcher_solution_ptr = node_from_bytes(&mut ctx.allocator, &launcher_spend.solution)?;
    let launcher_solution =
        LauncherSolution::<NodePtr>::from_clvm(&ctx.allocator, launcher_solution_ptr)
            .map_err(|err| CliError::Driver(DriverError::FromClvm(err)))?;

    let parsed_state_scheduler_info = StateSchedulerInfo::<S>::from_launcher_solution::<
        MedievalVaultHint,
    >(&mut ctx.allocator, launcher_solution)?;
    if let Some((state_scheduler_info, medieval_vault_hint)) = parsed_state_scheduler_info {
        let target_vault_info = MedievalVaultInfo::from_hint(medieval_vault_hint);
        let target_puzzle_hash = target_vault_info.inner_puzzle_hash();
        if target_puzzle_hash != state_scheduler_info.final_puzzle_hash.into() {
            return Err(CliError::Custom("Singleton hinted incorrectly".to_string()));
        }

        if let Some(print_state_info) = print_state_info {
            println!("Vault launched as a state scheduler first. Schedule: ");
            for (block, state) in state_scheduler_info.state_schedule.clone() {
                print_state_info(block, &state)?;
            }

            println!("\nInitial medieval vault configuration: ");
            print_medieval_vault_configuration(
                target_vault_info.m,
                &target_vault_info.public_key_list,
            )?;

            println!("\nFollowing state scheduler on-chain...");
        }

        let Some(mut state_scheduler) =
            StateScheduler::<S>::from_launcher_spend(ctx, launcher_spend)?
        else {
            return Err(CliError::Custom(
                "Failed to parse state scheduler".to_string(),
            ));
        };

        let mut coin_record;
        loop {
            coin_record = client
                .get_coin_record_by_name(state_scheduler.coin.coin_id())
                .await?;

            let Some(coin_record) = coin_record.coin_record else {
                return Err(CliError::CoinNotFound(state_scheduler.coin.coin_id()));
            };

            if !coin_record.spent {
                if print_sync {
                    println!("Latest state scheduler coin not spent.");
                }
                break;
            }

            if let Some(child) = state_scheduler.child() {
                state_scheduler = child;
                if print_sync {
                    println!(
                        "State scheduler spent to update state to one after block {}.",
                        state_scheduler.info.state_schedule[state_scheduler.info.generation - 1].0
                    );
                }
            } else {
                if print_sync {
                    println!("State scheduler phase finished - next coin will be a vault.");
                }
                break;
            }
        }

        Ok((
            MultisigSingleton::StateScheduler(state_scheduler),
            Some(state_scheduler_info),
        ))
    } else {
        return Err(CliError::Custom(
            "Vault not launched as a state scheduler singleton".to_string(),
        ));
    }
}

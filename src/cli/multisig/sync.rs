use chia::{
    bls::PublicKey,
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::{LauncherSolution, SingletonArgs},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::driver::{
    MedievalVaultHint, MedievalVaultInfo, SpendContext, StateSchedulerInfo,
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{MedievalVault, StateScheduler},
};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{serde::node_from_bytes, Allocator, NodePtr};

use crate::{get_alias_map, CliError};

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
#[allow(clippy::type_complexity)]
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

    let launcher_coin_record = client
        .get_coin_record_by_name(launcher_id)
        .await?
        .coin_record
        .ok_or(CliError::CoinNotFound(launcher_id))?;
    if !launcher_coin_record.spent {
        return Err(CliError::CoinNotSpent(launcher_id));
    }

    let launcher_spend = client
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?
        .coin_solution
        .ok_or(CliError::CoinNotSpent(launcher_id))?;

    let launcher_solution_ptr = node_from_bytes(ctx, &launcher_spend.solution)?;
    let launcher_solution = ctx.extract::<LauncherSolution<NodePtr>>(launcher_solution_ptr)?;

    let parsed_state_scheduler_info = StateSchedulerInfo::<S>::from_launcher_solution::<
        MedievalVaultHint,
    >(ctx, launcher_solution);

    let (eve_vault, state_scheduler_info) = if let Ok(Some((
        state_scheduler_info,
        medieval_vault_hint,
    ))) = parsed_state_scheduler_info
    {
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

        let mut state_scheduler = StateScheduler::<S>::from_launcher_spend(ctx, &launcher_spend)?
            .ok_or(CliError::Custom(
            "Failed to parse state scheduler".to_string(),
        ))?;

        loop {
            let coin_record = client
                .get_coin_record_by_name(state_scheduler.coin.coin_id())
                .await?
                .coin_record
                .ok_or(CliError::CoinNotFound(state_scheduler.coin.coin_id()))?;

            if !coin_record.spent {
                if print_sync {
                    println!("Latest state scheduler coin not spent.");
                }

                return Ok((
                    MultisigSingleton::StateScheduler(state_scheduler),
                    Some(state_scheduler_info),
                ));
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

        let new_vault_coin = Coin::new(
            state_scheduler.coin.coin_id(),
            SingletonArgs::curry_tree_hash(
                launcher_id,
                state_scheduler.info.final_puzzle_hash.into(),
            )
            .into(),
            state_scheduler.coin.amount,
        );
        let new_vault_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: state_scheduler.coin.parent_coin_info,
            parent_inner_puzzle_hash: state_scheduler.info.inner_puzzle_hash().into(),
            parent_amount: state_scheduler.coin.amount,
        });
        let eve_vault = MedievalVault::new(new_vault_coin, new_vault_proof, target_vault_info);

        (eve_vault, Some(state_scheduler_info))
    } else {
        let eve_vault = MedievalVault::from_launcher_spend(ctx, &launcher_spend)?.ok_or(
            CliError::Custom("Singleton not a state scheduler nor a vault".to_string()),
        )?;

        if print_sync {
            println!("Singleton directly launched as a vault with info: ");
            print_medieval_vault_configuration(eve_vault.info.m, &eve_vault.info.public_key_list)?;
        }
        (eve_vault, None)
    };

    if print_sync {
        println!("Getting latest vault on-chain...");
    }
    let vault_records = client
        .get_coin_records_by_hint(launcher_id, None, None, Some(false))
        .await?
        .coin_records
        .ok_or(CliError::CoinNotFound(launcher_id))?;

    for vault_record in vault_records {
        if vault_record.spent {
            continue;
        }

        let vault_parent_spend = client
            .get_puzzle_and_solution(
                vault_record.coin.parent_coin_info,
                Some(vault_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(vault_record.coin.parent_coin_info))?;

        if let Some(vault) = MedievalVault::from_parent_spend(ctx, &vault_parent_spend)? {
            return Ok((MultisigSingleton::Vault(vault), state_scheduler_info));
        }
    }

    // couldn't parse medieval vault from parent spend ->
    //   vault either just laucnhed (parent is launcher) or parent is state scheduler
    Ok((MultisigSingleton::Vault(eve_vault), state_scheduler_info))
}

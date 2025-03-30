use chia_wallet_sdk::SpendContext;
use clvm_traits::{FromClvm, ToClvm};

use crate::{
    get_coinset_client, hex_string_to_bytes32, print_medieval_vault_configuration,
    sync_multisig_singleton, CatalogRegistryState, CliError, XchandlesRegistryState,
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

    if let (crate::MultisigSingleton::Vault(current_vault), _info) = sync_multisig_singleton(
        &cli,
        &mut ctx,
        launcher_id,
        Some(|block, state| {
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

            Ok(())
        }),
    ).await? {
        println!("\nCurrent (latest) vault configuration:");
        print_medieval_vault_configuration(current_vault.info.m, &current_vault.info.public_key_list)?;
    } else {
        println!("\nVault still in state scheduler phase.");
    };

    Ok(())
}

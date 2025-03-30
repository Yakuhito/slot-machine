use chia::bls::PublicKey;
use chia_wallet_sdk::SpendContext;
use clvmr::NodePtr;

use crate::{
    get_constants, hex_string_to_pubkey, print_medieval_vault_configuration, CliError,
    MedievalVault,
};

use super::multisig_sign_thing;

async fn summary_and_delegated_puzzle_for_rekey(
    ctx: &mut SpendContext,
    medieval_vault: &MedievalVault,
    my_alias: &String,
    testnet11: bool,
    (new_pubkeys, new_m): (Vec<PublicKey>, usize),
) -> Result<NodePtr, CliError> {
    println!("\nNew configuration:");
    print_medieval_vault_configuration(new_m, &new_pubkeys)?;

    println!("\nYou'll sign this REKEY with the following pubkey:");
    println!("  {}", my_alias);

    MedievalVault::delegated_puzzle_for_rekey(
        ctx,
        medieval_vault.info.launcher_id,
        new_m,
        new_pubkeys,
        medieval_vault.coin.coin_id(),
        get_constants(testnet11).genesis_challenge,
    )
    .map_err(CliError::Driver)
}

pub async fn multisig_sign_rekey(
    new_pubkeys_str: String,
    new_m: usize,
    my_pubkey_str: String,
    launcher_id_str: String,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let mut new_pubkeys = Vec::new();
    for pubkey_str in new_pubkeys_str.split(',') {
        let pubkey = hex_string_to_pubkey(pubkey_str)?;
        new_pubkeys.push(pubkey);
    }
    if new_m > new_pubkeys.len() {
        return Err(CliError::Custom(
            "New m is greater than the number of new pubkeys".to_string(),
        ));
    }

    multisig_sign_thing(
        summary_and_delegated_puzzle_for_rekey,
        (new_pubkeys, new_m),
        my_pubkey_str,
        launcher_id_str,
        testnet11,
        debug,
    )
    .await
}

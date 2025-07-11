use chia_wallet_sdk::driver::MedievalVault;

use crate::{
    get_constants, hex_string_to_pubkey, multisig_sign_thing_finish, multisig_sign_thing_start,
    print_medieval_vault_configuration, CliError,
};

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

    let (my_pubkey, mut ctx, _client, medieval_vault) =
        multisig_sign_thing_start(my_pubkey_str, launcher_id_str, testnet11).await?;

    println!("\nNew configuration:");
    print_medieval_vault_configuration(new_m, &new_pubkeys)?;

    let delegated_puzzle = MedievalVault::delegated_puzzle_for_rekey(
        &mut ctx,
        medieval_vault.info.launcher_id,
        new_m,
        new_pubkeys,
        medieval_vault.coin.coin_id(),
        get_constants(testnet11).genesis_challenge,
    )
    .map_err(CliError::Driver)?;

    multisig_sign_thing_finish(
        &mut ctx,
        delegated_puzzle,
        &medieval_vault,
        my_pubkey,
        testnet11,
        debug,
    )
    .await
}

use crate::{
    get_constants, hex_string_to_pubkey, multisig_broadcast_thing_finish,
    multisig_broadcast_thing_start, print_medieval_vault_configuration, CliError, MedievalVault,
};

pub async fn multisig_broadcast_rekey(
    new_pubkeys_str: String,
    new_m: usize,
    signatures_str: String,
    launcher_id_str: String,
    testnet11: bool,
    fee_str: String,
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

    let (signature_from_signers, pubkeys, client, mut ctx, medieval_vault) =
        multisig_broadcast_thing_start(signatures_str, launcher_id_str, testnet11).await?;

    println!("\nNew configuration:");
    print_medieval_vault_configuration(new_m, &new_pubkeys)?;

    let conditions = MedievalVault::rekey_create_coin_unsafe(
        &mut ctx,
        medieval_vault.info.launcher_id,
        new_m,
        new_pubkeys,
    )?;
    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    medieval_vault.spend(
        &mut ctx,
        &pubkeys,
        conditions,
        get_constants(testnet11).genesis_challenge,
    )?;

    multisig_broadcast_thing_finish(
        client,
        &mut ctx,
        signature_from_signers,
        fee_str,
        testnet11,
        medieval_vault_coin_id,
        None,
    )
    .await
}

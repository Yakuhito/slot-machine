use crate::{
    get_constants, hex_string_to_pubkey, multisig_broadcast_thing_finish,
    multisig_broadcast_thing_start, parse_amount, print_medieval_vault_configuration,
    yes_no_prompt, CliError, MedievalVault,
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

    let fee = parse_amount(&fee_str, false)?;

    println!(
        "A one-sided offer offering 1 mojo and {} XCH ({} mojos) as fee will be generated and broadcast.",
        fee_str,
        fee
    );
    println!("The resulting spend bundle will be automatically submitted to the mempool.");
    yes_no_prompt("Are you COMPLETELY SURE you want to proceed?")?;

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
    )
    .await
}

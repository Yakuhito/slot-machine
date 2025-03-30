use chia::bls::sign;
use chia::protocol::Bytes;
use chia_wallet_sdk::{AggSig, AggSigConstants, AggSigKind, RequiredBlsSignature, SpendContext};
use clvmr::serde::node_to_bytes;

use crate::{
    get_alias_map, get_coinset_client, get_constants, hex_string_to_bytes32, hex_string_to_pubkey,
    hex_string_to_secret_key, parse_amount, print_medieval_vault_configuration, prompt_for_value,
    sync_multisig_singleton, yes_no_prompt, CliError, MedievalVault, MultisigSingleton,
    StateSchedulerHintedState,
};

pub async fn multisig_sign_catalog_state_update(
    new_payment_asset_id_str: String,
    new_payment_asset_amount_str: String,
    my_pubkey_str: String,
    launcher_id_str: String,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    let my_pubkey = hex_string_to_pubkey(&my_pubkey_str)?;
    let new_payment_asset_id = hex_string_to_bytes32(&new_payment_asset_id_str)?;
    let new_payment_asset_amount = parse_amount(new_payment_asset_amount_str, true)?;

    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    println!("Syncing multisig...");
    let client = get_coinset_client(testnet11);
    let mut ctx = SpendContext::new();
    let (MultisigSingleton::Vault(medieval_vault), _state_scheduler_info) =
        sync_multisig_singleton::<StateSchedulerHintedState>(&client, &mut ctx, launcher_id, None)
            .await?
    else {
        return Err(CliError::Custom(
            "Multisig not in 'medieval vault' phase (not fully unrolled)".to_string(),
        ));
    };

    let alias_map = get_alias_map()?;
    let my_alias = if let Some(alias) = alias_map.get(&my_pubkey) {
        alias
    } else {
        &format!("0x{}", hex::encode(my_pubkey.to_bytes()))
    };

    println!("Current vault configuration:");
    print_medieval_vault_configuration(
        medieval_vault.info.m,
        &medieval_vault.info.public_key_list,
    )?;

    println!("\nYou'll update the CATalog state to:");
    println!("  Payment asset id: {}", new_payment_asset_id);

    println!("\nYou'll sign this CATALOG STATE UPDATE with the following pubkey:");
    println!("  {}", my_alias);

    yes_no_prompt("Continue?")?;

    let constants = get_constants(testnet11);
    let delegated_puzzle_ptr = MedievalVault::delegated_puzzle_for_rekey(
        &mut ctx,
        launcher_id,
        new_m,
        new_pubkeys,
        medieval_vault.coin.coin_id(),
        constants.genesis_challenge,
    )?;
    let delegated_puzzle_hash = ctx.tree_hash(delegated_puzzle_ptr);

    println!(
        "Delegated puzzle hash (secure - dependent on coin id & network):\n  {}",
        hex::encode(delegated_puzzle_hash.to_bytes())
    );
    let my_index = medieval_vault
        .info
        .public_key_list
        .iter()
        .position(|pk| pk == &my_pubkey)
        .unwrap();
    println!("Your index: {}", my_index);

    if debug {
        println!("DEBUG MODE");
        println!(
            "Full delegated puzzle: {}",
            hex::encode(node_to_bytes(&ctx.allocator, delegated_puzzle_ptr)?)
        );
        let sk_str = prompt_for_value("Paste your secret key:")?;
        let sk = hex_string_to_secret_key(&sk_str)?;
        if sk.public_key() != my_pubkey {
            return Err(CliError::Custom(
                "Public key does not match the provided secret key".to_string(),
            ));
        }

        let required_signature = RequiredBlsSignature::from_condition(
            &medieval_vault.coin,
            AggSig::new(
                AggSigKind::Unsafe,
                my_pubkey,
                Bytes::new(delegated_puzzle_hash.to_vec()),
            ),
            &AggSigConstants::new(constants.agg_sig_amount_additional_data),
        );

        let signature = sign(&sk, required_signature.message());
        println!(
            "\nYour signature: {}-{}",
            my_index,
            hex::encode(signature.to_bytes())
        );
    }

    Ok(())
}

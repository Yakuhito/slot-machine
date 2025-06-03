use chia::bls::sign;
use chia::bls::PublicKey;
use chia::protocol::Bytes;
use chia_wallet_sdk::coinset::CoinsetClient;
use chia_wallet_sdk::driver::SpendContext;
use chia_wallet_sdk::prelude::AggSig;
use chia_wallet_sdk::prelude::AggSigKind;
use chia_wallet_sdk::signer::AggSigConstants;
use chia_wallet_sdk::signer::RequiredBlsSignature;
use clvmr::NodePtr;

use crate::{
    get_alias_map, get_coinset_client, get_constants, hex_string_to_bytes32, hex_string_to_pubkey,
    hex_string_to_secret_key, print_medieval_vault_configuration, prompt_for_value,
    sync_multisig_singleton, yes_no_prompt, CliError, MedievalVault, MultisigSingleton,
    StateSchedulerHintedState,
};

pub async fn multisig_sign_thing_start(
    my_pubkey_str: String,
    launcher_id_str: String,
    testnet11: bool,
) -> Result<(PublicKey, SpendContext, CoinsetClient, MedievalVault), CliError> {
    let my_pubkey = hex_string_to_pubkey(&my_pubkey_str)?;
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
    println!("  Signing key: {}", my_alias);
    println!("  Coin id: {}", hex::encode(medieval_vault.coin.coin_id()));

    Ok((my_pubkey, ctx, client, medieval_vault))
}

pub async fn multisig_sign_thing_finish(
    ctx: &mut SpendContext,
    delegated_puzzle: NodePtr,
    medieval_vault: &MedievalVault,
    my_pubkey: PublicKey,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    println!(
        "Current medieval vault coin id: {}",
        hex::encode(medieval_vault.coin.coin_id())
    );
    yes_no_prompt("Continue?")?;

    let delegated_puzzle_hash = ctx.tree_hash(delegated_puzzle);

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
            &AggSigConstants::new(get_constants(testnet11).agg_sig_amount_additional_data),
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

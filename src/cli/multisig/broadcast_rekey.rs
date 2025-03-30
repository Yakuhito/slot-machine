use chia::{bls::Signature, protocol::SpendBundle};
use chia_wallet_sdk::{ChiaRpcClient, Offer, SpendContext, MAINNET_CONSTANTS, TESTNET11_CONSTANTS};
use sage_api::{Amount, Assets, MakeOffer};

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, hex_string_to_pubkey,
    hex_string_to_signature, new_sk, parse_amount, parse_one_sided_offer,
    print_medieval_vault_configuration, spend_security_coin, sync_multisig_singleton,
    wait_for_coin, yes_no_prompt, CliError, MedievalVault, MultisigSingleton, SageClient,
    StateSchedulerHintedState,
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

    let signature_strs = signatures_str.split(',').collect::<Vec<_>>();

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

    let mut signatures = Vec::with_capacity(signature_strs.len());
    let mut pubkeys = Vec::with_capacity(signature_strs.len());

    for signature_str in signature_strs.into_iter() {
        let parts = signature_str.split('-').collect::<Vec<_>>();
        let index = parts[0].parse::<usize>()?;
        let signature = hex_string_to_signature(parts[1])?;
        signatures.push(signature);
        pubkeys.push(medieval_vault.info.public_key_list[index]);
    }

    println!("Current vault configuration:");
    print_medieval_vault_configuration(
        medieval_vault.info.m,
        &medieval_vault.info.public_key_list,
    )?;

    if signatures.len() != medieval_vault.info.m {
        return Err(CliError::Custom(
            "Number of signatures does not match the required number of signatures".to_string(),
        ));
    }

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

    let sage = SageClient::new()?;
    let offer_resp = sage
        .make_offer(MakeOffer {
            requested_assets: Assets {
                xch: Amount::u64(0),
                cats: vec![],
                nfts: vec![],
            },
            offered_assets: Assets {
                xch: Amount::u64(1),
                cats: vec![],
                nfts: vec![],
            },
            fee: Amount::u64(fee),
            receive_address: None,
            expires_at_second: None,
            auto_import: false,
        })
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::decode(&offer_resp.offer).map_err(CliError::Offer)?;
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(&mut ctx, offer, security_coin_sk.public_key(), None, false)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let conditions =
        MedievalVault::rekey_create_coin_unsafe(&mut ctx, launcher_id, new_m, new_pubkeys)?;
    let medieval_vault_coin_id = medieval_vault.coin.coin_id();
    medieval_vault.spend(
        &mut ctx,
        &pubkeys,
        conditions,
        get_constants(testnet11).genesis_challenge,
    )?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer
            .security_base_conditions
            .assert_concurrent_spend(medieval_vault_coin_id),
        &security_coin_sk,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
    )?;

    let vault_agg_sig = signatures
        .iter()
        .fold(Signature::default(), |acc, sig| acc + sig);
    let sb = SpendBundle::new(
        ctx.take(),
        offer.aggregated_signature + &security_coin_sig + &vault_agg_sig,
    );

    println!("Submitting transaction...");
    let resp = client.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

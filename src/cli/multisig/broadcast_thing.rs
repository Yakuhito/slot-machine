use chia::{
    bls::{PublicKey, Signature},
    protocol::{Bytes32, SpendBundle},
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{
        create_security_coin, decode_offer, spend_security_coin, MedievalVault, Offer, SpendContext,
    },
    types::{Conditions, MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};

use crate::{
    assets_xch_only, get_coinset_client, hex_string_to_bytes32, hex_string_to_signature, no_assets,
    parse_amount, print_medieval_vault_configuration, sync_multisig_singleton, wait_for_coin,
    yes_no_prompt, CliError, MultisigSingleton, SageClient, StateSchedulerHintedState,
};

pub async fn multisig_broadcast_thing_start(
    signatures_str: String,
    launcher_id_str: String,
    testnet11: bool,
) -> Result<
    (
        Signature,
        Vec<PublicKey>,
        CoinsetClient,
        SpendContext,
        MedievalVault,
    ),
    CliError,
> {
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
    println!("  Coin id: {}", hex::encode(medieval_vault.coin.coin_id()));

    if signatures.len() != medieval_vault.info.m {
        return Err(CliError::Custom(
            "Number of signatures does not match the required number of signatures".to_string(),
        ));
    }

    let signature = signatures
        .iter()
        .fold(Signature::default(), |acc, sig| acc + sig);
    Ok((signature, pubkeys, client, ctx, medieval_vault))
}

pub async fn multisig_broadcast_thing_finish(
    client: CoinsetClient,
    ctx: &mut SpendContext,
    signature_from_signers: Signature,
    fee_str: String,
    testnet11: bool,
    medieval_vault_coin_id: Bytes32,
    additional_security_conditions: Option<Conditions>,
) -> Result<(), CliError> {
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
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(ctx, offer.offered_coins().xch[0])?;

    let mut conditions = Conditions::new().assert_concurrent_spend(medieval_vault_coin_id);
    if let Some(additional_security_conditions) = additional_security_conditions {
        conditions = conditions.extend(additional_security_conditions);
    }

    let security_coin_sig = spend_security_coin(
        ctx,
        security_coin,
        conditions,
        &security_coin_sk,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
    )?;

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &signature_from_signers,
    ));

    println!("Submitting transaction...");
    let resp = client.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&client, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

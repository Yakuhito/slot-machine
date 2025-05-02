use chia::{
    bls::{PublicKey, Signature},
    protocol::{Bytes32, SpendBundle},
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{Offer, SpendContext},
    types::{MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
};
use sage_api::{Amount, Assets, MakeOffer};

use crate::{
    get_coinset_client, hex_string_to_bytes32, hex_string_to_signature, new_sk, parse_amount,
    parse_one_sided_offer, print_medieval_vault_configuration, print_spend_bundle_to_file,
    spend_security_coin, sync_multisig_singleton, wait_for_coin, yes_no_prompt, CliError,
    MedievalVault, MultisigSingleton, SageClient, StateSchedulerHintedState,
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
    let offer = parse_one_sided_offer(ctx, offer, security_coin_sk.public_key(), None, false)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_sig = spend_security_coin(
        ctx,
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

    let sb = SpendBundle::new(
        ctx.take(),
        offer.aggregated_signature + &security_coin_sig + &signature_from_signers,
    );

    println!("Submitting transaction...");
    print_spend_bundle_to_file(
        sb.coin_spends.clone(),
        sb.aggregated_signature.clone(),
        "sb.debug",
    );
    let resp = client.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);
    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

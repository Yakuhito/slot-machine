use chia::bls::Signature;
use chia::clvm_utils::ToTreeHash;
use chia::protocol::{Bytes32, SpendBundle};
use chia::puzzles::singleton::SingletonSolution;
use chia_wallet_sdk::{
    ChiaRpcClient, Layer, Offer, Spend, SpendContext, MAINNET_CONSTANTS, TESTNET11_CONSTANTS,
};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, MakeOffer};

use crate::{
    get_coinset_client, get_constants, hex_string_to_bytes32, hex_string_to_signature, new_sk,
    parse_amount, parse_one_sided_offer, print_medieval_vault_configuration, quick_sync_catalog,
    spend_security_coin, sync_multisig_singleton, wait_for_coin, yes_no_prompt,
    CatalogRegistryConstants, CatalogRegistryState, CliError, DefaultCatMakerArgs, MedievalVault,
    MultisigSingleton, P2MOfNDelegateDirectArgs, P2MOfNDelegateDirectSolution, SageClient,
    StateSchedulerHintedState, StateSchedulerLayerSolution,
};

pub async fn multisig_broadcast_catalog_state_update(
    new_payment_asset_id_str: String,
    new_payment_asset_amount_str: String,
    launcher_id_str: String,
    signatures_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let signature_strs = signatures_str.split(',').collect::<Vec<_>>();
    let new_payment_asset_id = hex_string_to_bytes32(&new_payment_asset_id_str)?;
    let new_payment_asset_amount = parse_amount(&new_payment_asset_amount_str, true)?;

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

    println!("Current vault configuration:");
    print_medieval_vault_configuration(
        medieval_vault.info.m,
        &medieval_vault.info.public_key_list,
    )?;

    let mut signatures = Vec::with_capacity(signature_strs.len());
    let mut pubkeys = Vec::with_capacity(signature_strs.len());

    for signature_str in signature_strs.into_iter() {
        let parts = signature_str.split('-').collect::<Vec<_>>();
        let index = parts[0].parse::<usize>()?;
        let signature = hex_string_to_signature(parts[1])?;
        signatures.push(signature);
        pubkeys.push(medieval_vault.info.public_key_list[index]);
    }

    if signatures.len() != medieval_vault.info.m {
        return Err(CliError::Custom(
            "Number of signatures does not match the required number of signatures".to_string(),
        ));
    }

    println!("\nSyncing CATalog... ");
    let catalog_constants = CatalogRegistryConstants::get(testnet11);
    let catalog = quick_sync_catalog(&client, &mut ctx, catalog_constants).await?;
    println!("Done!");

    println!("Current CATalog state:");
    println!(
        "  CAT Maker: {}",
        hex::encode(catalog.info.state.cat_maker_puzzle_hash.to_bytes())
    );
    println!(
        "  Registration price (mojos): {}",
        catalog.info.state.registration_price
    );
    println!("You'll update the CATalog state to:");

    let new_cat_maker_puzzle_hash: Bytes32 =
        DefaultCatMakerArgs::curry_tree_hash(new_payment_asset_id.tree_hash().into()).into();
    let new_state = CatalogRegistryState {
        cat_maker_puzzle_hash: new_cat_maker_puzzle_hash,
        registration_price: new_payment_asset_amount,
    };
    println!(
        "  CAT Maker: {}",
        hex::encode(new_state.cat_maker_puzzle_hash.to_bytes())
    );
    println!(
        "  Registration price (mojos): {}",
        new_state.registration_price
    );
    println!(
        "  Payment asset id: {}",
        hex::encode(new_state.cat_maker_puzzle_hash.to_bytes())
    );

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

    let constants = get_constants(testnet11);
    let medieval_vault_coin_id = medieval_vault.coin.coin_id();

    let delegated_puzzle_ptr = MedievalVault::delegated_puzzle_for_catalog_state_update(
        &mut ctx,
        new_state.tree_hash().into(),
        catalog_constants.launcher_id,
        medieval_vault.coin,
        medieval_vault.info.launcher_id,
        constants.genesis_challenge,
    )?;

    let delegated_solution_ptr = ctx.alloc(&StateSchedulerLayerSolution {
        other_singleton_inner_puzzle_hash: catalog.info.inner_puzzle_hash().into(),
        inner_solution: NodePtr::NIL,
    })?;

    let medieval_vault_layers = medieval_vault.info.into_layers();
    let medieval_vault_puzzle = medieval_vault_layers.construct_puzzle(&mut ctx)?;
    let medieval_vault_solution = medieval_vault_layers.construct_solution(
        &mut ctx,
        SingletonSolution {
            lineage_proof: medieval_vault.proof,
            amount: medieval_vault.coin.amount,
            inner_solution: P2MOfNDelegateDirectSolution {
                selectors: P2MOfNDelegateDirectArgs::selectors_for_used_pubkeys(
                    &medieval_vault.info.public_key_list,
                    &pubkeys,
                ),
                delegated_puzzle: delegated_puzzle_ptr,
                delegated_solution: delegated_solution_ptr,
            },
        },
    )?;

    ctx.spend(
        medieval_vault.coin,
        Spend::new(medieval_vault_puzzle, medieval_vault_solution),
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

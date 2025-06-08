use chia::protocol::SpendBundle;
use chia_puzzle_types::{standard::StandardArgs, LineageProof};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{HashedPtr, Layer, Nft, Offer, Puzzle, SingletonLayer, SpendContext},
    types::Conditions,
    utils::Address,
};
use clvmr::NodePtr;
use sage_api::{Amount, Assets, GetDerivations, MakeOffer};

use crate::{
    get_coinset_client, get_constants, get_last_onchain_timestamp, get_prefix,
    hex_string_to_bytes32, hex_string_to_pubkey, new_sk, parse_amount, parse_one_sided_offer,
    spend_security_coin, sync_distributor, wait_for_coin, yes_no_prompt, CliError, Db,
    IntermediaryCoinProof, NftLauncherProof, RewardDistributorStakeAction,
    RewardDistributorSyncAction, SageClient,
};

pub async fn reward_distributor_stake(
    launcher_id_str: String,
    nft_id_str: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let nft_launcher_id = Address::decode(&nft_id_str)?.puzzle_hash;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();
    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let latest_timestamp = get_last_onchain_timestamp(&client).await?;
    if latest_timestamp > distributor.info.state.round_time_info.epoch_end {
        return Err(CliError::Custom(
            "The current epoch has already ended - start a new epoch first".to_string(),
        ));
    }

    let also_sync = distributor.info.state.round_time_info.last_update + 180 < latest_timestamp;
    if also_sync {
        println!(
            "Will also sync the distributor to timestamp {}",
            latest_timestamp
        );
    }

    let sage = SageClient::new()?;
    let custody_info = sage
        .get_derivations(GetDerivations {
            hardened: false,
            offset: 0,
            limit: 1,
        })
        .await?
        .derivations[0]
        .clone();
    let custody_puzzle_hash = Address::decode(&custody_info.address)?.puzzle_hash;
    if StandardArgs::curry_tree_hash(hex_string_to_pubkey(&custody_info.public_key)?)
        != custody_puzzle_hash.into()
    {
        return Err(CliError::Custom(
            "Custody puzzle hash does not match the retrieved public key".to_string(),
        ));
    }

    print!("Generating NFT launcher proof...");
    let mut intemrediary_coins = Vec::new();
    let did_proof;
    let mut latest_coin_id = nft_launcher_id;
    loop {
        let coin_record = client
            .get_coin_record_by_name(latest_coin_id)
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(latest_coin_id))?;

        // speed up: only odd coins can be the launcher DID
        if coin_record.coin.amount % 2 == 1 {
            let coin_spend = client
                .get_puzzle_and_solution(latest_coin_id, Some(coin_record.spent_block_index))
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(latest_coin_id))?;
            let puzzle = ctx.alloc(&coin_spend.puzzle_reveal)?;
            let puzzle = Puzzle::parse(&ctx, puzzle);

            if let Ok(Some(layer)) = SingletonLayer::<HashedPtr>::parse_puzzle(&ctx, puzzle) {
                did_proof = LineageProof {
                    parent_parent_coin_info: coin_record.coin.parent_coin_info,
                    parent_inner_puzzle_hash: layer.inner_puzzle.tree_hash().into(),
                    parent_amount: coin_record.coin.amount,
                };
                if layer.launcher_id
                    != distributor
                        .info
                        .constants
                        .manager_or_collection_did_launcher_id
                {
                    println!("FAILED");
                    return Err(CliError::Custom(
                        "The DID launcher ID does not match the reward distributor's configuration - does the NFT belong to the right collection?"
                            .to_string(),
                    ));
                }
                break;
            }
        }

        latest_coin_id = coin_record.coin.parent_coin_info;
        intemrediary_coins.push(IntermediaryCoinProof {
            full_puzzle_hash: coin_record.coin.puzzle_hash,
            amount: coin_record.coin.amount,
        });
    }

    let nft_launcher_proof = NftLauncherProof {
        did_proof,
        intermediary_coin_proofs: intemrediary_coins.into_iter().rev().collect(),
    };
    println!(
        "done ({} intermediary coins).",
        nft_launcher_proof.intermediary_coin_proofs.len() - 1
    );

    println!(
        "Using the following address as custody: {}",
        Address::new(custody_puzzle_hash, get_prefix(testnet11)).encode()?
    );
    println!("Custody puzzle hash: {}", hex::encode(custody_puzzle_hash));

    println!("A one-sided offer will be created. It will contain:");
    println!("  the NFT to be deposited");
    println!("  1 mojo");
    println!("  {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

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
                nfts: vec![nft_id_str],
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

    let sec_conds = if also_sync {
        distributor
            .new_action::<RewardDistributorSyncAction>()
            .spend(&mut ctx, &mut distributor, latest_timestamp)?
    } else {
        Conditions::new()
    };

    // find NFT
    let mut current_nft = None;
    for coin_spend in offer.clone().parse(&mut ctx)?.coin_spends {
        let puzzle_ptr = ctx.alloc(&coin_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(&ctx, puzzle_ptr);
        if let Ok(Some(layer)) = SingletonLayer::<NodePtr>::parse_puzzle(&ctx, puzzle) {
            if layer.launcher_id != nft_launcher_id {
                continue;
            }

            let parent_record = client
                .get_coin_record_by_name(coin_spend.coin.parent_coin_info)
                .await?
                .coin_record
                .ok_or(CliError::CoinNotFound(coin_spend.coin.parent_coin_info))?;
            let parent_coin_spend = client
                .get_puzzle_and_solution(
                    coin_spend.coin.parent_coin_info,
                    Some(parent_record.spent_block_index),
                )
                .await?
                .coin_solution
                .ok_or(CliError::CoinNotSpent(coin_spend.coin.parent_coin_info))?;

            let parent_puzzle = ctx.alloc(&parent_coin_spend.puzzle_reveal)?;
            let parent_puzzle = Puzzle::parse(&ctx, parent_puzzle);
            let parent_solution = ctx.alloc(&parent_coin_spend.solution)?;
            current_nft = Nft::parse_child(
                &mut ctx,
                parent_coin_spend.coin,
                parent_puzzle,
                parent_solution,
            )?;
            break;
        }
    }

    // accept offer
    let (conds, notarized_payment, _slot, _locked_nft) = distributor
        .new_action::<RewardDistributorStakeAction>()
        .spend(
            &mut ctx,
            &mut distributor,
            current_nft.unwrap(),
            nft_launcher_proof,
            custody_puzzle_hash,
        )?;
    let offer = parse_one_sided_offer(
        &mut ctx,
        offer,
        security_coin_sk.public_key(),
        None,
        Some(notarized_payment),
    )?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let sec_conds = sec_conds.extend(conds).reserve_fee(1);

    let _new_distributor = distributor.finish_spend(&mut ctx, vec![])?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        offer.security_coin,
        offer.security_base_conditions.extend(sec_conds),
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let spend_bundle =
        SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig);

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, offer.security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

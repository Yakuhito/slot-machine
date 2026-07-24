use chia_protocol::{Bytes32, SpendBundle};
use chia_puzzle_types::{cat::CatArgs, singleton::SingletonStruct, LineageProof};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, Cat, CatInfo, CatLayer, CatSpend,
        Layer, Offer, Puzzle, RewardDistributor, RewardDistributorCommitIncentivesAction,
        RewardDistributorType, Spend, SpendContext,
    },
    types::{
        puzzles::{
            NonceWrapperArgs, P2NextRewardDistributorEpochArgs,
            P2NextRewardDistributorEpochSolution,
        },
        Conditions, Mod,
    },
    utils::Address,
};
use clvm_utils::{ToTreeHash, TreeHash};

use crate::{
    assets_xch_only, confirm_pushed_transaction, find_reward_slot, get_coinset_client,
    get_constants, hex_string_to_bytes32, no_assets, parse_amount, sync_distributor, yes_no_prompt,
    CliError, Db, SageClient,
};

pub async fn reward_distributor_commit_available_rewards(
    launcher_id_str: String,
    clawback_address: Option<String>,
    max_coins: usize,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    if max_coins == 0 || max_coins > 32 {
        return Err(CliError::Custom(
            "max-coins must be between 1 and 32".to_string(),
        ));
    }

    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let clawback_inner_puzzle_hash = clawback_address
        .as_deref()
        .map(Address::decode)
        .transpose()?
        .map_or_else(Bytes32::default, |address| address.puzzle_hash);
    let fee = parse_amount(&fee_str, false)?;

    let client = get_coinset_client(testnet11);
    let mut ctx = SpendContext::new();
    let launcher_record = client
        .get_coin_record_by_name(launcher_id)
        .await?
        .coin_record
        .ok_or(CliError::CoinNotFound(launcher_id))?;
    let launcher_spend = client
        .get_puzzle_and_solution(launcher_id, Some(launcher_record.spent_block_index))
        .await?
        .coin_solution
        .ok_or(CliError::CoinNotSpent(launcher_id))?;
    let launcher_solution = ctx.alloc(&launcher_spend.solution)?;
    let Some((constants, initial_state, _eve_coin)) = RewardDistributor::from_launcher_solution(
        &mut ctx,
        launcher_spend.coin,
        launcher_solution,
    )?
    else {
        return Err(CliError::Custom(
            "Could not parse reward distributor launcher spend".to_string(),
        ));
    };
    let first_epoch_start = initial_state.round_time_info.epoch_end;
    let reward_asset_id = match constants.reward_distributor_type {
        RewardDistributorType::Cat { asset_id, .. } => asset_id,
        _ => constants.reserve_asset_id,
    };

    let p2_args = P2NextRewardDistributorEpochArgs::new(
        clawback_inner_puzzle_hash,
        SingletonStruct::new(launcher_id).tree_hash(),
        first_epoch_start,
        constants.epoch_seconds,
    );
    let p2_inner_puzzle_hash: Bytes32 = p2_args.curry_tree_hash().into();
    let full_puzzle_hash: Bytes32 =
        CatArgs::curry_tree_hash(reward_asset_id, p2_inner_puzzle_hash.into()).into();

    println!(
        "Reward CAT deposit address: {}",
        Address::new(p2_inner_puzzle_hash, crate::get_prefix(testnet11)).encode()?
    );
    println!("Syncing reward distributor...");
    let db = Db::new(false).await?;
    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    println!(
        "Scanning full Reward CAT puzzle hash {}...",
        hex::encode(full_puzzle_hash)
    );
    let mut records = client
        .get_coin_records_by_puzzle_hash(full_puzzle_hash, None, None, Some(false), None)
        .await?
        .coin_records
        .unwrap_or_default();
    records.retain(|record| !record.spent && record.coin.puzzle_hash == full_puzzle_hash);
    records.sort_by_key(|record| record.coin.coin_id());
    records.truncate(max_coins);

    if records.is_empty() {
        return Err(CliError::Custom(
            "No unspent Reward CAT coins are available to commit".to_string(),
        ));
    }

    let next_epoch_start = distributor.info.state.round_time_info.epoch_end;
    let reward_slot = find_reward_slot(&mut ctx, &client, constants, next_epoch_start).await?;
    let total_rewards = records.iter().try_fold(0_u64, |total, record| {
        total
            .checked_add(record.coin.amount)
            .ok_or_else(|| CliError::Custom("Available Reward CAT total exceeds u64".to_string()))
    })?;

    println!(
        "Found {} coin(s), totaling {} CAT mojos, for epoch {}.",
        records.len(),
        total_rewards,
        next_epoch_start
    );
    println!(
        "Commitments will use clawback inner puzzle hash {}.",
        hex::encode(clawback_inner_puzzle_hash)
    );
    println!("{} XCH ({} mojos) will be reserved as fees.", fee_str, fee);
    yes_no_prompt("Proceed?")?;

    let sage = SageClient::new()?;
    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let mut p2_cat_spends = Vec::with_capacity(records.len());
    for record in records {
        let parent_spend = client
            .get_puzzle_and_solution(
                record.coin.parent_coin_info,
                Some(record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(record.coin.parent_coin_info))?;
        let parent_puzzle_ptr = ctx.alloc(&parent_spend.puzzle_reveal)?;
        let parent_puzzle = Puzzle::parse(&ctx, parent_puzzle_ptr);
        let Some(parent_cat) = CatLayer::<clvmr::NodePtr>::parse_puzzle(&ctx, parent_puzzle)?
        else {
            return Err(CliError::Custom(format!(
                "Parent of coin {} is not a CAT",
                hex::encode(record.coin.coin_id())
            )));
        };
        if parent_cat.asset_id != reward_asset_id {
            return Err(CliError::Custom(format!(
                "Coin {} is not the distributor Reward CAT",
                hex::encode(record.coin.coin_id())
            )));
        }

        let cat = Cat::new(
            record.coin,
            Some(LineageProof {
                parent_parent_coin_info: parent_spend.coin.parent_coin_info,
                parent_inner_puzzle_hash: ctx.tree_hash(parent_cat.inner_puzzle).into(),
                parent_amount: parent_spend.coin.amount,
            }),
            CatInfo::new(reward_asset_id, None, p2_inner_puzzle_hash),
        );
        let clawback_ph = NonceWrapperArgs::<Bytes32, TreeHash> {
            nonce: cat.coin.coin_id(),
            inner_puzzle: clawback_inner_puzzle_hash.into(),
        }
        .curry_tree_hash();

        let _security_conditions = distributor
            .new_action::<RewardDistributorCommitIncentivesAction>()
            .spend(
                &mut ctx,
                &mut distributor,
                reward_slot.clone(),
                next_epoch_start,
                clawback_ph.into(),
                cat.coin.amount,
            )?;

        let p2_inner_puzzle = ctx.curry(p2_args)?;
        let p2_inner_solution = ctx.alloc(&P2NextRewardDistributorEpochSolution {
            next_epoch_start,
            my_id: cat.coin.coin_id(),
            my_amount: cat.coin.amount,
            reward_distributor_inner_puzzle_hash: distributor.info.inner_puzzle_hash().into(),
        })?;
        p2_cat_spends.push(CatSpend::new(
            cat,
            Spend::new(p2_inner_puzzle, p2_inner_solution),
        ));
    }

    let (_new_distributor, distributor_signature) =
        distributor.finish_spend(&mut ctx, p2_cat_spends)?;
    let security_signature = spend_security_coin(
        &mut ctx,
        security_coin,
        Conditions::new(),
        &security_coin_sk,
        get_constants(testnet11),
    )?;
    let spend_bundle = offer.take(SpendBundle::new(
        ctx.take(),
        security_signature + &distributor_signature,
    ));

    println!("Submitting transaction...");
    let response = client.push_tx(spend_bundle).await?;
    if confirm_pushed_transaction(&client, &response, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

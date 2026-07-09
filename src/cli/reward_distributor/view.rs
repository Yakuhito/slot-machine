use crate::{
    format_cat_mojos, format_precision_amount, get_coinset_client, get_prefix,
    hex_string_to_bytes32, sync_distributor, CliError, Db,
};
use chia_wallet_sdk::{
    driver::{RewardDistributorType, SpendContext},
    utils::Address,
};

pub async fn reward_distributor_view(
    launcher_id_str: String,
    testnet11: bool,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();
    let distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    let precision = distributor.info.constants.precision;
    let cumulative_payout = distributor.info.state.round_reward_info.cumulative_payout;
    let remaining_rewards = distributor.info.state.round_reward_info.remaining_rewards;

    println!(
        "Latest coin id: {}",
        hex::encode(distributor.coin.coin_id())
    );
    println!("State:");
    println!("  Active shares: {}", distributor.info.state.active_shares);
    println!(
        "  Cumulative payout: {}",
        format_precision_amount(cumulative_payout, precision)
    );
    println!(
        "  Remaining rewards: {}",
        format_precision_amount(remaining_rewards, precision)
    );
    println!(
        "  Epoch end: {}",
        distributor.info.state.round_time_info.epoch_end
    );
    println!(
        "  Last update: {}",
        distributor.info.state.round_time_info.last_update
    );
    println!(
        "  Total reserves: {}",
        format_cat_mojos(distributor.info.state.total_reserves)
    );

    println!("Constants:");
    println!(
        "  Launcher ID: {}",
        hex::encode(distributor.info.constants.launcher_id)
    );
    match distributor.info.constants.reward_distributor_type {
        RewardDistributorType::Managed {
            manager_singleton_launcher_id,
        } => println!(
            "  Manager launcher ID: {}",
            hex::encode(manager_singleton_launcher_id)
        ),
        RewardDistributorType::NftCollection {
            collection_did_launcher_id,
        } => println!(
            "  Collection DID launcher ID: {}",
            hex::encode(collection_did_launcher_id)
        ),
        RewardDistributorType::CuratedNft {
            store_launcher_id,
            refreshable,
        } => {
            println!(
                "  DataStore launcher ID: {}",
                hex::encode(store_launcher_id)
            );
            println!("  Refreshable: {refreshable}");
        }
        RewardDistributorType::Cat {
            asset_id,
            hidden_puzzle_hash,
        } => {
            println!("  Stake asset ID: {}", hex::encode(asset_id));
            if let Some(hidden_puzzle_hash) = hidden_puzzle_hash {
                println!("  Hidden puzzle hash: {}", hex::encode(hidden_puzzle_hash));
            }
        }
    };
    println!(
        "  Fee payout address: {}",
        Address::new(
            distributor.info.constants.fee_payout_puzzle_hash,
            get_prefix(testnet11)
        )
        .encode()?
    );
    println!(
        "  Seconds per epoch: {}",
        distributor.info.constants.epoch_seconds
    );
    println!("  Precision: {}", precision);
    println!(
        "  Max seconds offset: {}",
        distributor.info.constants.max_seconds_offset
    );
    let threshold = distributor.info.constants.payout_threshold;
    println!("  Payout threshold: {}", format_cat_mojos(threshold));
    println!(
        "  Require payout approval: {}",
        distributor.info.constants.require_payout_approval
    );
    println!("  Fee bps: {}", distributor.info.constants.fee_bps);
    println!(
        "  Withdrawal share bps: {}",
        distributor.info.constants.withdrawal_share_bps
    );

    Ok(())
}

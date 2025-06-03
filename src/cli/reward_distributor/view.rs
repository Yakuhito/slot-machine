use crate::{
    get_coinset_client, get_prefix, hex_string_to_bytes32, sync_distributor, CliError, Db,
};
use chia_wallet_sdk::{driver::SpendContext, utils::Address};

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

    println!(
        "Latest coin id: {}",
        hex::encode(distributor.coin.coin_id())
    );
    println!("State:");
    println!("  Active shares: {}", distributor.info.state.active_shares);
    println!(
        "  Cumulative payout: {} mojos per share",
        distributor.info.state.round_reward_info.cumulative_payout
    );
    println!(
        "  Remaining rewards: {} mojos",
        distributor.info.state.round_reward_info.remaining_rewards
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
        "  Total reserves: {} CATs",
        distributor.info.state.total_reserves as f64 / 1000.0
    );

    println!("Constants:");
    println!(
        "  Launcher ID: {}",
        hex::encode(distributor.info.constants.launcher_id)
    );
    println!(
        "  Manager launcher ID: {}",
        hex::encode(distributor.info.constants.manager_launcher_id)
    );
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
    println!(
        "  Max seconds offset: {}",
        distributor.info.constants.max_seconds_offset
    );
    println!(
        "  Payout threshold: {} mojos",
        distributor.info.constants.payout_threshold
    );
    println!("  Fee bps: {}", distributor.info.constants.fee_bps);
    println!(
        "  Withdrawal share bps: {}",
        distributor.info.constants.withdrawal_share_bps
    );

    Ok(())
}

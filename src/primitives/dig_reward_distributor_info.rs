use chia::protocol::Bytes32;
use chia_wallet_sdk::SingletonLayer;
use clvm_traits::{FromClvm, ToClvm};

use crate::ActionLayer;

pub type DigRewardDistributorLayers = SingletonLayer<ActionLayer<DigRewardDistributorState>>;

#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm, Copy)]
#[clvm(list)]
pub struct RoundRewardInfo {
    pub cumulative_payout: u64,
    #[clvm(rest)]
    pub remaining_rewards: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm, Copy)]
#[clvm(list)]
pub struct RoundTimeInfo {
    pub last_update: u64,
    #[clvm(rest)]
    pub epoch_end: u64,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm, Copy)]
#[clvm(list)]
pub struct DigRewardDistributorState {
    pub total_reserves: u64,
    pub active_shares: u64,
    pub round_reward_info: RoundRewardInfo,
    pub round_time_info: RoundTimeInfo,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct DigRewardDistributorConstants {
    pub validator_launcher_id: Bytes32,
    pub validator_payout_puzzle_hash: Bytes32,
    pub epoch_seconds: u64,
    pub removal_max_seconds_offset: u64,
    pub payout_threshold: u64,
    pub validator_fee_bps: u64,
    pub withdrawal_share_bps: u64,
    pub reserve_inner_puzzle_hash: Bytes32,
    pub reserve_full_puzzle_hash: Bytes32,
}

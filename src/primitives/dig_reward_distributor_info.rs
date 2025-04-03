use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::{cat::CatArgs, singleton::SingletonArgs},
};
use chia_wallet_sdk::{
    driver::{DriverError, Layer, Puzzle, SingletonLayer},
    types::MerkleTree,
};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::Allocator;

use crate::{
    Action, ActionLayer, ActionLayerArgs, DigAddIncentivesAction, DigAddMirrorAction,
    DigCommitIncentivesAction, DigInitiatePayoutAction, DigNewEpochAction, DigRemoveMirrorAction,
    DigSyncAction, DigWithdrawIncentivesAction, Finalizer, P2DelegatedBySingletonLayerArgs,
    ReserveFinalizer2ndCurryArgs,
};

use super::Reserveful;

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

impl DigRewardDistributorState {
    pub fn initial(first_epoch_start: u64) -> Self {
        Self {
            total_reserves: 0,
            active_shares: 0,
            round_reward_info: RoundRewardInfo {
                cumulative_payout: 0,
                remaining_rewards: 0,
            },
            round_time_info: RoundTimeInfo {
                last_update: first_epoch_start,
                epoch_end: first_epoch_start,
            },
        }
    }
}

impl Reserveful for DigRewardDistributorState {
    fn reserve_amount(&self, index: u64) -> u64 {
        if index == 0 {
            self.total_reserves
        } else {
            0
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy, ToClvm, FromClvm)]
#[clvm(list)]
pub struct DigRewardDistributorConstants {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
    pub validator_payout_puzzle_hash: Bytes32,
    pub epoch_seconds: u64,
    pub max_seconds_offset: u64,
    pub payout_threshold: u64,
    pub validator_fee_bps: u64,
    pub withdrawal_share_bps: u64,
    pub reserve_asset_id: Bytes32,
    pub reserve_inner_puzzle_hash: Bytes32,
    pub reserve_full_puzzle_hash: Bytes32,
}

impl DigRewardDistributorConstants {
    #[allow(clippy::too_many_arguments)]
    pub fn without_launcher_id(
        validator_launcher_id: Bytes32,
        validator_payout_puzzle_hash: Bytes32,
        epoch_seconds: u64,
        max_seconds_offset: u64,
        payout_threshold: u64,
        validator_fee_bps: u64,
        withdrawal_share_bps: u64,
        reserve_asset_id: Bytes32,
    ) -> Self {
        Self {
            launcher_id: Bytes32::default(),
            validator_launcher_id,
            validator_payout_puzzle_hash,
            epoch_seconds,
            max_seconds_offset,
            payout_threshold,
            validator_fee_bps,
            withdrawal_share_bps,
            reserve_asset_id,
            reserve_inner_puzzle_hash: Bytes32::default(),
            reserve_full_puzzle_hash: Bytes32::default(),
        }
    }

    pub fn with_launcher_id(mut self, launcher_id: Bytes32) -> Self {
        self.launcher_id = launcher_id;
        self.reserve_inner_puzzle_hash =
            P2DelegatedBySingletonLayerArgs::curry_tree_hash_with_launcher_id(launcher_id, 0)
                .into();
        self.reserve_full_puzzle_hash =
            CatArgs::curry_tree_hash(self.reserve_asset_id, self.reserve_inner_puzzle_hash.into())
                .into();
        self
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct DigRewardDistributorInfo {
    pub state: DigRewardDistributorState,

    pub constants: DigRewardDistributorConstants,
}

impl DigRewardDistributorInfo {
    pub fn new(state: DigRewardDistributorState, constants: DigRewardDistributorConstants) -> Self {
        Self { state, constants }
    }

    pub fn with_state(mut self, state: DigRewardDistributorState) -> Self {
        self.state = state;
        self
    }

    pub fn action_puzzle_hashes(constants: &DigRewardDistributorConstants) -> [Bytes32; 8] {
        [
            DigAddIncentivesAction::from_constants(constants)
                .tree_hash()
                .into(),
            DigAddMirrorAction::from_constants(constants)
                .tree_hash()
                .into(),
            DigCommitIncentivesAction::from_constants(constants)
                .tree_hash()
                .into(),
            DigInitiatePayoutAction::from_constants(constants)
                .tree_hash()
                .into(),
            DigNewEpochAction::from_constants(constants)
                .tree_hash()
                .into(),
            DigRemoveMirrorAction::from_constants(constants)
                .tree_hash()
                .into(),
            DigSyncAction::from_constants(constants).tree_hash().into(),
            DigWithdrawIncentivesAction::from_constants(constants)
                .tree_hash()
                .into(),
        ]
    }

    #[must_use]
    pub fn into_layers(self) -> DigRewardDistributorLayers {
        SingletonLayer::new(
            self.constants.launcher_id,
            ActionLayer::from_action_puzzle_hashes(
                &Self::action_puzzle_hashes(&self.constants),
                self.state,
                Finalizer::Reserve {
                    hint: self.constants.launcher_id,
                    reserve_full_puzzle_hash: self.constants.reserve_full_puzzle_hash,
                    reserve_inner_puzzle_hash: self.constants.reserve_inner_puzzle_hash,
                },
            ),
        )
    }

    pub fn parse(
        allocator: &mut Allocator,
        puzzle: Puzzle,
        constants: DigRewardDistributorConstants,
    ) -> Result<Option<Self>, DriverError> {
        let Some(layers) = DigRewardDistributorLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        let action_puzzle_hashes = Self::action_puzzle_hashes(&constants);
        let merkle_root = MerkleTree::new(&action_puzzle_hashes).root();
        if layers.inner_puzzle.merkle_root != merkle_root {
            return Ok(None);
        }

        Ok(Some(Self::from_layers(layers, constants)))
    }

    pub fn from_layers(
        layers: DigRewardDistributorLayers,
        constants: DigRewardDistributorConstants,
    ) -> Self {
        Self {
            state: layers.inner_puzzle.state,
            constants,
        }
    }

    pub fn puzzle_hash(&self) -> TreeHash {
        SingletonArgs::curry_tree_hash(self.constants.launcher_id, self.inner_puzzle_hash())
    }

    pub fn inner_puzzle_hash(&self) -> TreeHash {
        ActionLayerArgs::curry_tree_hash(
            ReserveFinalizer2ndCurryArgs::curry_tree_hash(
                self.constants.reserve_full_puzzle_hash,
                self.constants.reserve_inner_puzzle_hash,
                self.constants.launcher_id,
            ),
            MerkleTree::new(&Self::action_puzzle_hashes(&self.constants)).root(),
            self.state.tree_hash(),
        )
    }
}

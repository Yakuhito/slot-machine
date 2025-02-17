use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::{cat::CatArgs, singleton::SingletonArgs},
};
use chia_wallet_sdk::{DriverError, Layer, MerkleTree, Puzzle, SingletonLayer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::Allocator;

use crate::{
    ActionLayer, ActionLayerArgs, DigAddIncentivesAction, DigAddMirrorAction,
    DigCommitIncentivesAction, DigInitiatePayoutAction, DigNewEpochAction, DigRemoveMirrorAction,
    DigSyncAction, DigWithdrawIncentivesAction, Finalizer, P2DelegatedBySingletonLayerArgs,
    ReserveFinalizer2ndCurryArgs,
};

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
    pub max_seconds_offset: u64,
    pub payout_threshold: u64,
    pub validator_fee_bps: u64,
    pub withdrawal_share_bps: u64,
    pub reserve_asset_id: Bytes32,
    pub reserve_inner_puzzle_hash: Bytes32,
    pub reserve_full_puzzle_hash: Bytes32,
}

impl DigRewardDistributorConstants {
    pub fn with_launcher_id(mut self, launcher_id: Bytes32) -> Self {
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
    pub launcher_id: Bytes32,
    pub state: DigRewardDistributorState,

    pub constants: DigRewardDistributorConstants,
}

impl DigRewardDistributorInfo {
    pub fn new(
        launcher_id: Bytes32,
        state: DigRewardDistributorState,
        constants: DigRewardDistributorConstants,
    ) -> Self {
        Self {
            launcher_id,
            state,
            constants,
        }
    }

    pub fn with_state(mut self, state: DigRewardDistributorState) -> Self {
        self.state = state;
        self
    }

    pub fn action_puzzle_hashes(
        launcher_id: Bytes32,
        constants: &DigRewardDistributorConstants,
    ) -> [Bytes32; 8] {
        [
            DigAddIncentivesAction::curry_tree_hash(
                constants.validator_payout_puzzle_hash,
                constants.validator_fee_bps,
            )
            .into(),
            DigAddMirrorAction::curry_tree_hash(
                launcher_id,
                constants.validator_launcher_id,
                constants.max_seconds_offset,
            )
            .into(),
            DigCommitIncentivesAction::curry_tree_hash(launcher_id, constants.epoch_seconds).into(),
            DigInitiatePayoutAction::curry_tree_hash(
                launcher_id,
                constants.validator_launcher_id,
                constants.payout_threshold,
            )
            .into(),
            DigNewEpochAction::curry_tree_hash(
                launcher_id,
                constants.validator_payout_puzzle_hash,
                constants.validator_fee_bps,
                constants.epoch_seconds,
            )
            .into(),
            DigRemoveMirrorAction::curry_tree_hash(
                launcher_id,
                constants.validator_launcher_id,
                constants.max_seconds_offset,
            )
            .into(),
            DigSyncAction::curry_tree_hash().into(),
            DigWithdrawIncentivesAction::curry_tree_hash(
                launcher_id,
                constants.withdrawal_share_bps,
            )
            .into(),
        ]
    }

    #[must_use]
    pub fn into_layers(self) -> DigRewardDistributorLayers {
        SingletonLayer::new(
            self.launcher_id,
            ActionLayer::from_action_puzzle_hashes(
                &Self::action_puzzle_hashes(self.launcher_id, &self.constants),
                self.state,
                Finalizer::Reserve {
                    hint: self.launcher_id,
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

        let action_puzzle_hashes = Self::action_puzzle_hashes(layers.launcher_id, &constants);
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
            launcher_id: layers.launcher_id,
            state: layers.inner_puzzle.state,
            constants,
        }
    }

    pub fn puzzle_hash(&self) -> TreeHash {
        SingletonArgs::curry_tree_hash(self.launcher_id, self.inner_puzzle_hash())
    }

    pub fn inner_puzzle_hash(&self) -> TreeHash {
        ActionLayerArgs::curry_tree_hash(
            ReserveFinalizer2ndCurryArgs::curry_tree_hash(
                self.constants.reserve_full_puzzle_hash,
                self.constants.reserve_inner_puzzle_hash,
                self.launcher_id,
            ),
            MerkleTree::new(&Self::action_puzzle_hashes(
                self.launcher_id,
                &self.constants,
            ))
            .root(),
            self.state.tree_hash(),
        )
    }
}

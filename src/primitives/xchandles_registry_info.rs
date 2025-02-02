use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonArgs,
};
use chia_wallet_sdk::{DriverError, Layer, MerkleTree, Puzzle, SingletonLayer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::Allocator;

use crate::{
    ActionLayer, ActionLayerArgs, DefaultFinalizer2ndCurryArgs, DelegatedStateActionArgs,
    XchandlesExpireAction, XchandlesExponentialPremiumRenewPuzzleArgs, XchandlesExtendAction,
    XchandlesFactorPricingPuzzleArgs, XchandlesOracleAction, XchandlesRefundAction,
    XchandlesRegisterAction, XchandlesUpdateAction,
};

use super::DefaultCatMakerArgs;

pub type XchandlesRegistryLayers = SingletonLayer<ActionLayer<XchandlesRegistryState>>;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm, Copy)]
#[clvm(list)]
pub struct XchandlesRegistryState {
    pub cat_maker_puzzle_hash: Bytes32,
    pub pricing_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub expired_handle_pricing_puzzle_hash: Bytes32,
}

impl XchandlesRegistryState {
    pub fn from(payment_cat_tail_hash_hash: Bytes32, base_price: u64) -> Self {
        Self {
            cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(payment_cat_tail_hash_hash)
                .into(),
            pricing_puzzle_hash: XchandlesFactorPricingPuzzleArgs::curry_tree_hash(base_price)
                .into(),
            expired_handle_pricing_puzzle_hash:
                XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(base_price, 1000).into(),
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct XchandlesConstants {
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
    pub price_singleton_launcher_id: Bytes32,
}

impl XchandlesConstants {
    pub fn new(
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
        price_singleton_launcher_id: Bytes32,
    ) -> Self {
        Self {
            precommit_payout_puzzle_hash,
            relative_block_height,
            price_singleton_launcher_id,
        }
    }

    pub fn with_price_singleton(mut self, price_singleton_launcher_id: Bytes32) -> Self {
        self.price_singleton_launcher_id = price_singleton_launcher_id;
        self
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct XchandlesRegistryInfo {
    pub launcher_id: Bytes32,
    pub state: XchandlesRegistryState,

    pub constants: XchandlesConstants,
}

impl XchandlesRegistryInfo {
    pub fn new(
        launcher_id: Bytes32,
        state: XchandlesRegistryState,
        constants: XchandlesConstants,
    ) -> Self {
        Self {
            launcher_id,
            state,
            constants,
        }
    }

    pub fn with_state(mut self, state: XchandlesRegistryState) -> Self {
        self.state = state;
        self
    }

    pub fn action_puzzle_hashes(
        launcher_id: Bytes32,
        constants: &XchandlesConstants,
    ) -> [Bytes32; 7] {
        [
            XchandlesExpireAction::new(
                launcher_id,
                constants.relative_block_height,
                constants.precommit_payout_puzzle_hash,
            )
            .tree_hash()
            .into(),
            XchandlesExtendAction::new(launcher_id, constants.precommit_payout_puzzle_hash)
                .tree_hash()
                .into(),
            XchandlesOracleAction::new(launcher_id).tree_hash().into(),
            XchandlesRegisterAction::new(
                launcher_id,
                constants.relative_block_height,
                constants.precommit_payout_puzzle_hash,
            )
            .tree_hash()
            .into(),
            XchandlesUpdateAction::new(launcher_id).tree_hash().into(),
            XchandlesRefundAction::new(
                launcher_id,
                constants.relative_block_height,
                constants.precommit_payout_puzzle_hash,
            )
            .tree_hash()
            .into(),
            DelegatedStateActionArgs::curry_tree_hash(constants.price_singleton_launcher_id).into(),
        ]
    }

    #[must_use]
    pub fn into_layers(self) -> XchandlesRegistryLayers {
        SingletonLayer::new(
            self.launcher_id,
            ActionLayer::from_action_puzzle_hashes(
                &Self::action_puzzle_hashes(self.launcher_id, &self.constants),
                self.state,
                self.launcher_id,
            ),
        )
    }

    pub fn parse(
        allocator: &mut Allocator,
        puzzle: Puzzle,
        constants: XchandlesConstants,
    ) -> Result<Option<Self>, DriverError> {
        let Some(layers) = XchandlesRegistryLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        let action_puzzle_hashes = Self::action_puzzle_hashes(layers.launcher_id, &constants);
        let merkle_root = MerkleTree::new(&action_puzzle_hashes).root();
        if layers.inner_puzzle.merkle_root != merkle_root {
            return Ok(None);
        }

        Ok(Some(Self::from_layers(layers, constants)))
    }

    pub fn from_layers(layers: XchandlesRegistryLayers, constants: XchandlesConstants) -> Self {
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
            DefaultFinalizer2ndCurryArgs::curry_tree_hash(self.launcher_id),
            MerkleTree::new(&Self::action_puzzle_hashes(
                self.launcher_id,
                &self.constants,
            ))
            .root(),
            self.state.tree_hash(),
        )
    }
}

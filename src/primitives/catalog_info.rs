use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonArgs,
};
use chia_wallet_sdk::{DriverError, Layer, MerkleTree, Puzzle, SingletonLayer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::Allocator;
use hex_literal::hex;

use crate::{
    ActionLayer, ActionLayerArgs, CatalogRegisterActionArgs, DefaultFinalizerArgs,
    DelegatedStateActionArgs,
};

pub type CatalogLayers = SingletonLayer<ActionLayer<CatalogState>>;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm, Copy)]
#[clvm(list)]
pub struct CatalogState {
    pub registration_price: u64,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct CatalogConstants {
    pub royalty_address: Bytes32,
    pub royalty_ten_thousandths: u16,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
    pub price_singleton_launcher_id: Bytes32,
}

impl CatalogConstants {
    pub fn with_price_singleton(mut self, price_singleton_launcher_id: Bytes32) -> Self {
        self.price_singleton_launcher_id = price_singleton_launcher_id;
        self
    }
}

pub enum CatalogConstantsPresets {
    Testnet,
    Mainnet,
}

impl CatalogConstantsPresets {
    pub fn value(self) -> CatalogConstants {
        match self {
            CatalogConstantsPresets::Testnet => CatalogConstants {
                royalty_address: Bytes32::from([1; 32]).tree_hash().into(),
                royalty_ten_thousandths: 100,
                precommit_payout_puzzle_hash: Bytes32::from([2; 32]).tree_hash().into(),
                relative_block_height: 8,
                price_singleton_launcher_id: Bytes32::from(hex!(
                    "0000000000000000000000000000000000000000000000000000000000000000"
                )),
            },
            CatalogConstantsPresets::Mainnet => unimplemented!("oops - this isn't implemented yet"),
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct CatalogInfo {
    pub launcher_id: Bytes32,
    pub state: CatalogState,

    pub constants: CatalogConstants,
}

impl CatalogInfo {
    pub fn new(launcher_id: Bytes32, state: CatalogState, constants: CatalogConstants) -> Self {
        Self {
            launcher_id,
            state,
            constants,
        }
    }

    pub fn with_state(mut self, state: CatalogState) -> Self {
        self.state = state;
        self
    }

    pub fn action_puzzle_hashes(
        launcher_id: Bytes32,
        constants: &CatalogConstants,
    ) -> [Bytes32; 2] {
        let register_action_hash = CatalogRegisterActionArgs::curry_tree_hash(
            launcher_id,
            constants.royalty_address.tree_hash().into(),
            constants.royalty_ten_thousandths,
            constants.precommit_payout_puzzle_hash,
            constants.relative_block_height,
        )
        .tree_hash();

        let update_price_action_hash =
            DelegatedStateActionArgs::curry_tree_hash(constants.price_singleton_launcher_id);

        [register_action_hash.into(), update_price_action_hash.into()]
    }

    #[must_use]
    pub fn into_layers(self) -> CatalogLayers {
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
        constants: CatalogConstants,
    ) -> Result<Option<Self>, DriverError> {
        let Some(layers) = CatalogLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        let action_puzzle_hashes = Self::action_puzzle_hashes(layers.launcher_id, &constants);
        let merkle_root = MerkleTree::new(&action_puzzle_hashes).root;
        if layers.inner_puzzle.merkle_root != merkle_root {
            return Ok(None);
        }

        Ok(Some(Self::from_layers(layers, constants)))
    }

    pub fn from_layers(layers: CatalogLayers, constants: CatalogConstants) -> Self {
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
            DefaultFinalizerArgs::curry_tree_hash(self.launcher_id),
            MerkleTree::new(&Self::action_puzzle_hashes(
                self.launcher_id,
                &self.constants,
            ))
            .root,
            self.state.tree_hash(),
        )
    }
}

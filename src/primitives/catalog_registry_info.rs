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
    ActionLayer, ActionLayerArgs, CatalogRefundActionArgs, CatalogRegisterActionArgs,
    DefaultFinalizer2ndCurryArgs, DelegatedStateActionArgs,
};

pub type CatalogRegistryLayers = SingletonLayer<ActionLayer<CatalogRegistryState>>;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm, Copy)]
#[clvm(list)]
pub struct CatalogRegistryState {
    pub cat_maker_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub registration_price: u64,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct CatalogRegistryConstants {
    pub royalty_address: Bytes32,
    pub royalty_ten_thousandths: u16,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
    pub price_singleton_launcher_id: Bytes32,
}

impl CatalogRegistryConstants {
    pub fn get(testnet11: bool) -> Self {
        if testnet11 {
            return CatalogRegistryConstants {
                royalty_address: Bytes32::from(hex!(
                    "b3aea098428b2b5e6d57cf3bff6ee82e3950dec338b17df6d8ee20944787def5"
                )),
                royalty_ten_thousandths: 100,
                precommit_payout_puzzle_hash: Bytes32::from(hex!(
                    "b3aea098428b2b5e6d57cf3bff6ee82e3950dec338b17df6d8ee20944787def5"
                )),
                relative_block_height: 8,
                price_singleton_launcher_id: Bytes32::from(hex!(
                    "0000000000000000000000000000000000000000000000000000000000000000"
                )),
            };
        }

        todo!("oops - catalog constants for mainnet are not yet available");
    }

    pub fn with_price_singleton(mut self, price_singleton_launcher_id: Bytes32) -> Self {
        self.price_singleton_launcher_id = price_singleton_launcher_id;
        self
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct CatalogRegistryInfo {
    pub launcher_id: Bytes32,
    pub state: CatalogRegistryState,

    pub constants: CatalogRegistryConstants,
}

impl CatalogRegistryInfo {
    pub fn new(
        launcher_id: Bytes32,
        state: CatalogRegistryState,
        constants: CatalogRegistryConstants,
    ) -> Self {
        Self {
            launcher_id,
            state,
            constants,
        }
    }

    pub fn with_state(mut self, state: CatalogRegistryState) -> Self {
        self.state = state;
        self
    }

    pub fn action_puzzle_hashes(
        launcher_id: Bytes32,
        constants: &CatalogRegistryConstants,
    ) -> [Bytes32; 3] {
        let register_action_hash = CatalogRegisterActionArgs::curry_tree_hash(
            launcher_id,
            constants.royalty_address.tree_hash().into(),
            constants.royalty_ten_thousandths,
            constants.relative_block_height,
            constants.precommit_payout_puzzle_hash,
        )
        .tree_hash();

        let refund_action_hash = CatalogRefundActionArgs::curry_tree_hash(
            launcher_id,
            constants.relative_block_height,
            constants.precommit_payout_puzzle_hash,
        )
        .tree_hash();

        let update_price_action_hash =
            DelegatedStateActionArgs::curry_tree_hash(constants.price_singleton_launcher_id);

        [
            register_action_hash.into(),
            refund_action_hash.into(),
            update_price_action_hash.into(),
        ]
    }

    #[must_use]
    pub fn into_layers(self) -> CatalogRegistryLayers {
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
        constants: CatalogRegistryConstants,
    ) -> Result<Option<Self>, DriverError> {
        let Some(layers) = CatalogRegistryLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        let action_puzzle_hashes = Self::action_puzzle_hashes(layers.launcher_id, &constants);
        let merkle_root = MerkleTree::new(&action_puzzle_hashes).root();
        if layers.inner_puzzle.merkle_root != merkle_root {
            return Ok(None);
        }

        Ok(Some(Self::from_layers(layers, constants)))
    }

    pub fn from_layers(layers: CatalogRegistryLayers, constants: CatalogRegistryConstants) -> Self {
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

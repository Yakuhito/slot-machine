use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonArgs,
};
use chia_wallet_sdk::{DriverError, Layer, MerkleTree, Puzzle, SingletonLayer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::{
    Action, ActionLayer, ActionLayerArgs, CatalogRegisterAction, CatalogRegisterActionArgs,
    CatalogRegisterActionSolution, DelegatedStateAction, DelegatedStateActionSolution,
};

pub type CatalogLayers = SingletonLayer<ActionLayer<CatalogState>>;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm)]
#[clvm(list)]
pub struct CatalogState {
    pub registration_price: u64,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogConstants {
    pub royalty_address_hash: Bytes32,
    pub trade_price_percentage: u8,
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
                royalty_address_hash: Bytes32::from([1; 32]).tree_hash().into(),
                trade_price_percentage: 100,
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

pub enum CatalogAction {
    Register(CatalogRegisterAction),
    UpdatePrice(DelegatedStateAction),
}

pub enum CatalogActionSolution {
    Register(CatalogRegisterActionSolution),
    UpdatePrice(DelegatedStateActionSolution<CatalogState>),
}

impl Action for CatalogAction {
    type Solution = CatalogActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        match self {
            CatalogAction::Register(action) => action.construct_puzzle(ctx),
            CatalogAction::UpdatePrice(action) => action.construct_puzzle(ctx),
        }
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        match self {
            CatalogAction::Register(action) => {
                let CatalogActionSolution::Register(solution) = solution else {
                    return Err(DriverError::Custom("Invalid solution".to_string()));
                };

                action.construct_solution(ctx, solution)
            }
            CatalogAction::UpdatePrice(action) => {
                let CatalogActionSolution::UpdatePrice(solution) = solution else {
                    return Err(DriverError::Custom("Invalid solution".to_string()));
                };

                let new_state = solution.new_state.to_clvm(&mut ctx.allocator)?;
                action.construct_solution(
                    ctx,
                    DelegatedStateActionSolution {
                        new_state,
                        other_singleton_inner_puzzle_hash: solution
                            .other_singleton_inner_puzzle_hash,
                    },
                )
            }
        }
    }
}

impl ToTreeHash for CatalogAction {
    fn tree_hash(&self) -> TreeHash {
        match self {
            CatalogAction::Register(action) => action.tree_hash(),
            CatalogAction::UpdatePrice(action) => action.tree_hash(),
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
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
    ) -> Vec<Bytes32> {
        let register_action_hash = CatalogRegisterActionArgs::new(
            launcher_id,
            constants.royalty_address_hash,
            constants.trade_price_percentage,
            constants.precommit_payout_puzzle_hash,
            constants.relative_block_height,
        )
        .tree_hash();

        let update_price_action_hash =
            DelegatedStateAction::new(constants.price_singleton_launcher_id).tree_hash();

        vec![register_action_hash.into(), update_price_action_hash.into()]
    }

    #[must_use]
    pub fn into_layers(self) -> CatalogLayers {
        SingletonLayer::new(
            self.launcher_id,
            ActionLayer::new(
                Self::action_puzzle_hashes(self.launcher_id, &self.constants),
                self.state,
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
        if layers.inner_puzzle.action_puzzle_hashes != action_puzzle_hashes {
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
            MerkleTree::new(&Self::action_puzzle_hashes(
                self.launcher_id,
                &self.constants,
            ))
            .root,
            self.state.tree_hash(),
        )
    }
}

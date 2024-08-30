use chia::protocol::Bytes32;
use chia_wallet_sdk::{DriverError, SingletonLayer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;

use crate::{Action, ActionLayer, CatalogRegisterAction, DelegatedStateAction};

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
}

pub enum CatalogAction {
    Register(CatalogRegisterAction),
    UpdatePrice(DelegatedStateAction),
}

impl Action for CatalogAction {
    type Solution = NodePtr;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        match self {
            CatalogAction::Register(action) => action.construct_puzzle(ctx),
            CatalogAction::UpdatePrice(action) => action.construct_puzzle(ctx),
        }
    }

    fn puzzle_hash(&self, ctx: &mut chia_wallet_sdk::SpendContext) -> chia::clvm_utils::TreeHash {
        match self {
            CatalogAction::Register(action) => action.puzzle_hash(ctx),
            CatalogAction::UpdatePrice(action) => action.puzzle_hash(ctx),
        }
    }
}

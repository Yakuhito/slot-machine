use chia::protocol::Bytes32;
use chia_wallet_sdk::SingletonLayer;
use clvm_traits::{FromClvm, ToClvm};

use crate::ActionLayer;

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

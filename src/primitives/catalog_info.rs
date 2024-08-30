use chia::protocol::Bytes32;
use chia_wallet_sdk::SingletonLayer;
use clvm_traits::{FromClvm, ToClvm};

type CatalogLayers = SingletonLayer<ActionLayer<CatalogState>>;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, ToClvm, FromClvm)]
#[clvm(list)]
pub struct CatalogState {
    pub registration_price: u64,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogInfo {
    pub launcher_id: Bytes32,
    pub royalty_address_hash: Bytes32,
    pub trade_price_percentage: u8,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
    pub price_singleton_launcher_id: Bytes32,

    pub state: CatalogState,
}

impl CatalogInfo {
    pub fn new(
        launcher_id: Bytes32,
        royalty_address_hash: Bytes32,
        trade_price_percentage: u8,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
        price_singleton_launcher_id: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            royalty_address_hash,
            trade_price_percentage,
            precommit_payout_puzzle_hash,
            relative_block_height,
            price_singleton_launcher_id,
        }
    }
}

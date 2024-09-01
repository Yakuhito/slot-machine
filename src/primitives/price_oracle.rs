use chia::{
    protocol::{Bytes32, Coin},
    puzzles::Proof,
};
use chia_wallet_sdk::SingletonLayer;
use once_cell::sync::Lazy;

use crate::PriceLayer;

pub type PriceOracleLayers = SingletonLayer<PriceLayer>;

// https://docs.chia.net/block-rewards/#rewards-schedule
pub static BLOCK_REWARD_SCHEDULE: Lazy<Vec<(u32, u64)>> = Lazy::new(|| {
    vec![
        (10_091_520, 500_000_000_000),
        (15_137_280, 250_000_000_000),
        (20_183_040, 125_000_000_000),
    ]
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceOracle {
    pub coin: Coin,
    pub proof: Proof,

    pub launcher_id: Bytes32,

    pub price_schedule: Vec<(u32, u64)>,
    pub generation: u32,
    pub other_singleton_puzzle_hash: Bytes32,
}

impl PriceOracle {
    pub fn new(
        coin: Coin,
        proof: Proof,
        launcher_id: Bytes32,
        price_schedule: Vec<(u32, u64)>,
        generation: u32,
        other_singleton_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            coin,
            proof,
            launcher_id,
            price_schedule,
            generation,
            other_singleton_puzzle_hash,
        }
    }
}

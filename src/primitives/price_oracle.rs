use chia::{
    clvm_utils::TreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, Proof},
};
use chia_wallet_sdk::{DriverError, Layer, SingletonLayer, Spend, SpendContext};
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

    #[must_use]
    pub fn into_layers(self) -> PriceOracleLayers {
        SingletonLayer::new(
            self.launcher_id,
            PriceLayer::new(
                self.launcher_id,
                self.price_schedule,
                self.generation,
                self.other_singleton_puzzle_hash,
            ),
        )
    }

    pub fn inner_puzzle_hash(&self, ctx: &mut SpendContext) -> Result<TreeHash, DriverError> {
        let inner_puzzle = PriceLayer::new(
            self.launcher_id,
            self.price_schedule.clone(),
            self.generation,
            self.other_singleton_puzzle_hash,
        )
        .construct_puzzle(ctx)?;

        Ok(ctx.tree_hash(inner_puzzle))
    }

    pub fn child(self) -> Self {
        let generation = if self.generation < self.price_schedule.len() as u32 {
            self.generation + 1
        } else {
            self.generation
        };

        Self {
            coin: self.coin,
            proof: self.proof,
            launcher_id: self.launcher_id,
            price_schedule: self.price_schedule,
            generation,
            other_singleton_puzzle_hash: self.other_singleton_puzzle_hash,
        }
    }

    pub fn spend(self, ctx: &mut SpendContext) -> Result<(), DriverError> {
        let lineage_proof = self.proof;
        let coin = self.coin;

        let layers = self.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof,
                amount: coin.amount,
                inner_solution: (),
            },
        )?;

        ctx.spend(coin, Spend::new(puzzle, solution))?;

        Ok(())
    }
}

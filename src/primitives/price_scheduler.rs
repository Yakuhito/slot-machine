use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::SingletonSolution, standard::DEFAULT_HIDDEN_PUZZLE_HASH, LineageProof, Proof,
    },
};
use chia_wallet_sdk::{DriverError, Layer, SingletonLayer, Spend, SpendContext};
use clvm_traits::clvm_list;
use once_cell::sync::Lazy;

use crate::{StateSchedulerLayer, StateSchedulerLayerArgs, StateSchedulerLayerSolution};

pub type PriceOracleLayers = SingletonLayer<StateSchedulerLayer>;

// https://docs.chia.net/block-rewards/#rewards-schedule
pub static BLOCK_REWARD_SCHEDULE: Lazy<Vec<(u32, u64)>> = Lazy::new(|| {
    vec![
        (10_091_520, 500_000_000_000),
        (15_137_280, 250_000_000_000),
        (20_183_040, 125_000_000_000),
    ]
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceScheduler {
    pub coin: Coin,
    pub proof: Proof,

    pub launcher_id: Bytes32,

    pub price_schedule: Vec<(u32, u64)>,
    pub generation: usize,
    pub other_singleton_launcher_id: Bytes32,
}

impl PriceScheduler {
    pub fn new(
        coin: Coin,
        proof: Proof,
        launcher_id: Bytes32,
        price_schedule: Vec<(u32, u64)>,
        generation: usize,
        other_singleton_launcher_id: Bytes32,
    ) -> Self {
        Self {
            coin,
            proof,
            launcher_id,
            price_schedule,
            generation,
            other_singleton_launcher_id,
        }
    }

    pub fn generation_inner_puzzle_hash_step(
        &self,
        new_state_hash: Bytes32,
        required_block_height: u32,
        new_puzzle_hash: Bytes32,
    ) -> TreeHash {
        StateSchedulerLayerArgs::curry_tree_hash(
            self.other_singleton_launcher_id,
            new_state_hash,
            required_block_height,
            new_puzzle_hash,
        )
    }

    pub fn generation_inner_puzzle_hash(&self, generation: usize) -> TreeHash {
        let mut inner_puzzle_hash = DEFAULT_HIDDEN_PUZZLE_HASH;

        let mut i = self.price_schedule.len();
        while i >= generation {
            let (required_block_height, new_price) = self.price_schedule[i];

            let new_state_hash = clvm_list!(new_price).tree_hash();
            inner_puzzle_hash = self.generation_inner_puzzle_hash_step(
                new_state_hash.into(),
                required_block_height,
                inner_puzzle_hash.into(),
            );

            i -= 1;
        }

        inner_puzzle_hash
    }

    #[must_use]
    pub fn into_layers(self) -> PriceOracleLayers {
        let (required_block_height, new_price) = self.price_schedule[self.generation];
        let new_state_hash = clvm_list!(new_price).tree_hash();

        SingletonLayer::new(
            self.launcher_id,
            StateSchedulerLayer::new(
                self.other_singleton_launcher_id,
                new_state_hash.into(),
                required_block_height,
                self.generation_inner_puzzle_hash(self.generation + 1)
                    .into(),
            ),
        )
    }

    pub fn inner_puzzle_hash(&self) -> TreeHash {
        self.generation_inner_puzzle_hash(self.generation)
    }

    pub fn child(self) -> Option<Self> {
        if self.generation >= self.price_schedule.len() {
            return None;
        };

        let child_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.inner_puzzle_hash().into(),
            parent_amount: self.coin.amount,
        });

        let child_puzzle_hash = self.generation_inner_puzzle_hash(self.generation + 1);
        let child_coin = Coin::new(self.coin.coin_id(), child_puzzle_hash.into(), 1);

        Some(Self {
            coin: child_coin,
            proof: child_proof,
            launcher_id: self.launcher_id,
            price_schedule: self.price_schedule,
            generation: self.generation + 1,
            other_singleton_launcher_id: self.other_singleton_launcher_id,
        })
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        other_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(), DriverError> {
        let lineage_proof = self.proof;
        let coin = self.coin;

        let layers = self.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof,
                amount: coin.amount,
                inner_solution: StateSchedulerLayerSolution {
                    other_singleton_inner_puzzle_hash,
                },
            },
        )?;

        ctx.spend(coin, Spend::new(puzzle, solution))?;

        Ok(())
    }
}

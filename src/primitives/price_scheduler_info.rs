use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::standard::DEFAULT_HIDDEN_PUZZLE_HASH,
};
use chia_wallet_sdk::{Condition, SingletonLayer};
use clvm_traits::clvm_list;
use once_cell::sync::Lazy;

use crate::{StateSchedulerLayer, StateSchedulerLayerArgs};

pub type PriceOracleLayers = SingletonLayer<StateSchedulerLayer>;
pub type PriceSchedule = Vec<(u32, u64)>;

// https://docs.chia.net/block-rewards/#rewards-schedule
pub static BLOCK_REWARD_SCHEDULE: Lazy<PriceSchedule> = Lazy::new(|| {
    vec![
        (10_091_520, 500_000_000_000),
        (15_137_280, 250_000_000_000),
        (20_183_040, 125_000_000_000),
    ]
});
pub static CATALOG_BLOCK_COST_MULTIPLIER: u64 = 8; // registering a CAT costs 8 blocks

pub fn price_schedule_for_catalog(testnet11: bool) -> PriceSchedule {
    if testnet11 {
        let pivot_block = 1_550_000;
        let mut schedule: PriceSchedule = Vec::with_capacity(8 + BLOCK_REWARD_SCHEDULE.len());

        for i in 0..BLOCK_REWARD_SCHEDULE.len() {
            schedule.push((
                pivot_block + (i as u32) * 4608,
                1_000_000_000_000 + 2 * i as u64,
            ));
        }
        for (block, mojo_reward) in BLOCK_REWARD_SCHEDULE.iter() {
            schedule.push((*block, mojo_reward * CATALOG_BLOCK_COST_MULTIPLIER));
        }
        return schedule;
    }

    BLOCK_REWARD_SCHEDULE
        .iter()
        .map(|(block, mojo_reward)| (*block, mojo_reward * CATALOG_BLOCK_COST_MULTIPLIER))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceSchedulerInfo {
    pub launcher_id: Bytes32,

    pub price_schedule: PriceSchedule,
    pub generation: usize,
    pub other_singleton_launcher_id: Bytes32,
}

impl PriceSchedulerInfo {
    pub fn new(
        launcher_id: Bytes32,
        price_schedule: PriceSchedule,
        generation: usize,
        other_singleton_launcher_id: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            price_schedule,
            generation,
            other_singleton_launcher_id,
        }
    }

    pub fn with_generation(self, generation: usize) -> Self {
        Self { generation, ..self }
    }

    pub fn generation_inner_puzzle_hash_step(
        &self,
        new_state_hash: Bytes32,
        required_block_height: u32,
        new_puzzle_hash: Bytes32,
    ) -> TreeHash {
        StateSchedulerLayerArgs::curry_tree_hash(
            self.other_singleton_launcher_id,
            vec![
                Condition::<()>::create_coin(new_puzzle_hash, 1, vec![]),
                Condition::assert_height_absolute(required_block_height),
            ],
            new_state_hash,
        )
    }

    pub fn generation_inner_puzzle_hash(&self, generation: usize) -> TreeHash {
        let mut inner_puzzle_hash = DEFAULT_HIDDEN_PUZZLE_HASH;

        let mut i = self.price_schedule.len() - 1;
        while i >= generation {
            let (required_block_height, new_price) = self.price_schedule[i];

            let new_state_hash = clvm_list!(new_price).tree_hash();
            inner_puzzle_hash = self.generation_inner_puzzle_hash_step(
                new_state_hash.into(),
                required_block_height,
                inner_puzzle_hash.into(),
            );

            if i > 0 {
                i -= 1;
            } else {
                break;
            }
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
}

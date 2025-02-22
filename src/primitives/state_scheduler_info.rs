use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{Condition, SingletonLayer};

use crate::{StateSchedulerLayer, StateSchedulerLayerArgs};

pub type StateSchedulerLayers<S> = SingletonLayer<StateSchedulerLayer<S>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSchedulerInfo<S> {
    pub launcher_id: Bytes32,

    pub other_singleton_launcher_id: Bytes32,
    pub state_schedule: Vec<(u32, S)>, // block height + state
    pub generation: usize,
    pub final_puzzle_hash: Bytes32,
}

impl<S> StateSchedulerInfo<S>
where
    S: ToTreeHash + Clone,
{
    pub fn new(
        launcher_id: Bytes32,
        other_singleton_launcher_id: Bytes32,
        state_schedule: Vec<(u32, S)>,
        generation: usize,
        final_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            other_singleton_launcher_id,
            state_schedule,
            generation,
            final_puzzle_hash,
        }
    }

    pub fn inner_puzzle_hash_for(
        &self,
        next_puzzle_hash: Bytes32,
        required_block_height: u32,
        new_state: S,
    ) -> TreeHash {
        StateSchedulerLayerArgs::curry_tree_hash(
            self.other_singleton_launcher_id,
            vec![
                Condition::<()>::create_coin(next_puzzle_hash, 1, None),
                Condition::assert_height_absolute(required_block_height),
            ],
            new_state,
        )
    }

    pub fn inner_puzzle_hash_for_generation(&self, generation: usize) -> TreeHash {
        if generation >= self.state_schedule.len() {
            return self.final_puzzle_hash.into();
        }

        let mut inner_puzzle_hash: TreeHash = self.final_puzzle_hash.into();

        let mut i = self.state_schedule.len() - 1;
        while i > generation {
            inner_puzzle_hash = self.inner_puzzle_hash_for(
                inner_puzzle_hash.into(),
                self.state_schedule[i].0,
                self.state_schedule[i].1.clone(),
            );

            i -= 1;
        }

        inner_puzzle_hash
    }

    pub fn inner_puzzle_hash(&self) -> TreeHash {
        self.inner_puzzle_hash_for_generation(self.generation)
    }

    pub fn into_layers(self) -> Option<StateSchedulerLayers<S>> {
        if self.generation >= self.state_schedule.len() {
            return None;
        }

        let (required_block_height, new_state) = self.state_schedule[self.generation].clone();

        Some(SingletonLayer::new(
            self.launcher_id,
            StateSchedulerLayer::new(
                self.other_singleton_launcher_id,
                new_state,
                required_block_height,
                self.inner_puzzle_hash_for_generation(self.generation + 1)
                    .into(),
            ),
        ))
    }
}

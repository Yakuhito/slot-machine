use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::{LauncherSolution, SingletonArgs},
};
use chia_wallet_sdk::{Condition, DriverError, SingletonLayer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{StateSchedulerLayer, StateSchedulerLayerArgs};

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

    pub fn with_generation(&self, generation: usize) -> Self {
        Self {
            generation,
            ..self.clone()
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

        let mut i = self.state_schedule.len();
        while i > generation {
            inner_puzzle_hash = self.inner_puzzle_hash_for(
                inner_puzzle_hash.into(),
                self.state_schedule[i - 1].0,
                self.state_schedule[i - 1].1.clone(),
            );

            i -= 1;
        }

        inner_puzzle_hash
    }

    pub fn inner_puzzle_hash(&self) -> TreeHash {
        self.inner_puzzle_hash_for_generation(self.generation)
    }

    pub fn into_layers(
        self,
        allocator: &mut Allocator,
    ) -> Result<SingletonLayer<StateSchedulerLayer<NodePtr>>, DriverError>
    where
        S: ToClvm<Allocator>,
    {
        if self.generation >= self.state_schedule.len() {
            return Err(DriverError::Custom("Generation out of bounds".to_string()));
        }

        let (required_block_height, new_state) = self.state_schedule[self.generation].clone();

        Ok(SingletonLayer::new(
            self.launcher_id,
            StateSchedulerLayer::new(
                self.other_singleton_launcher_id,
                new_state.to_clvm(allocator)?,
                required_block_height,
                self.inner_puzzle_hash_for_generation(self.generation + 1)
                    .into(),
            ),
        ))
    }

    pub fn from_launcher_solution<H>(
        allocator: &mut Allocator,
        laucher_solution: LauncherSolution<NodePtr>,
    ) -> Result<Option<(Self, H)>, DriverError>
    where
        S: FromClvm<Allocator>,
        H: FromClvm<Allocator>,
    {
        let hints = StateSchedulerLauncherHints::<S, H>::from_clvm(
            allocator,
            laucher_solution.key_value_list,
        )?;

        let candidate = Self::new(
            hints.my_launcher_id,
            hints.other_singleton_launcher_id,
            hints.state_schedule,
            0,
            hints.final_puzzle_hash,
        );

        let predicted_inner_puzzle_hash = candidate.inner_puzzle_hash();
        let predicted_puzzle_hash =
            SingletonArgs::curry_tree_hash(hints.my_launcher_id, predicted_inner_puzzle_hash);

        if laucher_solution.amount == 1
            && laucher_solution.singleton_puzzle_hash == predicted_puzzle_hash.into()
        {
            Ok(Some((candidate, hints.final_puzzle_hash_hints)))
        } else {
            Ok(None)
        }
    }

    pub fn to_hints<H>(&self, final_puzzle_hash_hints: H) -> StateSchedulerLauncherHints<S, H> {
        StateSchedulerLauncherHints {
            my_launcher_id: self.launcher_id,
            other_singleton_launcher_id: self.other_singleton_launcher_id,
            final_puzzle_hash: self.final_puzzle_hash,
            state_schedule: self.state_schedule.clone(),
            final_puzzle_hash_hints,
        }
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(curry)]
pub struct StateSchedulerLauncherHints<S, H> {
    pub my_launcher_id: Bytes32,
    pub other_singleton_launcher_id: Bytes32,
    pub final_puzzle_hash: Bytes32,
    pub state_schedule: Vec<(u32, S)>,
    #[clvm(rest)]
    pub final_puzzle_hash_hints: H,
}

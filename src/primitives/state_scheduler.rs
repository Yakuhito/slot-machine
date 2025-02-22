use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::{SingletonArgs, SingletonSolution},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{DriverError, Layer, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::Allocator;

use crate::{StateSchedulerInfo, StateSchedulerLayerSolution};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateScheduler<S>
where
    S: ToTreeHash + Clone + ToClvm<Allocator> + FromClvm<Allocator>,
{
    pub coin: Coin,
    pub proof: Proof,

    pub info: StateSchedulerInfo<S>,
}

impl<S> StateScheduler<S>
where
    S: ToTreeHash + Clone + ToClvm<Allocator> + FromClvm<Allocator>,
{
    pub fn new(coin: Coin, proof: Proof, info: StateSchedulerInfo<S>) -> Self {
        Self { coin, proof, info }
    }

    pub fn child(&self) -> Option<Self> {
        if self.info.generation >= self.info.state_schedule.len() {
            return None;
        };

        let child_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            parent_amount: self.coin.amount,
        });

        let child_puzzle_hash = SingletonArgs::curry_tree_hash(
            self.info.launcher_id,
            self.info
                .inner_puzzle_hash_for_generation(self.info.generation + 1),
        );
        let child_coin = Coin::new(self.coin.coin_id(), child_puzzle_hash.into(), 1);

        let new_generation = self.info.generation + 1;
        Some(Self {
            coin: child_coin,
            proof: child_proof,
            info: self.info.clone().with_generation(new_generation),
        })
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        other_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(), DriverError> {
        let lineage_proof = self.proof;
        let coin = self.coin;

        let layers = self.info.into_layers(&mut ctx.allocator)?;

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

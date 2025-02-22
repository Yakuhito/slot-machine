use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{
        singleton::{
            LauncherSolution, SingletonArgs, SingletonSolution, SINGLETON_LAUNCHER_PUZZLE_HASH,
        },
        EveProof, LineageProof, Proof,
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
        // check for both self.info.generation and self.info.generation + 1 to be < self.info.state_schedule.len()
        if self.info.generation > self.info.state_schedule.len() {
            return None;
        };

        let child_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            parent_amount: self.coin.amount,
        });

        let child_info = self.info.clone().with_generation(self.info.generation + 1);
        let child_inner_puzzle_hash = child_info.inner_puzzle_hash();

        Some(Self {
            coin: Coin::new(
                self.coin.coin_id(),
                SingletonArgs::curry_tree_hash(self.info.launcher_id, child_inner_puzzle_hash)
                    .into(),
                1,
            ),
            proof: child_proof,
            info: child_info,
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

    pub fn from_launcher_spend(
        ctx: &mut SpendContext,
        launcher_spend: CoinSpend,
    ) -> Result<Option<Self>, DriverError> {
        if launcher_spend.coin.puzzle_hash != SINGLETON_LAUNCHER_PUZZLE_HASH.into() {
            return Ok(None);
        }

        let solution = launcher_spend.solution.to_clvm(&mut ctx.allocator)?;
        let solution = LauncherSolution::from_clvm(&ctx.allocator, solution)?;

        let Some(info) = StateSchedulerInfo::from_launcher_solution(&mut ctx.allocator, solution)?
        else {
            return Ok(None);
        };

        Ok(Some(Self::new(
            launcher_spend.coin,
            Proof::Eve(EveProof {
                parent_parent_coin_info: launcher_spend.coin.parent_coin_info,
                parent_amount: launcher_spend.coin.amount,
            }),
            info,
        )))
    }
}

use chia::{
    clvm_utils::TreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{cat::CatArgs, singleton::SingletonSolution, LineageProof},
};
use chia_wallet_sdk::{
    driver::{Cat, CatSpend, DriverError, Layer, Spend, SpendContext},
    prelude::{CreateCoin, Memos},
    types::run_puzzle,
};
use clvm_traits::{clvm_list, clvm_quote, match_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    P2DelegatedBySingletonLayer, P2DelegatedBySingletonLayerArgs,
    P2DelegatedBySingletonLayerSolution, RawActionLayerSolution,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct Reserve {
    pub coin: Coin,
    pub asset_id: Bytes32,
    pub proof: LineageProof,
    pub inner_puzzle_hash: Bytes32,

    pub controller_singleton_struct_hash: Bytes32,
    pub nonce: u64,
}

pub trait Reserveful {
    fn reserve_amount(&self, index: u64) -> u64;
}

impl Reserve {
    pub fn new(
        parent_coin_id: Bytes32,
        proof: LineageProof,
        asset_id: Bytes32,
        controller_singleton_struct_hash: Bytes32,
        nonce: u64,
        amount: u64,
    ) -> Self {
        let inner_puzzle_hash = P2DelegatedBySingletonLayerArgs::curry_tree_hash(
            controller_singleton_struct_hash,
            nonce,
        );

        Self {
            coin: Coin::new(
                parent_coin_id,
                CatArgs::curry_tree_hash(asset_id, inner_puzzle_hash).into(),
                amount,
            ),
            proof,
            asset_id,
            inner_puzzle_hash: inner_puzzle_hash.into(),
            controller_singleton_struct_hash,
            nonce,
        }
    }

    pub fn puzzle_hash(
        asset_id: Bytes32,
        controller_singleton_struct_hash: Bytes32,
        nonce: u64,
    ) -> TreeHash {
        CatArgs::curry_tree_hash(
            asset_id,
            P2DelegatedBySingletonLayerArgs::curry_tree_hash(
                controller_singleton_struct_hash,
                nonce,
            ),
        )
    }

    pub fn construct_inner_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let layer =
            P2DelegatedBySingletonLayer::new(self.controller_singleton_struct_hash, self.nonce);

        layer.construct_puzzle(ctx)
    }

    pub fn to_cat(&self) -> Cat {
        Cat::new(
            self.coin,
            Some(self.proof),
            self.asset_id,
            self.inner_puzzle_hash,
        )
    }

    pub fn inner_spend(
        &self,
        ctx: &mut SpendContext,
        controller_singleton_inner_puzzle_hash: Bytes32,
        delegated_puzzle: NodePtr,
        delegated_solution: NodePtr,
    ) -> Result<Spend, DriverError> {
        P2DelegatedBySingletonLayer::new(self.controller_singleton_struct_hash, self.nonce)
            .construct_spend(
                ctx,
                P2DelegatedBySingletonLayerSolution {
                    singleton_inner_puzzle_hash: controller_singleton_inner_puzzle_hash,
                    delegated_puzzle,
                    delegated_solution,
                },
            )
    }

    pub fn delegated_puzzle_for_finalizer_controller<S>(
        &self,
        ctx: &mut SpendContext,
        controlelr_initial_state: S,
        controller_solution: NodePtr,
    ) -> Result<NodePtr, DriverError>
    where
        S: ToClvm<Allocator> + FromClvm<Allocator> + Clone + Reserveful,
    {
        let controller_solution = ctx.extract::<SingletonSolution<
            RawActionLayerSolution<NodePtr, NodePtr, NodePtr>,
        >>(controller_solution)?;

        let mut state: S = controlelr_initial_state;
        let mut reserve_conditions: Vec<NodePtr> = Vec::new();
        for raw_action in controller_solution.inner_solution.actions {
            let actual_solution = ctx.alloc(&clvm_list!(state, raw_action.action_solution))?;

            let output = run_puzzle(ctx, raw_action.action_puzzle_reveal, actual_solution)?;
            let (new_state, conditions) =
                ctx.extract::<match_tuple!(S, Vec<(i64, NodePtr)>)>(output)?;
            state = new_state;

            for (opcode, cond) in conditions {
                if opcode == -42 {
                    reserve_conditions.push(cond);
                }
            }
        }

        // prepend CREATE_COIN, just like the reserve finalizer does
        // (list CREATE_COIN RESERVE_INNER_PUZZLE_HASH (f New_State) (list RESERVE_INNER_PUZZLE_HASH))
        let new_reserve_amount = state.reserve_amount(0);
        let cc = CreateCoin::new(
            self.inner_puzzle_hash,
            new_reserve_amount,
            Some(Memos::new(vec![self.inner_puzzle_hash])),
        );
        reserve_conditions.insert(0, ctx.alloc(&cc)?);

        let delegated_puzzle = ctx.alloc(&clvm_quote!(reserve_conditions))?;

        Ok(delegated_puzzle)
    }

    pub fn cat_spend_for_reserve_finalizer_controller<S>(
        &self,
        ctx: &mut SpendContext,
        controlelr_initial_state: S,
        controller_singleton_inner_puzzle_hash: Bytes32,
        controller_solution: NodePtr,
    ) -> Result<CatSpend, DriverError>
    where
        S: ToClvm<Allocator> + FromClvm<Allocator> + Clone + Reserveful,
    {
        let delegated_puzzle = self.delegated_puzzle_for_finalizer_controller(
            ctx,
            controlelr_initial_state,
            controller_solution,
        )?;

        Ok(CatSpend::new(
            self.to_cat(),
            self.inner_spend(
                ctx,
                controller_singleton_inner_puzzle_hash,
                delegated_puzzle,
                NodePtr::NIL,
            )?,
        ))
    }
}

use chia::{
    clvm_utils::TreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        cat::{CatArgs, CatSolution},
        singleton::SingletonSolution,
        CoinProof, LineageProof,
    },
};
use chia_wallet_sdk::{
    run_puzzle, Cat, CatLayer, CreateCoin, DriverError, Layer, Memos, Spend, SpendContext,
};
use clvm_traits::{clvm_list, clvm_quote, match_tuple, FromClvm, ToClvm};
use clvmr::{serde::node_to_bytes, Allocator, NodePtr};

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

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let layers = CatLayer::<P2DelegatedBySingletonLayer>::new(
            self.asset_id,
            P2DelegatedBySingletonLayer::new(self.controller_singleton_struct_hash, self.nonce),
        );

        layers.construct_puzzle(ctx)
    }

    pub fn to_cat(&self) -> Cat {
        Cat::new(
            self.coin,
            Some(self.proof),
            self.asset_id,
            self.inner_puzzle_hash,
        )
    }

    pub fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        controller_singleton_inner_puzzle_hash: Bytes32,
        delegated_puzzle: NodePtr,
        delegated_solution: NodePtr,
    ) -> Result<NodePtr, DriverError> {
        let layers = CatLayer::<P2DelegatedBySingletonLayer>::new(
            self.asset_id,
            P2DelegatedBySingletonLayer::new(self.controller_singleton_struct_hash, self.nonce),
        );

        layers.construct_solution(
            ctx,
            CatSolution {
                inner_puzzle_solution: P2DelegatedBySingletonLayerSolution {
                    singleton_inner_puzzle_hash: controller_singleton_inner_puzzle_hash,
                    delegated_puzzle,
                    delegated_solution,
                },
                lineage_proof: Some(self.proof),
                prev_coin_id: self.coin.coin_id(),
                this_coin_info: self.coin,
                next_coin_proof: CoinProof {
                    parent_coin_info: self.coin.parent_coin_info,
                    inner_puzzle_hash: self.inner_puzzle_hash,
                    amount: self.coin.amount,
                },
                prev_subtotal: 0,
                extra_delta: 0,
            },
        )
    }

    pub fn spend(
        &self,
        ctx: &mut SpendContext,
        controller_singleton_inner_puzzle_hash: Bytes32,
        delegated_puzzle: NodePtr,
        delegated_solution: NodePtr,
    ) -> Result<(), DriverError> {
        let puzzle = self.construct_puzzle(ctx)?;
        let solution = self.construct_solution(
            ctx,
            controller_singleton_inner_puzzle_hash,
            delegated_puzzle,
            delegated_solution,
        )?;

        ctx.spend(self.coin, Spend::new(puzzle, solution))
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
        new_reserve_amount: u64,
        controller_solution: NodePtr,
    ) -> Result<NodePtr, DriverError>
    where
        S: ToClvm<Allocator> + FromClvm<Allocator> + Clone,
    {
        let controller_solution = SingletonSolution::<
            RawActionLayerSolution<NodePtr, NodePtr, NodePtr>,
        >::from_clvm(&ctx.allocator, controller_solution)?;

        let mut state: S = controlelr_initial_state;
        let mut reserve_conditions: Vec<NodePtr> = Vec::new();
        for raw_action in controller_solution.inner_solution.actions {
            let actual_solution =
                clvm_list!(state, raw_action.action_solution).to_clvm(&mut ctx.allocator)?;

            let output = run_puzzle(
                &mut ctx.allocator,
                raw_action.action_puzzle_reveal,
                actual_solution,
            )?;
            let (new_state, conditions) =
                <match_tuple!(S, Vec<(i64, NodePtr)>)>::from_clvm(&ctx.allocator, output)?;
            state = new_state;

            for (opcode, cond) in conditions {
                if opcode == -42 {
                    reserve_conditions.push(cond);
                }
            }
        }

        // prepend CREATE_COIN, just like the reserve finalizer does
        // (list CREATE_COIN RESERVE_INNER_PUZZLE_HASH (f New_State) (list RESERVE_INNER_PUZZLE_HASH))
        let cc = CreateCoin::new(
            self.inner_puzzle_hash,
            new_reserve_amount,
            Some(Memos::new(vec![self.inner_puzzle_hash])),
        );
        reserve_conditions.insert(0, cc.to_clvm(&mut ctx.allocator)?);

        let delegated_puzzle = clvm_quote!(reserve_conditions).to_clvm(&mut ctx.allocator)?;
        println!(
            "delegated_puzzle: {:?}",
            hex::encode(node_to_bytes(&ctx.allocator, delegated_puzzle)?)
        );
        println!(
            "delegated_puzzle hash: {:?}",
            ctx.tree_hash(delegated_puzzle)
        );

        Ok(delegated_puzzle)
    }

    pub fn spend_for_reserve_finalizer_controller<S>(
        &self,
        ctx: &mut SpendContext,
        controlelr_initial_state: S,
        new_reserve_amount: u64,
        controller_singleton_inner_puzzle_hash: Bytes32,
        controller_solution: NodePtr,
    ) -> Result<(), DriverError>
    where
        S: ToClvm<Allocator> + FromClvm<Allocator> + Clone,
    {
        let delegated_puzzle = self.delegated_puzzle_for_finalizer_controller(
            ctx,
            controlelr_initial_state,
            new_reserve_amount,
            controller_solution,
        )?;

        let puzzle = self.construct_puzzle(ctx)?;
        let solution = self.construct_solution(
            ctx,
            controller_singleton_inner_puzzle_hash,
            delegated_puzzle,
            NodePtr::NIL,
        )?;

        ctx.spend(self.coin, Spend::new(puzzle, solution))
    }
}

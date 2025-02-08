use chia::{
    clvm_utils::TreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        cat::{CatArgs, CatSolution},
        CoinProof, LineageProof,
    },
};
use chia_wallet_sdk::{CatLayer, DriverError, Layer, Spend, SpendContext};
use clvmr::NodePtr;

use crate::{
    P2DelegatedBySingletonLayer, P2DelegatedBySingletonLayerArgs,
    P2DelegatedBySingletonLayerSolution,
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
}

use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{cat::CatArgs, singleton::SingletonStruct, Proof},
};
use chia_wallet_sdk::{CatLayer, DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::{PrecommitLayer, SpendContextExt};

#[derive(Debug, Clone)]
#[must_use]
pub struct PrecommitCoin<V> {
    pub coin: Coin,
    pub asset_id: Bytes32,
    pub proof: Proof,

    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub value: V,
}

impl<V> PrecommitCoin<V> {
    pub fn new(
        ctx: &mut SpendContext,
        parent_coin_id: Bytes32,
        proof: Proof,
        asset_id: Bytes32,
        launcher_id: Bytes32,
        relative_block_height: u32,
        precommit_payout_puzzle_hash: Bytes32,
        value: V,
        precommit_amount: u64,
    ) -> Result<Self, DriverError>
    where
        V: ToClvm<Allocator> + Clone,
    {
        let value_ptr = ctx.alloc(&value)?;
        let value_hash = ctx.tree_hash(value_ptr);

        Ok(Self {
            coin: Coin::new(
                parent_coin_id,
                PrecommitCoin::<V>::puzzle_hash(
                    asset_id,
                    launcher_id,
                    relative_block_height,
                    precommit_payout_puzzle_hash,
                    value_hash,
                )
                .into(),
                precommit_amount,
            ),
            proof,
            asset_id,
            launcher_id,
            relative_block_height,
            precommit_payout_puzzle_hash,
            value,
        })
    }

    pub fn puzzle_hash(
        asset_id: Bytes32,
        launcher_id: Bytes32,
        relative_block_height: u32,
        precommit_payout_puzzle_hash: Bytes32,
        value_hash: TreeHash,
    ) -> TreeHash {
        CatArgs::curry_tree_hash(
            asset_id,
            PrecommitLayer::<V>::puzzle_hash(
                launcher_id,
                relative_block_height,
                precommit_payout_puzzle_hash,
                value_hash,
            ),
        )
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>
    where
        V: ToClvm<Allocator> + Clone,
    {
        let layers = CatLayer::<PrecommitLayer<V>>::new(
            self.asset_id,
            PrecommitLayer::<V>::new(
                self.launcher_id,
                self.relative_block_height,
                self.precommit_payout_puzzle_hash,
                self.value,
            ),
        );

        layers.construct_puzzle(ctx)
    }
}

use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone)]
#[must_use]
pub struct PrecommitCoin<V> {
    pub coin: Coin,
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub value: V,
    pub precommit_amount: u64,
}

impl<V> PrecommitCoin<V> {
    pub fn new(
        ctx: &mut SpendContext,
        parent_coin_id: Bytes32,
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
                    launcher_id,
                    relative_block_height,
                    precommit_payout_puzzle_hash,
                    value_hash,
                )
                .into(),
                precommit_amount,
            ),
            launcher_id,
            relative_block_height,
            precommit_payout_puzzle_hash,
            value,
            precommit_amount,
        })
    }

    pub fn first_curry_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        precommit_payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: PRECOMMIT_COIN_PUZZLE_HASH,
            args: PrecommitCoin1stCurryArgs {
                singleton_struct: SingletonStruct::new(launcher_id),
                relative_block_height,
                precommit_payout_puzzle_hash,
            },
        }
        .tree_hash()
    }

    pub fn puzzle_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        precommit_payout_puzzle_hash: Bytes32,
        value_hash: TreeHash,
    ) -> TreeHash {
        CurriedProgram {
            program: Self::first_curry_hash(
                launcher_id,
                relative_block_height,
                precommit_payout_puzzle_hash,
            ),
            args: PrecommitCoin2ndCurryArgs { value: value_hash },
        }
        .tree_hash()
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>
    where
        V: ToClvm<Allocator> + Clone,
    {
        let prog_1st_curry = CurriedProgram {
            program: ctx.precommit_coin_puzzle()?,
            args: PrecommitCoin1stCurryArgs {
                singleton_struct: SingletonStruct::new(self.launcher_id),
                relative_block_height: self.relative_block_height,
                precommit_payout_puzzle_hash: self.precommit_payout_puzzle_hash,
            },
        }
        .to_clvm(&mut ctx.allocator)?;

        Ok(CurriedProgram {
            program: prog_1st_curry,
            args: PrecommitCoin2ndCurryArgs {
                value: self.value.clone(),
            },
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(), DriverError>
    where
        V: ToClvm<Allocator> + Clone,
    {
        let puzzle_reveal = self.construct_puzzle(ctx)?;
        let puzzle_reveal = ctx.serialize(&puzzle_reveal)?;

        let solution = ctx.serialize(&PrecommitCoinSolution {
            precommit_amount: self.coin.amount,
            singleton_inner_puzzle_hash,
        })?;

        ctx.insert(CoinSpend::new(self.coin, puzzle_reveal, solution));

        Ok(())
    }
}

pub const PRECOMMIT_COIN_PUZZLE: [u8; 526] = hex!("ff02ffff01ff04ffff04ff18ffff04ff5fff808080ffff04ffff04ff14ffff04ff17ffff04ff5fff80808080ffff04ffff04ff10ffff04ff0bff808080ffff04ffff04ff2cffff04ffff0112ffff04ff5fffff04ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff3effff04ff02ffff04ff05ff80808080ffff04ff81bfff808080808080ff8080808080ff8080808080ffff04ffff01ffffff5249ff33ff4302ffffff02ffff03ff05ffff01ff0bff7affff02ff2effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016a80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff5affff02ff2effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff3cffff0bff3cff6aff0580ffff0bff3cff0bff4a8080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff3effff04ff02ffff04ff09ff80808080ffff02ff3effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const PRECOMMIT_COIN_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    825ffe9a6c747756835ea074f5c45b78478bf9bf50377e2788a5832424fdccc4
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PrecommitCoin1stCurryArgs {
    pub singleton_struct: SingletonStruct,
    pub relative_block_height: u32,
    pub precommit_payout_puzzle_hash: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PrecommitCoin2ndCurryArgs<V> {
    pub value: V,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(solution)]
pub struct PrecommitCoinSolution {
    pub precommit_amount: u64,
    pub singleton_inner_puzzle_hash: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrecommitValue<T = NodePtr> {
    pub initial_inner_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub tail_reveal: T,
}

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesSecretAndName {
    pub secret: Bytes32,
    #[clvm(rest)]
    pub name: String,
}

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesPrecommitValue {
    pub secret_and_name: XchandlesSecretAndName,
    pub name_nft_launcher_id: Bytes32,
    #[clvm(rest)]
    pub start_time: u64,
}

impl XchandlesPrecommitValue {
    pub fn new(
        secret: Bytes32,
        name: String,
        name_nft_launcher_id: Bytes32,
        start_time: u64,
    ) -> Self {
        Self {
            secret_and_name: XchandlesSecretAndName { secret, name },
            name_nft_launcher_id,
            start_time,
        }
    }
}

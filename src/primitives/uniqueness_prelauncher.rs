use chia::{clvm_utils::{CurriedProgram, ToTreeHash, TreeHash}, protocol::{Bytes32, Coin, CoinSpend}, puzzles::singleton::{SINGLETON_LAUNCHER_PUZZLE, SINGLETON_LAUNCHER_PUZZLE_HASH}};
use chia_wallet_sdk::{DriverError, Launcher, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;


#[derive(Debug, Clone)]
#[must_use]
pub struct UniquenessPrelauncher<V> {
    pub coin: Coin,
    pub value: V,
}

impl<V> UniquenessPrelauncher<V> {
    pub fn from_coin(coin: Coin, value: V) -> Self {
        Self { coin, value }
    }

    pub fn new(ctx: &mut SpendContext,parent_coin_id: Bytes32, value: V) -> Result<Self, DriverError> where V: ToClvm<Allocator> + Clone { 
        let value_ptr = ctx.alloc(&value)?;
        let value_hash = ctx.tree_hash(value_ptr);

        Ok(Self::from_coin(
            Coin::new(
                parent_coin_id,
                UniquenessPrelauncher::<V>::puzzle_hash(value_hash).into(),
                0,
            ),
            value,
        ))
    }

    pub fn puzzle_hash(value_hash: TreeHash) -> TreeHash {
        let tree_hash_1st_curry = CurriedProgram {
            program: UNIQUENESS_PRELAUNCHER_PUZZLE_HASH,
            args: UniquenessPrelauncher1stCurryArgs {
                launcher_puzzle_hash: SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
            },
        }.tree_hash();

        CurriedProgram {
            program: tree_hash_1st_curry,
            args: UniquenessPrelauncher2ndCurryArgs {
                value: value_hash,
            },
        }.tree_hash()
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> where V: ToClvm<Allocator> + Clone {
        let prog_1st_curry = CurriedProgram {
            program: ctx.uniqueness_prelauncher_puzzle()?,
            args: UniquenessPrelauncher1stCurryArgs {
                launcher_puzzle_hash: SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
            },
        }.to_clvm(&mut ctx.allocator)?;

        Ok(CurriedProgram {
            program: prog_1st_curry,
            args: UniquenessPrelauncher2ndCurryArgs {
                value: self.value.clone(),
            },
        }.to_clvm(&mut ctx.allocator)?)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
    ) -> Result<Launcher, DriverError>
    where
        V: ToClvm<Allocator> + Clone,
    {
        let puzzle_reveal = self.construct_puzzle(ctx)?;
        let puzzle_reveal = ctx.serialize(&puzzle_reveal)?;

        let solution = ctx.serialize(&NodePtr::NIL)?;

        ctx.insert(CoinSpend::new(
            self.coin,
            puzzle_reveal,
            solution
        ));

        Ok(Launcher::new(self.coin.coin_id(), 1))
    }
}

pub const UNIQUENESS_PRELAUNCHER_PUZZLE: [u8; 59] = hex!("ff02ffff01ff04ffff04ff04ffff04ff05ffff01ff01808080ffff04ffff04ff06ffff04ff0bff808080ff808080ffff04ffff01ff333eff018080");

pub const  UNIQUENESS_PRELAUNCHER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e5759069429397243b808748e5bd5270ea0891953ea06df9a46b87ce4ade466
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct UniquenessPrelauncher1stCurryArgs {
    pub launcher_puzzle_hash: Bytes32,
}


#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct UniquenessPrelauncher2ndCurryArgs<V> {
    pub value: V,
}
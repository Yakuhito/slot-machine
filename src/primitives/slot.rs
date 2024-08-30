use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, Launcher, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone)]
#[must_use]
pub struct Slot {
    pub coin: Coin,
    pub launcher_id: Bytes32,
    pub value_hash: Bytes32,
}

impl Slot {
    pub fn new(
        parent_coin_id: Bytes32,
        launcher_id: Bytes32,
        value_hash: Bytes32,
    ) -> Result<Self, DriverError> {
        Ok(Self {
            coin: Coin::new(
                parent_coin_id,
                Slot::puzzle_hash(launcher_id, value_hash).into(),
                0,
            ),
            launcher_id,
            value_hash,
        })
    }

    pub fn first_curry_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: SLOT_PUZZLE_HASH,
            args: Slot1stCurryArgs {
                singleton_struct: SingletonStruct::new(launcher_id),
            },
        }
        .tree_hash()
    }

    pub fn puzzle_hash(launcher_id: Bytes32, value_hash: Bytes32) -> TreeHash {
        CurriedProgram {
            program: Self::first_curry_hash(launcher_id),
            args: Slot2ndCurryArgs { value_hash },
        }
        .tree_hash()
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let prog_1st_curry = CurriedProgram {
            program: ctx.slot_puzzle()?,
            args: Slot1stCurryArgs {
                singleton_struct: SingletonStruct::new(self.launcher_id),
            },
        }
        .to_clvm(&mut ctx.allocator)?;

        Ok(CurriedProgram {
            program: prog_1st_curry,
            args: Slot2ndCurryArgs {
                value_hash: self.value_hash,
            },
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        solution: SlotSolution,
    ) -> Result<Launcher, DriverError> {
        let puzzle_reveal = self.construct_puzzle(ctx)?;
        let puzzle_reveal = ctx.serialize(&puzzle_reveal)?;

        let solution = ctx.serialize(&solution)?;

        ctx.insert(CoinSpend::new(self.coin, puzzle_reveal, solution));

        Ok(Launcher::new(self.coin.coin_id(), 1))
    }
}

pub const SLOT_PUZZLE: [u8; 533] = hex!("ff02ffff01ff04ffff04ff10ffff04ffff30ff17ffff02ff3effff04ff02ffff04ff05ffff04ff2fff8080808080ffff010180ff808080ffff04ffff04ff18ffff04ffff0112ffff04ff80ffff04ffff02ff3effff04ff02ffff04ff5fff80808080ff8080808080ff808080ffff04ffff01ffffff4743ff02ff02ffff03ff05ffff01ff0bff72ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff1cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff0bff52ffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff1cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff14ffff0bff14ff62ff0580ffff0bff14ff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff02ff1affff04ff02ffff04ff09ffff04ffff02ff2effff04ff02ffff04ff05ff80808080ffff04ff0bff808080808080ff018080");

pub const SLOT_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e5759069429397243b808748e5bd5270ea0891953ea06df9a46b87ce4ade466
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct Slot1stCurryArgs {
    pub singleton_struct: SingletonStruct,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct Slot2ndCurryArgs {
    pub value_hash: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct SlotSolution {
    pub parent_parent_info: Bytes32,
    pub parent_inner_puzzle_hash: Bytes32,
    pub spender_inner_puzzle_hash: Bytes32,
}
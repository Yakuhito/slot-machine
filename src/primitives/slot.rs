use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, Launcher, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;
use std::cmp::Ordering;

use crate::SpendContextExt;

// the values below are for slots organized into a double-linked ordered list
// the minimum possible value of an slot - this will be contained by one of the ends of the list
pub static SLOT32_MIN_VALUE: [u8; 32] =
    hex!("8000000000000000000000000000000000000000000000000000000000000000");
// the maximum possible value of a slot - will be contained by the other end of the list
pub static SLOT32_MAX_VALUE: [u8; 32] =
    hex!("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

#[derive(Debug, Clone)]
#[must_use]
pub struct Slot<V> {
    pub coin: Coin,
    pub launcher_id: Bytes32,
    pub value_hash: Bytes32,
    pub value: Option<V>,
}

impl<V> Slot<V> {
    pub fn new(
        parent_coin_id: Bytes32,
        launcher_id: Bytes32,
        value_hash: Bytes32,
    ) -> Result<Self, DriverError> {
        Ok(Self {
            coin: Coin::new(
                parent_coin_id,
                Slot::<V>::puzzle_hash(launcher_id, value_hash).into(),
                0,
            ),
            launcher_id,
            value_hash,
            value: None,
        })
    }

    pub fn from_value(parent_coin_id: Bytes32, launcher_id: Bytes32, value: V) -> Self
    where
        V: ToTreeHash,
    {
        let value_hash = value.tree_hash().into();

        Self {
            coin: Coin::new(
                parent_coin_id,
                Slot::<V>::puzzle_hash(launcher_id, value_hash).into(),
                0,
            ),
            launcher_id,
            value_hash,
            value: Some(value),
        }
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
    b38ba57fdab27038c023662fb1e5f86611a3923291226571a68f12d936bb7401
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
#[clvm(solution)]
pub struct SlotSolution {
    pub parent_parent_info: Bytes32,
    pub parent_inner_puzzle_hash: Bytes32,
    pub spender_inner_puzzle_hash: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogSlotNeigborsInfo {
    pub left_asset_id: Bytes32,
    #[clvm(rest)]
    pub right_asset_id: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogSlotValue {
    pub asset_id: Bytes32,
    #[clvm(rest)]
    pub neighbors: CatalogSlotNeigborsInfo,
}

impl CatalogSlotValue {
    pub fn new(asset_id: Bytes32, left_asset_id: Bytes32, right_asset_id: Bytes32) -> Self {
        Self {
            asset_id,
            neighbors: CatalogSlotNeigborsInfo {
                left_asset_id,
                right_asset_id,
            },
        }
    }
}

impl Ord for CatalogSlotValue {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_is_negative = self.asset_id >= Bytes32::from(SLOT32_MIN_VALUE);
        let other_is_negative = other.asset_id >= Bytes32::from(SLOT32_MIN_VALUE);

        if self_is_negative && !other_is_negative {
            return Ordering::Less;
        }

        if !self_is_negative && other_is_negative {
            return Ordering::Greater;
        }

        if self_is_negative {
            return match self.asset_id.cmp(&other.asset_id) {
                Ordering::Less => Ordering::Less,
                Ordering::Equal => Ordering::Equal,
                Ordering::Greater => Ordering::Greater,
            };
        }

        match self.asset_id.cmp(&other.asset_id) {
            Ordering::Less => Ordering::Greater,
            Ordering::Equal => Ordering::Equal,
            Ordering::Greater => Ordering::Less,
        }
    }
}

impl PartialOrd for CatalogSlotValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

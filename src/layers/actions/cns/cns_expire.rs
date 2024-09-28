use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex::encode;
use hex_literal::hex;

use crate::{Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CnsExpireAction {
    pub launcher_id: Bytes32,
}

impl CnsExpireAction {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self { launcher_id }
    }
}

impl Layer for CnsExpireAction {
    type Solution = CnsExpireActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.cns_oracle_puzzle()?,
            args: CnsExpireActionArgs::new(self.launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: CnsExpireActionSolution,
    ) -> Result<NodePtr, DriverError> {
        println!(
            "expire solution: {:?}",
            encode(ctx.serialize(&solution)?.into_bytes())
        );
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }

    fn parse_puzzle(
        _: &clvmr::Allocator,
        _: chia_wallet_sdk::Puzzle,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        unimplemented!()
    }

    fn parse_solution(_: &clvmr::Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        unimplemented!()
    }
}

impl ToTreeHash for CnsExpireAction {
    fn tree_hash(&self) -> TreeHash {
        CnsExpireActionArgs::curry_tree_hash(self.launcher_id)
    }
}

pub const CNS_EXPIRE_PUZZLE: [u8; 43] =
    hex!("ff02ffff01ff04ff0bffff04ffff04ff02ffff04ff8217f7ff808080ff808080ffff04ffff0151ff018080");

pub const CNS_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    2db7823e2996d534dce1bfde71a84589fde4bd41dcffb3f51224bcea8df2124d
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct CnsExpireActionArgs {
    pub slot_1st_curry_hash: Bytes32,
}

impl CnsExpireActionArgs {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self {
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl CnsExpireActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: CNS_EXPIRE_PUZZLE_HASH,
            args: CnsExpireActionArgs::new(launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct CnsExpireActionSolution {
    pub value: Bytes32,
    pub left_value: Bytes32,
    pub left_left_value: Bytes32,
    pub left_rest_hash: Bytes32,
    pub right_value: Bytes32,
    pub right_right_value: Bytes32,
    pub right_rest_hash: Bytes32,
    pub expiration: u64,
    pub data_hash: Bytes32,
}

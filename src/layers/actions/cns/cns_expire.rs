use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
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
            program: ctx.cns_expire_puzzle()?,
            args: CnsExpireActionArgs::new(self.launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: CnsExpireActionSolution,
    ) -> Result<NodePtr, DriverError> {
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

pub const CNS_EXPIRE_PUZZLE: [u8; 909] = hex!("ff02ffff01ff04ff0bffff04ffff04ff10ffff04ff8217f7ff808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff27ffff04ff57ffff04ff8202f7ffff04ffff0bffff0102ffff0bffff0101ff8217f780ff822ff780ff80808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff57ffff04ff81b7ffff04ff27ffff04ff820177ff80808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff8202f7ffff04ff27ffff04ff8205f7ffff04ff820bf7ff80808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff57ffff04ff81b7ffff04ff8202f7ffff04ff820177ff80808080808080ffff04ffff0bffff0101ff5780ff808080808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff8202f7ffff04ff57ffff04ff8205f7ffff04ff820bf7ff80808080808080ffff04ffff0bffff0101ff8202f780ff808080808080ff8080808080808080ffff04ffff01ffffff5133ff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff18ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ff17ff8080808080ff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff178080ff2f8080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const CNS_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    45f28b9fce7646be66c5ed1d9d8ef5f198fa9c54d02a3553c5a33614da3bfe52
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

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

pub const CNS_EXPIRE_PUZZLE: [u8; 1080] = hex!("ff02ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff82015fffff04ff8202dfffff04ff8205dfffff04ff8207dfff80808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff82015fffff04ff8202dfffff04ffff10ff8205dfffff12ffff013cffff013cffff0118ffff0182016effff02ff2cffff04ff02ffff04ffff05ffff14ff819fffff12ff4fffff02ff3affff04ff02ffff04ffff0dff82015f80ff80808080808080ff808080808080ffff04ff8207dfff80808080808080ffff04ffff0bff82015f80ff808080808080ffff04ffff04ff10ffff04ffff0bff05ffff02ff2effff04ff02ffff04ffff04ffff02ff2effff04ff02ffff04ffff04ff82015fff8205df80ff80808080ffff04ffff04ff0bffff04ff819fff808080ff808080ff8080808080ff808080ff8080808080ffff04ffff01ffffff3fff3342ffff02ff02ffff03ff05ffff01ff0bff81fcffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff34ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffff02ffff03ffff15ff05ff8080ffff0105ffff01ff088080ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff28ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ff17ff8080808080ffff0bff81bcffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff34ffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0108ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010180ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff0101ffff01ff012080ff018080ff0180ffff01ff088080ff0180ffffff0bffff0102ffff0bffff0101ffff0bffff0101ff058080ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ff0bff24ffff0bff24ff81dcff0580ffff0bff24ff0bff819c8080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff38ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const CNS_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    4f9afcf92364e16265e6ff8c9ba57115da7f8f0f7b5c920687417211a79ff76a
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

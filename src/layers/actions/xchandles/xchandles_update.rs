use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::{SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH},
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesUpdateAction {
    pub launcher_id: Bytes32,
}

impl XchandlesUpdateAction {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self { launcher_id }
    }
}

impl Layer for XchandlesUpdateAction {
    type Solution = XchandlesUpdateActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_update_puzzle()?,
            args: XchandlesUpdateActionArgs::new(self.launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: XchandlesUpdateActionSolution,
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

impl ToTreeHash for XchandlesUpdateAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesUpdateActionArgs::curry_tree_hash(self.launcher_id)
    }
}

pub const XCHANDLES_UPDATE_PUZZLE: [u8; 778] = hex!("ff02ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff82015fffff04ff8202dfffff04ff8205dfff80808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff17ffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff82015fffff04ff8202dfffff04ff820bdfff80808080808080ff8080808080ffff04ffff04ff18ffff04ffff0112ffff04ffff0bffff0102ff819fffff0bffff0101ff820bdf8080ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ff8205df80ff0b8080ffff04ff820fdfff808080808080ff8080808080ff8080808080ffff04ffff01ffffff3343ff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff10ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bffff0102ff05ffff0bffff0102ff0bffff0bffff0102ffff01a09facad3119a2064f3825e7a244a42f521eafe7b22ed30d2aa4dfce6ab61d7db7ffff0bffff0101ff2f80808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_UPDATE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    57fb21bbd61a62fbbf583e0e637150402f8b626652570d6d20d71b7e36b4baec
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesUpdateActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub singleton_launcher_mod_hash_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesUpdateActionArgs {
    pub fn new(launcher_id: Bytes32) -> Self {
        let singleton_launcher_mod_hash: Bytes32 = SINGLETON_LAUNCHER_PUZZLE_HASH.into();
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            singleton_launcher_mod_hash_hash: singleton_launcher_mod_hash.tree_hash().into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl XchandlesUpdateActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_UPDATE_PUZZLE_HASH,
            args: XchandlesUpdateActionArgs::new(launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesUpdateActionSolution {
    pub value_hash: Bytes32,
    pub neighbors_hash: Bytes32,
    pub expiration: u64,
    pub current_launcher_id: Bytes32,
    pub new_launcher_id: Bytes32,
    #[clvm(rest)]
    pub announcer_inner_puzzle_hash: Bytes32,
}

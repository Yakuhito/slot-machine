use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::offer::SETTLEMENT_PAYMENTS_PUZZLE_HASH,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesExtendAction {
    pub launcher_id: Bytes32,
    pub payout_puzzle_hash: Bytes32,
}

impl XchandlesExtendAction {
    pub fn new(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            payout_puzzle_hash,
        }
    }
}

impl Layer for XchandlesExtendAction {
    type Solution = XchandlesExtendActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_extend_puzzle()?,
            args: XchandlesExtendActionArgs::new(self.launcher_id, self.payout_puzzle_hash),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: XchandlesExtendActionSolution,
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

impl ToTreeHash for XchandlesExtendAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesExtendActionArgs::curry_tree_hash(self.launcher_id, self.payout_puzzle_hash)
    }
}

pub const XCHANDLES_EXTEND_PUZZLE: [u8; 1133] = hex!("ff02ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff82015fffff04ff8202dfffff04ff8205dfffff04ff8207dfff80808080808080ff8080808080ffff04ffff04ff38ffff04ffff0effff0165ffff0bffff0102ffff0bffff0101ff819f80ffff0bffff0101ff82015f808080ff808080ffff04ffff02ff32ffff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff82015fffff04ff8202dfffff04ffff10ff8205dfffff12ffff013cffff013cffff0118ffff0182016effff02ff3cffff04ff02ffff04ffff05ffff14ff819fffff12ff4fffff02ff3affff04ff02ffff04ffff0dff82015f80ff80808080808080ff808080808080ffff04ff8207dfff80808080808080ff8080808080ffff04ffff04ff10ffff04ffff0bff05ffff02ff2effff04ff02ffff04ffff04ffff02ff2effff04ff02ffff04ffff04ff82015fff8205df80ff80808080ffff04ffff04ff0bffff04ff819fff808080ff808080ff8080808080ff808080ff808080808080ffff04ffff01ffffff3fff333effff4202ffff02ffff03ff05ffff01ff0bff81e2ffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181c280ff0180ff02ffff03ffff15ff05ff8080ffff0105ffff01ff088080ff0180ffffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff28ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff81a2ffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0108ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010180ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff01ff0140ffff01ff012080ff018080ff0180ffff01ff088080ff0180ffffff0bffff0102ffff0bffff0101ffff0bffff0101ff058080ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ff0bff34ffff0bff34ff81c2ff0580ffff0bff34ff0bff81828080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_EXTEND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    59abb661c91d82bb246da5111cd3c2c67c38b8c25fe2b26318c093356c6e29dc
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExtendActionArgs {
    pub offer_mod_hash: Bytes32,
    pub payout_puzzle_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesExtendActionArgs {
    pub fn new(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> Self {
        Self {
            offer_mod_hash: SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
            payout_puzzle_hash,
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl XchandlesExtendActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXTEND_PUZZLE_HASH,
            args: XchandlesExtendActionArgs::new(launcher_id, payout_puzzle_hash),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExtendActionSolution {
    pub renew_amount: u64,
    pub name: String,
    pub neighbors_hash: Bytes32,
    pub expiration: u64,
    #[clvm(rest)]
    pub launcher_id_hash: Bytes32,
}

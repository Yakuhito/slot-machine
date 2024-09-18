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
pub struct CnsExtendAction {
    pub launcher_id: Bytes32,
    pub payout_puzzle_hash: Bytes32,
}

impl CnsExtendAction {
    pub fn new(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            payout_puzzle_hash,
        }
    }
}

impl Layer for CnsExtendAction {
    type Solution = CnsExtendActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.cns_extend_puzzle()?,
            args: CnsExtendActionArgs::new(self.launcher_id, self.payout_puzzle_hash),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: CnsExtendActionSolution,
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

impl ToTreeHash for CnsExtendAction {
    fn tree_hash(&self) -> TreeHash {
        CnsExtendActionArgs::curry_tree_hash(self.launcher_id, self.payout_puzzle_hash)
    }
}

pub const CNS_EXTEND_PUZZLE: [u8; 1072] = hex!("ff02ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff82015fffff04ff8202dfffff04ff8205dfffff04ff8207dfff80808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff82015fffff04ff8202dfffff04ffff10ff8205dfffff12ffff013cffff013cffff0118ffff0182016effff02ff2cffff04ff02ffff04ffff05ffff14ff819fffff12ff4fffff02ff3affff04ff02ffff04ff82015fff80808080808080ff808080808080ffff04ff8207dfff80808080808080ffff04ffff0bff82015f80ff808080808080ffff04ffff04ff10ffff04ffff0bff05ffff02ff2effff04ff02ffff04ffff04ffff02ff2effff04ff02ffff04ff82015fffff04ff8205dfff8080808080ffff04ffff04ff0bffff04ff819fff808080ff808080ff8080808080ff808080ff8080808080ffff04ffff01ffffff3fff3342ffff02ff02ffff03ff05ffff01ff0bff81fcffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff34ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffff02ffff03ffff15ff05ff8080ffff0105ffff01ff088080ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff28ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ff17ff8080808080ffff0bff81bcffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff34ffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0108ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010180ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff0101ffff01ff012080ff018080ff0180ffff01ff088080ff0180ffffff0bffff0102ffff0bffff0101ffff0bff058080ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ff0bff24ffff0bff24ff81dcff0580ffff0bff24ff0bff819c8080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff38ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const CNS_EXTEND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    8a03592f79ae976d51bc1238d5236a00d7da88c1c50129c1e6cb8b937ff94ebf
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct CnsExtendActionArgs {
    pub offer_mod_hash: Bytes32,
    pub payout_puzzle_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl CnsExtendActionArgs {
    pub fn new(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> Self {
        Self {
            offer_mod_hash: SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
            payout_puzzle_hash,
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl CnsExtendActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32, payout_puzzle_hash: Bytes32) -> TreeHash {
        CurriedProgram {
            program: CNS_EXTEND_PUZZLE_HASH,
            args: CnsExtendActionArgs::new(launcher_id, payout_puzzle_hash),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct CnsExtendActionSolution {
    pub renew_amount: u64,
    pub name: String,
    pub neighbors_hash: Bytes32,
    pub expiration: u64,
    #[clvm(rest)]
    pub rest_hash: Bytes32,
}
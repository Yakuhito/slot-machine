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

use super::XchandlesFactorPricingSolution;

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
    type Solution =
        XchandlesExtendActionSolution<NodePtr, XchandlesFactorPricingSolution, NodePtr, ()>;

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
        solution: Self::Solution,
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

pub const XCHANDLES_EXTEND_PUZZLE: [u8; 1043] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff4fffff02ff2effff04ff02ffff04ff8205dfff8080808080ffff09ff81afffff02ff2effff04ff02ffff04ff82015fff8080808080ffff09ff8204dfff822fdf80ffff09ff819fffff0bffff0101ff820adf808080ffff01ff04ff2fffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ff8217dfffff04ff822fdfffff04ff823fdfff80808080808080ff8080808080ffff04ffff04ff24ffff04ffff0effff0165ffff0bffff0102ffff0bffff0101ffff05ffff02ff82015fff8202df808080ff819f8080ff808080ffff04ffff04ff10ffff04ff822fdfff808080ffff04ffff02ff2affff04ff02ffff04ff17ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ff8217dfffff04ffff10ff822fdfffff06ffff02ff82015fff8202df808080ffff04ff823fdfff80808080808080ff8080808080ffff04ffff04ff28ffff04ffff0bffff02ff8205dfffff04ff05ffff04ff820bdfff80808080ffff02ff2effff04ff02ffff04ffff04ffff0bffff0102ff819fffff0bffff0101ff822fdf8080ffff04ffff04ff0bffff04ffff05ffff02ff82015fff8202df8080ff808080ff808080ff8080808080ff808080ff80808080808080ffff01ff088080ff0180ffff04ffff01ffffff55ff3f33ffff3e42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff38ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff34ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_EXTEND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    0ee45f0f32b1bcc836cf2063aa71126d6d58ec19c165de12bdc74e0292328f85
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
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, None).into(),
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
pub struct XchandlesExtendActionSolution<PP, PS, CMP, CMS> {
    pub handle_hash: Bytes32,
    pub pricing_puzzle_reveal: PP,
    pub pricing_solution: PS,
    pub cat_maker_puzzle_reveal: CMP,
    pub cat_maker_solution: CMS,
    pub neighbors_hash: Bytes32,
    pub expiration: u64,
    #[clvm(rest)]
    pub rest_hash: Bytes32,
}

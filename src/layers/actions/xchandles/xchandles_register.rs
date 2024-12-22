use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::cat::CAT_PUZZLE_HASH,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{PrecommitLayer, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesRegisterAction {
    pub launcher_id: Bytes32,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
}

impl XchandlesRegisterAction {
    pub fn new(
        launcher_id: Bytes32,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> Self {
        Self {
            launcher_id,
            precommit_payout_puzzle_hash,
            relative_block_height,
        }
    }

    pub fn get_price_factor(handle: &str) -> Option<u64> {
        match handle.len() {
            0..3 => None,
            3 => Some(64),
            4 => Some(32),
            5 => Some(8),
            6..31 => Some(1),
            _ => None,
        }
    }
}

impl Layer for XchandlesRegisterAction {
    type Solution = XchandlesRegisterActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_register_puzzle()?,
            args: XchandlesRegisterActionArgs::new(
                self.launcher_id,
                self.precommit_payout_puzzle_hash,
                self.relative_block_height,
            ),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: XchandlesRegisterActionSolution,
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

impl ToTreeHash for XchandlesRegisterAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesRegisterActionArgs::curry_tree_hash(
            self.launcher_id,
            self.precommit_payout_puzzle_hash,
            self.relative_block_height,
        )
    }
}

pub const XCHANDLES_REGISTER_PUZZLE: [u8; 1640] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff819fffff0bffff0101ff82015f8080ffff15ff819fff8202df80ffff15ff8205dfff819f8080ffff01ff02ff26ffff04ff02ffff04ff05ffff04ff0bffff04ff17ffff04ff2fffff04ff8207dfffff04ff819fffff04ffff0bffff0101ff819f80ffff04ffff0bffff0101ff8202df80ffff04ffff0bffff0101ff8205df80ffff04ffff12ffff02ff32ffff04ff02ffff04ffff02ff2effff04ff02ffff04ff82015fff80808080ff80808080ff6f80ff80808080808080808080808080ffff01ff088080ff0180ffff04ffff01ffffffff5133ff4202ffffff02ffff03ff05ffff01ff0bff81ecffff02ff3affff04ff02ffff04ff09ffff04ffff02ff24ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181cc80ff0180ff02ffff03ffff15ff05ff8080ffff0105ffff01ff088080ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff30ffff04ffff02ff22ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffffffff0bff81acffff02ff3affff04ff02ffff04ff05ffff04ffff02ff24ffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0108ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010180ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff01ff0140ffff01ff012080ff018080ff0180ffff01ff088080ff0180ffff0bffff0102ff05ffff0bffff0102ffff0bffff0102ff0bff1780ff2f8080ff0bff38ffff0bff38ff81ccff0580ffff0bff38ff0bff818c8080ffffff04ff2fffff04ffff04ff20ffff04ff82015fff808080ffff04ffff02ff36ffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8202ffffff04ff820bdfffff04ff8205ffffff04ff8217dfff80808080808080ff8080808080ffff04ffff02ff36ffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8205ffffff04ff8202ffffff04ff822fdfffff04ff825fdfff80808080808080ff8080808080ffff04ffff02ff3cffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff82017fffff04ff8202ffffff04ff8205ffffff04ffff0bffff0102ffff0bffff0101ffff10ff82015fffff12ffff013cffff013cffff0118ffff0182016effff02ff34ffff04ff02ffff04ffff05ffff14ff8205dfff820bff8080ff80808080808080ffff0bffff0101ff819f8080ff80808080808080ff8080808080ffff04ffff02ff3cffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8202ffffff04ff820bdfffff04ff82017fffff04ff8217dfff80808080808080ff8080808080ffff04ffff02ff3cffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8205ffffff04ff82017fffff04ff822fdfffff04ff825fdfff80808080808080ff8080808080ffff04ffff04ff28ffff04ffff0112ffff04ff8205dfffff04ffff02ff22ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0580ffff04ff4fffff04ffff02ff22ffff04ff02ffff04ff0bffff04ffff0bffff0102ffff0bffff0102ff8202dfff81bf80ffff0bffff0102ffff0bffff0101ff819f80ffff0bffff0101ff82015f808080ff8080808080ff80808080808080ff8080808080ff808080808080808080ff04ff28ffff04ffff0112ffff04ff80ffff04ffff02ff22ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ffff02ffff03ff05ffff01ff02ffff03ffff02ff3effff04ff02ffff04ffff0cff05ff80ffff010180ff80808080ffff01ff10ffff0101ffff02ff2effff04ff02ffff04ffff0cff05ffff010180ff8080808080ffff01ff088080ff0180ff8080ff0180ff21ffff22ffff15ff05ffff016080ffff15ffff017bff058080ffff22ffff15ff05ffff012f80ffff15ffff013aff05808080ff018080");

pub const XCHANDLES_REGISTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    57145e0799928ad981e4ece812ade08041b69681675e51d17e4af0ff2aaf9eec
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesRegisterActionArgs {
    pub cat_mod_hash: Bytes32,
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesRegisterActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> Self {
        Self {
            cat_mod_hash: CAT_PUZZLE_HASH.into(),
            precommit_1st_curry_hash: PrecommitLayer::<()>::first_curry_hash(
                launcher_id,
                relative_block_height,
                precommit_payout_puzzle_hash,
            )
            .into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl XchandlesRegisterActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_REGISTER_PUZZLE_HASH,
            args: XchandlesRegisterActionArgs::new(
                launcher_id,
                precommit_payout_puzzle_hash,
                relative_block_height,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesRegisterActionSolution {
    pub handle_hash: Bytes32,
    pub handle_reveal: String,
    pub left_value: Bytes32,
    pub right_value: Bytes32,
    pub handle_nft_launcher_id: Bytes32,
    pub start_time: u64,
    pub secret_hash: Bytes32,
    pub precommitment_amount: u64,
    pub left_left_value_hash: Bytes32,
    pub left_data_hash: Bytes32,
    pub right_right_value_hash: Bytes32,
    pub right_data_hash: Bytes32,
}

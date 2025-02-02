use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::{SingletonStruct, SINGLETON_TOP_LAYER_PUZZLE_HASH},
};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{DigSlotNonce, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigAddMirrorAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
}

impl Layer for DigAddMirrorAction {
    type Solution = DigAddMirrorActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_add_mirror_action_puzzle()?,
            args: DigAddMirrorActionArgs::new(self.launcher_id, self.validator_launcher_id),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigAddMirrorActionSolution,
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

impl DigAddMirrorAction {
    pub fn curry_tree_hash(launcher_id: Bytes32, validator_launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: DIG_ADD_MIRROR_PUZZLE_HASH,
            args: DigAddMirrorActionArgs::new(launcher_id, validator_launcher_id),
        }
        .tree_hash()
    }
}

pub const DIG_ADD_MIRROR_PUZZLE: [u8; 651] = hex!("ff02ffff01ff04ffff04ff4fffff04ffff10ff81afff8201df80ffff04ff82016fffff04ff8202efff8080808080ffff04ffff04ff18ffff04ffff0112ffff04ffff0effff0161ffff0bffff0102ffff0bffff0101ff82015f80ffff0bffff0101ff8201df808080ffff04ffff0bff52ffff0bff14ffff0bff14ff62ff0580ffff0bff14ffff0bff72ffff0bff14ffff0bff14ff62ff0b80ffff0bff14ffff0bff72ffff0bff14ffff0bff14ff62ff819f80ffff0bff14ff62ff42808080ff42808080ff42808080ff8080808080ffff04ffff02ff1affff04ff02ffff04ff17ffff04ffff0bffff0102ffff0bffff0101ff82015f80ffff0bffff0102ffff0bffff0101ff82026f80ffff0bffff0101ff8201df808080ff8080808080ff80808080ffff04ffff01ffffff3343ff02ff02ffff03ff05ffff01ff0bff72ffff02ff1effff04ff02ffff04ff09ffff04ffff02ff1cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff10ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff52ffff02ff1effff04ff02ffff04ff05ffff04ffff02ff1cffff04ff02ffff04ff07ff80808080ff808080808080ff0bff14ffff0bff14ff62ff0580ffff0bff14ff0bff428080ff018080");

pub const DIG_ADD_MIRROR_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    8d2b70a8e78b1fcdd77dea490ebb4604905e08dcd214d09146c0f79da0e245c5
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigAddMirrorActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub validator_singleton_struct_hash: Bytes32,
    pub mirror_slot_1st_curry_hash: Bytes32,
}

impl DigAddMirrorActionArgs {
    pub fn new(launcher_id: Bytes32, validator_launcher_id: Bytes32) -> Self {
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            validator_singleton_struct_hash: SingletonStruct::new(validator_launcher_id)
                .tree_hash()
                .into(),
            mirror_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                Some(DigSlotNonce::MIRROR.to_u64()),
            )
            .into(),
        }
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigAddMirrorActionSolution {
    pub validator_singleton_inner_puzzle_hash: Bytes32,
    pub mirror_payout_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub mirror_shares: u64,
}

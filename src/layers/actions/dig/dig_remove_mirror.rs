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
pub struct DigRemoveMirrorAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
}

impl Layer for DigRemoveMirrorAction {
    type Solution = DigRemoveMirrorActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_remove_mirror_action_puzzle()?,
            args: DigRemoveMirrorActionArgs::new(self.launcher_id, self.validator_launcher_id),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigRemoveMirrorActionSolution,
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

pub const DIG_REMOVE_MIRROR_PUZZLE: [u8; 677] = hex!("ff02ffff01ff04ffff04ff4fffff04ffff11ff81afff8201df80ffff04ff82016fffff04ff8202efff8080808080ffff04ffff04ff18ffff04ffff0112ffff04ffff0effff0172ffff0bffff0102ffff0bffff0101ff82015f80ffff0bffff0101ff8201df808080ffff04ffff0bff5affff0bff1cffff0bff1cff6aff0580ffff0bff1cffff0bff7affff0bff1cffff0bff1cff6aff0b80ffff0bff1cffff0bff7affff0bff1cffff0bff1cff6aff819f80ffff0bff1cff6aff4a808080ff4a808080ff4a808080ff8080808080ffff04ffff04ff10ffff04ffff10ff8204efffff010180ff808080ffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff0bffff0102ffff0bffff0101ff82015f80ffff0bffff0102ffff0bffff0101ff82026f80ffff0bffff0101ff8201df808080ff8080808080ff8080808080ffff04ffff01ffffff5543ff4202ffffff02ffff03ff05ffff01ff0bff7affff02ff2effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016a80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff5affff02ff2effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff1cffff0bff1cff6aff0580ffff0bff1cff0bff4a8080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_REMOVE_MIRROR_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    addf22b86ab52e2fd13a1b5d2a0a9b31ccae2859012b202cae037295025c3f9b
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigRemoveMirrorActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub validator_singleton_struct_hash: Bytes32,
    pub mirror_slot_1st_curry_hash: Bytes32,
}

impl DigRemoveMirrorActionArgs {
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
pub struct DigRemoveMirrorActionSolution {
    pub validator_singleton_inner_puzzle_hash: Bytes32,
    pub mirror_payout_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub mirror_shares: u64,
}

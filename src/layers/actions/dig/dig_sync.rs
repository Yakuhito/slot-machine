use chia::{clvm_utils::TreeHash, protocol::Bytes32};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigSyncAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
}

impl Layer for DigSyncAction {
    type Solution = DigSyncActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        ctx.dig_sync_action_puzzle()
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigSyncActionSolution,
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

impl DigSyncAction {
    pub fn curry_tree_hash() -> TreeHash {
        DIG_SYNC_PUZZLE_HASH
    }
}

pub const DIG_SYNC_PUZZLE: [u8; 270] = hex!("ff02ffff01ff02ffff03ffff22ffff20ffff15ff13ff81dd8080ffff15ff13ff819d8080ffff01ff04ffff04ff09ffff04ff15ffff04ffff02ff0effff04ff02ffff04ff15ffff04ff4dffff04ff6dffff04ffff05ffff14ffff12ff6dffff11ff13ff819d8080ffff12ff15ffff11ff81ddff819d80808080ff80808080808080ffff04ffff04ff13ff81dd80ff8080808080ffff04ffff04ff04ffff04ff13ff808080ffff04ffff04ff0affff04ffff0effff0173ffff0bffff0102ffff0bffff0101ff1380ffff0bffff0101ff81dd808080ff808080ff80808080ffff01ff088080ff0180ffff04ffff01ff51ff3eff04ffff10ff0bff2f80ffff11ff17ffff12ff2fff05808080ff018080");

pub const DIG_SYNC_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    59f43204bc4029631fd3d7deaee02af4c66720788dd24249eb5e0176cd8348cc
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigSyncActionSolution {
    pub update_time: u64,
}

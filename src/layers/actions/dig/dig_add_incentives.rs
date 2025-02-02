use chia::clvm_utils::{ToTreeHash, TreeHash};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigAddIncentivesAction {}

impl Layer for DigAddIncentivesAction {
    type Solution = DigAddIncentivesActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        ctx.dig_add_incentives_action_puzzle()
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigAddIncentivesActionSolution,
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

impl ToTreeHash for DigAddIncentivesAction {
    fn tree_hash(&self) -> TreeHash {
        DIG_ADD_INCENTIVES_PUZZLE_HASH
    }
}

impl DigAddIncentivesAction {
    pub fn curry_tree_hash() -> TreeHash {
        DIG_ADD_INCENTIVES_PUZZLE_HASH
    }
}

pub const DIG_ADD_INCENTIVES_PUZZLE: [u8; 166] = hex!("ff02ffff01ff02ffff03ffff22ffff15ff81ddff819d80ffff15ff13ff808080ffff01ff04ffff04ffff10ff09ff1380ffff04ff15ffff04ffff04ff4dffff10ff6dff138080ffff04ffff04ff819dff81dd80ff8080808080ffff04ffff04ff02ffff04ffff0effff0163ffff0bffff0102ffff0bffff0101ff1380ffff0bffff0101ff81dd808080ff808080ff808080ffff01ff088080ff0180ffff04ffff013eff018080");

pub const DIG_ADD_INCENTIVES_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    54cf9beaff1aa6273c72dfb8d5d5c98b015b2a12e29365479495bd02f44d7fe4
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigAddIncentivesActionSolution {
    pub amount: u64,
}

use chia::clvm_utils::TreeHash;
use chia_wallet_sdk::{DriverError, SpendContext};
use clvmr::{Allocator, NodePtr};

pub trait Action {
    type Solution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>;
    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError>;

    fn puzzle_hash(&self, allocator: &mut Allocator) -> TreeHash;
}

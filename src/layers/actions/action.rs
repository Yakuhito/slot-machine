use chia_wallet_sdk::{DriverError, SpendContext};
use clvmr::NodePtr;

pub trait Action<S> {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>;
    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: S,
    ) -> Result<NodePtr, DriverError>;
}

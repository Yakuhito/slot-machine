use chia::clvm_utils::TreeHash;
use chia_wallet_sdk::{DriverError, SpendContext};
use clvmr::NodePtr;

pub trait Action {
    type Solution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>;
    fn puzzle_hash(&self, ctx: &mut SpendContext) -> TreeHash;
}

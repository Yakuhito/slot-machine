use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvmr::{Allocator, NodePtr};

pub trait Action
where
    Self: ToTreeHash + Layer,
{
    type Registry;
    type RegistryConstants;
    type SlotType;
    type SpendParams;

    fn from_constants(launcher_id: Bytes32, constants: &Self::RegistryConstants) -> Self;

    fn tree_hash(&self) -> TreeHash;
    fn curry_tree_hash(launcher_id: Bytes32, constants: &Self::RegistryConstants) -> TreeHash;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>;
    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError>;

    fn parse_puzzle(_: &Allocator, _: Puzzle) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        unimplemented!()
    }

    fn parse_solution(_: &clvmr::Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        unimplemented!()
    }

    fn get_created_slots(&self, params: &Self::SpendParams) -> Vec<Self::SlotType>;
    fn get_secure_conditions(&self, params: &Self::SpendParams) -> Conditions;
    fn spend(self, params: &Self::SpendParams) -> Result<Spend, DriverError>;
}

use chia::{clvm_utils::TreeHash, protocol::Bytes32};
use chia_wallet_sdk::{Conditions, DriverError, Spend, SpendContext};
use clvmr::NodePtr;

pub trait Action {
    type Registry;
    type RegistryState;
    type RegistryConstants;
    type SlotValueType;
    type Solution;
    type SpendParams;
    type SpendReturnParams;

    fn from_constants(launcher_id: Bytes32, constants: &Self::RegistryConstants) -> Self;

    fn curry_tree_hash(launcher_id: Bytes32, constants: &Self::RegistryConstants) -> TreeHash;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError>;

    fn get_created_slot_values(
        &self,
        state: &Self::RegistryState,
        params: &Self::Solution,
    ) -> Vec<Self::SlotValueType>;

    fn spend(
        self,
        ctx: &mut SpendContext,
        registry: &Self::Registry,
        params: &Self::SpendParams,
    ) -> Result<(Option<Conditions>, Spend, Self::SpendReturnParams), DriverError>;
}

use chia::{clvm_utils::TreeHash, protocol::Bytes32, puzzles::singleton::SingletonStruct};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSchedulerLayer {
    pub other_singleton_launcher_id: Bytes32,
    pub new_state_hash: Bytes32,
    pub required_block_height: u32,
    pub new_puzzle_hash: Bytes32,
}

impl StateSchedulerLayer {
    pub fn new(
        other_singleton_launcher_id: Bytes32,
        new_state_hash: Bytes32,
        required_block_height: u32,
        new_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            other_singleton_launcher_id,
            new_state_hash,
            required_block_height,
            new_puzzle_hash,
        }
    }
}

impl Layer for StateSchedulerLayer {
    type Solution = StateSchedulerLayerSolution;

    fn parse_puzzle(_: &Allocator, _: Puzzle) -> Result<Option<Self>, DriverError> {
        todo!()
    }

    fn parse_solution(_: &Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        todo!()
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        todo!()
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        Ok(solution.to_clvm(&mut ctx.allocator)?)
    }
}

pub const STATE_SCHEDULER_PUZZLE: [u8; 503] = hex!("ff02ffff01ff04ffff04ff18ffff04ff2fffff01ff01808080ffff04ffff04ff10ffff04ff17ff808080ffff04ffff04ff14ffff04ffff0112ffff04ff0bffff04ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff3effff04ff02ffff04ff05ff80808080ffff04ff5fff808080808080ff8080808080ff80808080ffff04ffff01ffffff5333ff4202ffffff02ffff03ff05ffff01ff0bff7affff02ff2effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016a80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff5affff02ff2effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff1cffff0bff1cff6aff0580ffff0bff1cff0bff4a8080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff3effff04ff02ffff04ff09ff80808080ffff02ff3effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const STATE_SCHEDULER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    f081173cc82c6940a0c0a9f35b7ae5e75ff7befa431ac97f216af94328b9a8be
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct StateSchedulerLayerArgs {
    pub other_singleton_struct: SingletonStruct,
    pub new_state_hash: Bytes32,
    pub required_block_height: u32,
    pub new_puzzle_hash: Bytes32,
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct StateSchedulerLayerSolution {
    pub other_singleton_inner_puzzle_hash: Bytes32,
}

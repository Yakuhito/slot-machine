use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::{
        SingletonStruct, SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH,
    },
};
use chia_wallet_sdk::{Condition, Conditions, DriverError, Layer, Puzzle, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

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

    fn parse_puzzle(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(puzzle) = puzzle.as_curried() else {
            return Ok(None);
        };

        if puzzle.mod_hash != STATE_SCHEDULER_PUZZLE_HASH {
            return Ok(None);
        }

        let args = StateSchedulerLayerArgs::<NodePtr>::from_clvm(allocator, puzzle.args)?;

        if args.other_singleton_struct.launcher_puzzle_hash != SINGLETON_LAUNCHER_PUZZLE_HASH.into()
            || args.other_singleton_struct.mod_hash != SINGLETON_TOP_LAYER_PUZZLE_HASH.into()
        {
            return Err(DriverError::NonStandardLayer);
        }

        let conditions = Conditions::<NodePtr>::from_clvm(allocator, args.base_conditions)?;
        let (
            Some(Condition::AssertHeightAbsolute(assert_height_condition)),
            Some(Condition::CreateCoin(create_coin_condition)),
        ) = conditions
            .into_iter()
            .fold(
                (None, None),
                |(assert_height, create_coin), cond| match cond {
                    Condition::AssertHeightAbsolute(_) if assert_height.is_none() => {
                        (Some(cond), create_coin)
                    }
                    Condition::CreateCoin(_) if create_coin.is_none() => {
                        (assert_height, Some(cond))
                    }
                    _ => (assert_height, create_coin),
                },
            )
        else {
            return Err(DriverError::NonStandardLayer);
        };

        Ok(Some(Self {
            other_singleton_launcher_id: args.other_singleton_struct.launcher_id,
            new_state_hash: args.new_state_hash,
            required_block_height: assert_height_condition.height,
            new_puzzle_hash: create_coin_condition.puzzle_hash,
        }))
    }

    fn parse_solution(
        allocator: &Allocator,
        solution: NodePtr,
    ) -> Result<Self::Solution, DriverError> {
        StateSchedulerLayerSolution::from_clvm(allocator, solution).map_err(DriverError::FromClvm)
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let base_conditions = Conditions::new()
            .create_coin(self.new_puzzle_hash, 1, vec![])
            .assert_height_absolute(self.required_block_height);

        CurriedProgram {
            program: ctx.state_shcheduler_puzzle()?,
            args: StateSchedulerLayerArgs {
                other_singleton_struct: SingletonStruct::new(self.other_singleton_launcher_id),
                new_state_hash: self.new_state_hash,
                base_conditions,
            },
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        Ok(solution.to_clvm(&mut ctx.allocator)?)
    }
}

pub const STATE_SCHEDULER_PUZZLE: [u8; 375] = hex!("ff02ffff01ff04ffff04ff08ffff04ffff0112ffff04ff17ffff04ffff0bff2affff0bff0cffff0bff0cff32ff0980ffff0bff0cffff0bff3affff0bff0cffff0bff0cff32ffff02ff0effff04ff02ffff04ff05ff8080808080ffff0bff0cffff0bff3affff0bff0cffff0bff0cff32ff2f80ffff0bff0cff32ff22808080ff22808080ff22808080ff8080808080ff0b80ffff04ffff01ffff4202ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff0effff04ff02ffff04ff09ff80808080ffff02ff0effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const STATE_SCHEDULER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e871966a3b575cdd720d4d831bfb9e5dd2b6390999fa9524d2ca60b88bc0b54
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(curry)]
pub struct StateSchedulerLayerArgs<T> {
    pub other_singleton_struct: SingletonStruct,
    pub base_conditions: T,
    pub new_state_hash: Bytes32,
}

impl<T> StateSchedulerLayerArgs<T>
where
    T: ToTreeHash,
{
    pub fn curry_tree_hash(
        other_singleton_launcher_id: Bytes32,
        base_conditions: T,
        new_state_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: STATE_SCHEDULER_PUZZLE_HASH,
            args: StateSchedulerLayerArgs {
                other_singleton_struct: SingletonStruct::new(other_singleton_launcher_id),
                base_conditions: base_conditions.tree_hash(),
                new_state_hash,
            },
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct StateSchedulerLayerSolution {
    pub other_singleton_inner_puzzle_hash: Bytes32,
}

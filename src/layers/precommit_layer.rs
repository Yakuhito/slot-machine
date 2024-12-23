use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SINGLETON_TOP_LAYER_PUZZLE_HASH,
};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone)]
#[must_use]
pub struct PrecommitLayer<V> {
    pub controller_singleton_struct_hash: Bytes32,
    pub relative_block_height: u32,
    pub refund_puzzle_hash: Bytes32,
    pub value: V,
}

impl<V> PrecommitLayer<V> {
    pub fn new(
        controller_singleton_struct_hash: Bytes32,
        relative_block_height: u32,
        refund_puzzle_hash: Bytes32,
        value: V,
    ) -> Self {
        Self {
            controller_singleton_struct_hash,
            relative_block_height,
            refund_puzzle_hash,
            value,
        }
    }

    pub fn first_curry_hash(
        controller_singleton_struct_hash: Bytes32,
        relative_block_height: u32,
    ) -> TreeHash {
        CurriedProgram {
            program: PRECOMMIT_LAYER_PUZZLE_HASH,
            args: PrecommitLayer1stCurryArgs {
                singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
                singleton_struct_hash: controller_singleton_struct_hash,
                relative_block_height,
            },
        }
        .tree_hash()
    }

    pub fn puzzle_hash(
        controller_singleton_struct_hash: Bytes32,
        relative_block_height: u32,
        refund_puzzle_hash: Bytes32,
        value_hash: TreeHash,
    ) -> TreeHash {
        CurriedProgram {
            program: Self::first_curry_hash(
                controller_singleton_struct_hash,
                relative_block_height,
            ),
            args: PrecommitLayer2ndCurryArgs {
                refund_puzzle_hash,
                value: value_hash,
            },
        }
        .tree_hash()
    }
}

impl<V> Layer for PrecommitLayer<V>
where
    V: ToClvm<Allocator> + FromClvm<Allocator> + Clone,
{
    type Solution = PrecommitLayerSolution;

    fn parse_puzzle(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(puzzle_2nd_curry) = puzzle.as_curried() else {
            return Ok(None);
        };

        let Some(curried) = CurriedProgram::<NodePtr, NodePtr>::parse_puzzle(allocator, puzzle)?
        else {
            return Ok(None);
        };
        let puzzle_1st_curry = Puzzle::parse(allocator, curried.program);
        let Some(puzzle_1st_curry) = puzzle_1st_curry.as_curried() else {
            return Ok(None);
        };

        if puzzle_1st_curry.mod_hash != PRECOMMIT_LAYER_PUZZLE_HASH {
            return Ok(None);
        }

        let args_2nd_curry =
            PrecommitLayer2ndCurryArgs::<V>::from_clvm(allocator, puzzle_2nd_curry.args)?;
        let args_1st_curry =
            PrecommitLayer1stCurryArgs::from_clvm(allocator, puzzle_1st_curry.args)?;

        Ok(Some(Self {
            controller_singleton_struct_hash: args_1st_curry.singleton_struct_hash,
            relative_block_height: args_1st_curry.relative_block_height,
            refund_puzzle_hash: args_2nd_curry.refund_puzzle_hash,
            value: args_2nd_curry.value,
        }))
    }

    fn parse_solution(
        allocator: &Allocator,
        solution: NodePtr,
    ) -> Result<Self::Solution, DriverError> {
        PrecommitLayerSolution::from_clvm(allocator, solution).map_err(DriverError::FromClvm)
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let prog_1st_curry = CurriedProgram {
            program: ctx.precommit_layer_puzzle()?,
            args: PrecommitLayer1stCurryArgs {
                singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
                singleton_struct_hash: self.controller_singleton_struct_hash,
                relative_block_height: self.relative_block_height,
            },
        }
        .to_clvm(&mut ctx.allocator)?;

        Ok(CurriedProgram {
            program: prog_1st_curry,
            args: PrecommitLayer2ndCurryArgs {
                refund_puzzle_hash: self.refund_puzzle_hash,
                value: self.value.clone(),
            },
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }
}

pub const PRECOMMIT_LAYER_PUZZLE: [u8; 450] = hex!("ff02ffff01ff04ffff04ff10ffff04ff17ff808080ffff04ffff04ff18ffff04ff82017fff808080ffff04ffff04ff14ffff04ff81bfffff04ff82017fffff04ffff04ff81bfff8080ff8080808080ffff04ffff04ff1cffff04ffff0113ffff04ff81bfffff04ffff02ff2effff04ff02ffff04ff05ffff04ff0bffff04ff8202ffff808080808080ff8080808080ff8080808080ffff04ffff01ffffff5249ff3343ffff02ff02ffff03ff05ffff01ff0bff76ffff02ff3effff04ff02ffff04ff09ffff04ffff02ff1affff04ff02ffff04ff0dff80808080ff808080808080ffff016680ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff56ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff1affff04ff02ffff04ff07ff80808080ff808080808080ff0bff12ffff0bff12ff66ff0580ffff0bff12ff0bff468080ff018080");

pub const PRECOMMIT_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    bf7daa5e86e400003235efcd3bf46a7bdaaa39c1518bae410843447e100b7ea9
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PrecommitLayer1stCurryArgs {
    pub singleton_mod_hash: Bytes32,
    pub singleton_struct_hash: Bytes32,
    pub relative_block_height: u32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PrecommitLayer2ndCurryArgs<V> {
    pub refund_puzzle_hash: Bytes32,
    pub value: V,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(solution)]
pub struct PrecommitLayerSolution {
    pub target_puzzle_hash: Bytes32,
    pub my_amount: u64,
    pub singleton_inner_puzzle_hash: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrecommitValue<T = NodePtr> {
    pub initial_inner_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub tail_reveal: T,
}

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesSecretAndHandle {
    pub secret: Bytes32,
    #[clvm(rest)]
    pub handle: String,
}

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct XchandlesPrecommitValue {
    pub secret_and_handle: XchandlesSecretAndHandle,
    pub handle_nft_launcher_id: Bytes32,
    #[clvm(rest)]
    pub start_time: u64,
}

impl XchandlesPrecommitValue {
    #[allow(dead_code)]
    pub fn new(
        secret: Bytes32,
        handle: String,
        handle_nft_launcher_id: Bytes32,
        start_time: u64,
    ) -> Self {
        Self {
            secret_and_handle: XchandlesSecretAndHandle { secret, handle },
            handle_nft_launcher_id,
            start_time,
        }
    }
}

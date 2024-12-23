use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone)]
#[must_use]
pub struct PrecommitLayer<V> {
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub refund_puzzle_hash: Bytes32,
    pub value: V,
}

impl<V> PrecommitLayer<V> {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        refund_puzzle_hash: Bytes32,
        value: V,
    ) -> Self {
        Self {
            launcher_id,
            relative_block_height,
            refund_puzzle_hash,
            value,
        }
    }

    pub fn first_curry_hash(launcher_id: Bytes32, relative_block_height: u32) -> TreeHash {
        CurriedProgram {
            program: PRECOMMIT_LAYER_PUZZLE_HASH,
            args: PrecommitLayer1stCurryArgs {
                singleton_struct: SingletonStruct::new(launcher_id),
                relative_block_height,
            },
        }
        .tree_hash()
    }

    pub fn puzzle_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        refund_puzzle_hash: Bytes32,
        value_hash: TreeHash,
    ) -> TreeHash {
        CurriedProgram {
            program: Self::first_curry_hash(launcher_id, relative_block_height),
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
            launcher_id: args_1st_curry.singleton_struct.launcher_id,
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
                singleton_struct: SingletonStruct::new(self.launcher_id),
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

pub const PRECOMMIT_LAYER_PUZZLE: [u8; 546] = hex!("ff02ffff01ff04ffff04ff10ffff04ff0bff808080ffff04ffff04ff18ffff04ff82017fff808080ffff04ffff04ff14ffff04ff81bfffff04ff82017fffff04ffff04ff81bfff8080ff8080808080ffff04ffff04ff2cffff04ffff0113ffff04ff81bfffff04ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff3effff04ff02ffff04ff05ff80808080ffff04ff8202ffff808080808080ff8080808080ff8080808080ffff04ffff01ffffff5249ff33ff4302ffffff02ffff03ff05ffff01ff0bff7affff02ff2effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016a80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff5affff02ff2effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff3cffff0bff3cff6aff0580ffff0bff3cff0bff4a8080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff3effff04ff02ffff04ff09ff80808080ffff02ff3effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const PRECOMMIT_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    258563f603a6fe6ab2f8866c798dfaf85c38c063fd2fff2d7b901dd918681b92
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PrecommitLayer1stCurryArgs {
    pub singleton_struct: SingletonStruct,
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

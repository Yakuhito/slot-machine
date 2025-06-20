use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes, Bytes32},
};
use chia_puzzles::SINGLETON_TOP_LAYER_V1_1_HASH;
use chia_wallet_sdk::{
    driver::{DriverError, Layer, Puzzle, SpendContext},
    types::Conditions,
};
use clvm_traits::{clvm_quote, clvm_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::{CatNftMetadata, DefaultCatMakerArgs, SpendContextExt};

#[derive(Debug, Clone)]
#[must_use]
pub struct PrecommitLayer<V> {
    pub controller_singleton_struct_hash: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
    pub refund_puzzle_hash: Bytes32,
    pub value: V,
}

impl<V> PrecommitLayer<V> {
    pub fn new(
        controller_singleton_struct_hash: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
        refund_puzzle_hash: Bytes32,
        value: V,
    ) -> Self {
        Self {
            controller_singleton_struct_hash,
            relative_block_height,
            payout_puzzle_hash,
            refund_puzzle_hash,
            value,
        }
    }

    pub fn first_curry_hash(
        controller_singleton_struct_hash: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: PRECOMMIT_LAYER_PUZZLE_HASH,
            args: PrecommitLayer1stCurryArgs {
                singleton_mod_hash: SINGLETON_TOP_LAYER_V1_1_HASH.into(),
                singleton_struct_hash: controller_singleton_struct_hash,
                relative_block_height,
                payout_puzzle_hash,
            },
        }
        .tree_hash()
    }

    pub fn puzzle_hash(
        controller_singleton_struct_hash: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
        refund_puzzle_hash: Bytes32,
        value_hash: TreeHash,
    ) -> TreeHash {
        CurriedProgram {
            program: Self::first_curry_hash(
                controller_singleton_struct_hash,
                relative_block_height,
                payout_puzzle_hash,
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
            payout_puzzle_hash: args_1st_curry.payout_puzzle_hash,
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
                singleton_mod_hash: SINGLETON_TOP_LAYER_V1_1_HASH.into(),
                singleton_struct_hash: self.controller_singleton_struct_hash,
                relative_block_height: self.relative_block_height,
                payout_puzzle_hash: self.payout_puzzle_hash,
            },
        }
        .to_clvm(ctx)?;

        Ok(CurriedProgram {
            program: prog_1st_curry,
            args: PrecommitLayer2ndCurryArgs {
                refund_puzzle_hash: self.refund_puzzle_hash,
                value: self.value.clone(),
            },
        }
        .to_clvm(ctx)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        ctx.alloc(&solution)
    }
}

pub const PRECOMMIT_LAYER_PUZZLE: [u8; 469] = hex!("ff02ffff01ff04ffff04ff10ffff04ff17ff808080ffff04ffff04ff18ffff04ff8202ffff808080ffff04ffff04ff14ffff04ffff03ff82017fff2fff5f80ffff04ff8202ffffff04ffff04ffff03ff82017fff2fff5f80ff8080ff8080808080ffff04ffff04ff1cffff04ffff0113ffff04ff82017fffff04ffff02ff2effff04ff02ffff04ff05ffff04ff0bffff04ff8205ffff808080808080ff8080808080ff8080808080ffff04ffff01ffffff5249ff3343ffff02ff02ffff03ff05ffff01ff0bff76ffff02ff3effff04ff02ffff04ff09ffff04ffff02ff1affff04ff02ffff04ff0dff80808080ff808080808080ffff016680ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff56ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff1affff04ff02ffff04ff07ff80808080ff808080808080ff0bff12ffff0bff12ff66ff0580ffff0bff12ff0bff468080ff018080");

pub const PRECOMMIT_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    10efe1dab105ef4780345baa2442196a26944040b12c0167375d79aaec89e33f
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PrecommitLayer1stCurryArgs {
    pub singleton_mod_hash: Bytes32,
    pub singleton_struct_hash: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
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
    pub mode: u8,
    pub my_amount: u64,
    pub singleton_inner_puzzle_hash: Bytes32,
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrecommitValue<T = NodePtr, S = ()> {
    pub tail_reveal: T,
    pub initial_inner_puzzle_hash: Bytes32,
    pub cat_maker_hash: Bytes32,
    #[clvm(rest)]
    pub cat_maker_solution: S,
}

impl<T> CatalogPrecommitValue<T> {
    pub fn with_default_cat_maker(
        payment_asset_tail_hash_hash: TreeHash,
        initial_inner_puzzle_hash: Bytes32,
        tail_reveal: T,
    ) -> Self {
        Self {
            tail_reveal,
            initial_inner_puzzle_hash,
            cat_maker_hash: DefaultCatMakerArgs::curry_tree_hash(
                payment_asset_tail_hash_hash.into(),
            )
            .into(),
            cat_maker_solution: (),
        }
    }

    pub fn initial_inner_puzzle(
        ctx: &mut SpendContext,
        owner_inner_puzzle_hash: Bytes32,
        initial_metadata: CatNftMetadata,
    ) -> Result<NodePtr, DriverError> {
        let mut conds = Conditions::new().create_coin(
            owner_inner_puzzle_hash,
            1,
            Some(ctx.hint(owner_inner_puzzle_hash)?),
        );
        let updater_solution = ctx.alloc(&initial_metadata)?;
        conds = conds.update_nft_metadata(ctx.any_metadata_updater()?, updater_solution);
        conds = conds.remark(ctx.alloc(&"MEOW".to_string())?);

        ctx.alloc(&clvm_quote!(conds))
    }
}

// value is:
// (c
//   (c (c cat_maker_reveal cat_maker_solution) (c pricing_puzzle_reveal pricing_solution))
//   (c (c secret handle) (c start_time (c new_owner_launcher_id new_resolved_data))))
// )
#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(transparent)]
pub struct XchandlesPrecommitValue {
    pub value: Bytes32,
}

impl XchandlesPrecommitValue {
    pub fn new(value: Bytes32) -> Self {
        Self { value }
    }

    pub fn from_data<CP, CS, PP, PS, S>(
        cat_maker_reveal: CP,
        cat_maker_solution: CS,
        pricing_puzzle_reveal: PP,
        pricing_solution: PS,
        secret: S,
        handle: String,
        start_time: u64,
        owner_launcher_id: Bytes32,
        resolved_data: Bytes,
    ) -> Self
    where
        CP: ToTreeHash,
        CS: ToTreeHash,
        PP: ToTreeHash,
        PS: ToTreeHash,
        S: ToTreeHash,
    {
        Self::new(
            clvm_tuple!(
                clvm_tuple!(
                    clvm_tuple!(cat_maker_reveal.tree_hash(), cat_maker_solution.tree_hash()),
                    clvm_tuple!(
                        pricing_puzzle_reveal.tree_hash(),
                        pricing_solution.tree_hash()
                    )
                ),
                clvm_tuple!(
                    clvm_tuple!(secret.tree_hash(), handle),
                    clvm_tuple!(start_time, clvm_tuple!(owner_launcher_id, resolved_data))
                )
            )
            .tree_hash()
            .into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn for_normal_registration<PS>(
        payment_tail_hash_hash: TreeHash,
        pricing_puzzle_hash: TreeHash,
        pricing_puzzle_solution: PS,
        secret: Bytes32,
        handle: String,
        start_time: u64,
        owner_launcher_id: Bytes32,
        resolved_data: Bytes,
    ) -> Self
    where
        PS: ToTreeHash,
    {
        Self::from_data(
            DefaultCatMakerArgs::curry_tree_hash(payment_tail_hash_hash.into()),
            (),
            pricing_puzzle_hash,
            pricing_puzzle_solution.tree_hash(),
            secret,
            handle,
            start_time,
            owner_launcher_id,
            resolved_data,
        )
    }
}

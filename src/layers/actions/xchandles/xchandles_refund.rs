use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{PrecommitLayer, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesRefundAction {
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
}

impl XchandlesRefundAction {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            relative_block_height,
            payout_puzzle_hash,
        }
    }
}

impl Layer for XchandlesRefundAction {
    type Solution = XchandlesRefundActionSolution<NodePtr, (), NodePtr, NodePtr>;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_refund_puzzle()?,
            args: XchandlesRefundActionArgs::new(
                self.launcher_id,
                self.relative_block_height,
                self.payout_puzzle_hash,
            ),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }

    fn parse_puzzle(
        _: &clvmr::Allocator,
        _: chia_wallet_sdk::Puzzle,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        unimplemented!()
    }

    fn parse_solution(_: &clvmr::Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        unimplemented!()
    }
}

impl ToTreeHash for XchandlesRefundAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesRefundActionArgs::curry_tree_hash(
            self.launcher_id,
            self.relative_block_height,
            self.payout_puzzle_hash,
        )
    }
}

pub const XCHANDLES_REFUND_PUZZLE: [u8; 1136] =
    hex!("ff02ffff01ff02ffff03ffff22ffff09ff82016fffff02ff2effff04ff02ffff04ff81afff8080808080ffff09ff820befffff02ff2effff04ff02ffff04ff8205efff8080808080ffff15ff83017fefff8080ffff010180ffff01ff04ff17ffff02ff16ffff04ff02ffff04ff0bffff04ffff0bffff0102ffff0bffff0101ff4f80ff8301ffef80ffff04ffff22ffff09ff82016fff2780ffff21ffff02ffff03ffff09ff820befff5780ffff01ff09ff4fffff0bffff0101ff8227ef8080ff8080ff0180ffff02ffff03ffff09ff820befff7780ffff01ff09ff4fffff0bffff0101ff82b7ef8080ff8080ff018080ffff09ff83017fefffff05ffff02ff8205efff8217ef80808080ffff04ffff04ffff04ff28ffff04ffff0effff0124ffff0bffff0102ffff0bffff0102ff822fefff4f80ff825fef8080ff808080ffff04ffff04ff38ffff04ffff0113ffff04ff80ffff04ffff02ff81afffff04ffff02ff2affff04ff02ffff04ff05ffff04ff82bfefffff04ffff0bffff0102ffff0bffff0101ffff0bffff0102ffff0bffff0102ff82016fffff02ff2effff04ff02ffff04ff8202efff8080808080ffff0bffff0102ff820befffff02ff2effff04ff02ffff04ff8217efff80808080808080ffff0bffff0102ffff0bffff0102ff822fefff4f80ff825fef8080ff808080808080ffff04ff8202efff80808080ffff04ff83017fefff808080808080ff808080ff8080808080808080ffff01ff088080ff0180ffff04ffff01ffffff33ff3e42ff02ffff02ffff03ff05ffff01ff0bff81fcffff02ff3affff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff10ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff81bcffff02ff3affff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ff0bff14ffff0bff14ff81dcff0580ffff0bff14ff0bff819c8080ffff02ffff03ff17ffff01ff04ffff02ff3effff04ff02ffff04ff05ffff04ff0bff8080808080ffff04ffff02ff12ffff04ff02ffff04ff05ffff04ff0bff8080808080ff2f8080ffff012f80ff0180ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff38ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_REFUND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    ac92130aa248b45ed9266b50e968be1abbb0e8f5374c0a58fca459459e639adc
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesRefundActionArgs {
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesRefundActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            precommit_1st_curry_hash: PrecommitLayer::<()>::first_curry_hash(
                SingletonStruct::new(launcher_id).tree_hash().into(),
                relative_block_height,
                payout_puzzle_hash,
            )
            .into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, None).into(),
        }
    }
}

impl XchandlesRefundActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_REFUND_PUZZLE_HASH,
            args: XchandlesRefundActionArgs::new(
                launcher_id,
                relative_block_height,
                payout_puzzle_hash,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesRefundActionSolution<CMP, CMS, PP, PS> {
    pub handle_hash: Bytes32,
    pub precommited_cat_maker_reveal: CMP,
    pub precommited_cat_maker_hash: Bytes32,
    pub precommited_cat_maker_solution: CMS,
    pub precommited_pricing_puzzle_reveal: PP,
    pub precommited_pricing_puzzle_hash: Bytes32,
    pub precommited_pricing_puzzle_solution: PS,
    pub secret_hash: Bytes32,
    pub precommit_value_rest_hash: Bytes32,
    pub refund_puzzle_hash_hash: Bytes32,
    pub precommit_amount: u64,
    #[clvm(rest)]
    pub rest_hash: Bytes32,
}

use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{CatalogRegistryInfo, CatalogSlotValue, PrecommitLayer, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRefundAction {
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
}

impl CatalogRefundAction {
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

    pub fn from_info(info: &CatalogRegistryInfo) -> Self {
        Self {
            launcher_id: info.launcher_id,
            relative_block_height: info.constants.relative_block_height,
            payout_puzzle_hash: info.constants.precommit_payout_puzzle_hash,
        }
    }
}

impl Layer for CatalogRefundAction {
    type Solution = CatalogRefundActionSolution<NodePtr, ()>;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.catalog_refund_action_puzzle()?,
            args: CatalogRefundActionArgs::new(
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
        solution: CatalogRefundActionSolution<NodePtr, ()>,
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

impl ToTreeHash for CatalogRefundAction {
    fn tree_hash(&self) -> TreeHash {
        CatalogRefundActionArgs::curry_tree_hash(
            self.launcher_id,
            self.relative_block_height,
            self.payout_puzzle_hash,
        )
    }
}

pub const CATALOG_REFUND_PUZZLE: [u8; 981] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff81afffff02ff2effff04ff02ffff04ff4fff8080808080ffff15ff8217efff808080ffff01ff04ff17ffff02ff36ffff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff8202efffff04ff821fefff8080808080ffff04ffff22ffff09ff37ff8217ef80ffff09ff81afff278080ffff04ffff04ffff04ff28ffff04ffff0effff0124ffff0bffff0102ffff0bffff0101ff8202ef80ffff0bffff0101ff8205ef808080ff808080ffff04ffff04ff38ffff04ffff0113ffff04ff80ffff04ffff02ff4fffff04ffff02ff2affff04ff02ffff04ff05ffff04ff820befffff04ffff0bffff0102ffff0bffff0101ffff0bffff0102ff81afffff02ff2effff04ff02ffff04ff82016fff808080808080ffff0bffff0102ffff0bffff0101ff8205ef80ff8202ef8080ff808080808080ffff04ff82016fff80808080ffff04ff8217efff808080808080ff808080ff8080808080808080ffff01ff088080ff0180ffff04ffff01ffffff33ff3e42ff02ffff02ffff03ff05ffff01ff0bff81fcffff02ff26ffff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff10ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff81bcffff02ff26ffff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ff0bffff0102ffff0bffff0101ff0580ff0b80ffffff0bff14ffff0bff14ff81dcff0580ffff0bff14ff0bff819c8080ff02ffff03ff17ffff01ff04ffff02ff3effff04ff02ffff04ff05ffff04ff0bff8080808080ffff04ffff02ff12ffff04ff02ffff04ff05ffff04ff0bff8080808080ff2f8080ffff012f80ff0180ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff38ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const CATALOG_REFUND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    8c8d0622c4bc95eb8d5be2ee16353569f04bd411d02aa4b1efa34dfe606d54c0
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct CatalogRefundActionArgs {
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl CatalogRefundActionArgs {
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
            slot_1st_curry_hash: Slot::<CatalogSlotValue>::first_curry_hash(launcher_id, None)
                .into(),
        }
    }
}

impl CatalogRefundActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: CATALOG_REFUND_PUZZLE_HASH,
            args: CatalogRefundActionArgs::new(
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
pub struct CatalogRefundActionSolution<P, S> {
    pub precommited_cat_maker_reveal: P,
    pub precommited_cat_maker_hash: Bytes32,
    pub precommited_cat_maker_solution: S,
    pub tail_hash: Bytes32,
    pub initial_nft_owner_ph: Bytes32,
    pub refund_puzzle_hash_hash: Bytes32,
    pub precommit_amount: u64,
    #[clvm(rest)]
    pub neighbors_hash: Bytes32,
}

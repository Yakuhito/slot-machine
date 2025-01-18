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

pub const CATALOG_REFUND_PUZZLE: [u8; 849] = hex!("ff02ffff01ff02ffff03ffff22ffff09ffff0bffff0102ff82016fffff02ff2effff04ff02ffff04ff8202efff8080808080ff4f80ffff09ff82016fffff02ff2effff04ff02ffff04ff81afff808080808080ffff01ff04ff17ffff04ffff04ff10ffff04ffff0effff0124ffff0bffff0102ffff0bffff0101ff8205ef80ffff0bffff0101ff820bef808080ff808080ffff04ffff02ffff03ffff22ffff09ff37ff822fef80ffff09ff82016fff278080ffff01ff02ff3effff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff8205efffff04ff823fefff8080808080ff8080808080ffff01ff04ff18ff808080ff0180ffff04ffff04ff14ffff04ffff0113ffff04ff80ffff04ffff02ff81afffff04ffff02ff2affff04ff02ffff04ff05ffff04ff8217efffff04ffff0bffff0101ff4f80ffff04ffff0bffff0102ffff0bffff0101ff820bef80ff8205ef80ff80808080808080ffff04ff8202efff80808080ffff04ff822fefff808080808080ff8080808080ffff01ff088080ff0180ffff04ffff01ffffff3e01ff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff52ffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ff0bffff0102ffff0bffff0101ff0580ff0b80ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const CATALOG_REFUND_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1a29ccc68f9971356b415ee4d3039a60592963ff54210b2e4dbb6fe072dfbc1d
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
            slot_1st_curry_hash: Slot::<CatalogSlotValue>::first_curry_hash(launcher_id).into(),
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
    pub refund_info_hash: Bytes32,
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

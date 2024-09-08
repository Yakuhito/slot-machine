use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::{
        nft::{
            NFT_OWNERSHIP_LAYER_PUZZLE_HASH, NFT_ROYALTY_TRANSFER_PUZZLE_HASH,
            NFT_STATE_LAYER_PUZZLE_HASH,
        },
        singleton::{SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH},
    },
};
use chia_wallet_sdk::DriverError;
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{CatalogSlotValue, PrecommitCoin, Slot, SpendContextExt, UniquenessPrelauncher};

use super::Action;

pub struct CatalogRegisterAction {
    pub launcher_id: Bytes32,
    pub royalty_puzzle_hash_hash: Bytes32,
    pub trade_price_percentage: u16,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
}

impl CatalogRegisterAction {
    pub fn new(
        launcher_id: Bytes32,
        royalty_puzzle_hash_hash: Bytes32,
        trade_price_percentage: u16,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> Self {
        Self {
            launcher_id,
            royalty_puzzle_hash_hash,
            trade_price_percentage,
            precommit_payout_puzzle_hash,
            relative_block_height,
        }
    }
}

impl Action for CatalogRegisterAction {
    type Solution = CatalogRegisterActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.delegated_state_action_puzzle()?,
            args: CatalogRegisterActionArgs::new(
                self.launcher_id,
                self.royalty_puzzle_hash_hash,
                self.trade_price_percentage,
                self.precommit_payout_puzzle_hash,
                self.relative_block_height,
            ),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: CatalogRegisterActionSolution,
    ) -> Result<NodePtr, DriverError> {
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }
}

impl ToTreeHash for CatalogRegisterAction {
    fn tree_hash(&self) -> TreeHash {
        CatalogRegisterActionArgs::curry_tree_hash(
            self.launcher_id,
            self.royalty_puzzle_hash_hash,
            self.trade_price_percentage,
            self.precommit_payout_puzzle_hash,
            self.relative_block_height,
        )
    }
}

pub const ANY_METADATA_UPDATER: [u8; 23] = hex!("ff04ffff04ff0bffff04ff05ff808080ffff01ff808080");

pub const ANY_METADATA_UPDATER_HASH: TreeHash = TreeHash::new(hex!(
    "
    9f28d55242a3bd2b3661c38ba8647392c26bb86594050ea6d33aad1725ca3eea
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct NftPack {
    pub launcher_hash: Bytes32,
    pub singleton_mod_hash: Bytes32,
    pub state_layer_mod_hash: Bytes32,
    pub metadata_updater_hash_hash: Bytes32,
    pub nft_ownership_layer_mod_hash: Bytes32,
    pub transfer_program_mod_hash: Bytes32,
    pub royalty_puzzle_hash_hash: Bytes32,
    pub trade_price_percentage: u16,
}

impl NftPack {
    pub fn new(royalty_puzzle_hash_hash: Bytes32, trade_price_percentage: u16) -> Self {
        let meta_updater_hash: Bytes32 = ANY_METADATA_UPDATER_HASH.into();

        Self {
            launcher_hash: SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            state_layer_mod_hash: NFT_STATE_LAYER_PUZZLE_HASH.into(),
            metadata_updater_hash_hash: meta_updater_hash.tree_hash().into(),
            nft_ownership_layer_mod_hash: NFT_OWNERSHIP_LAYER_PUZZLE_HASH.into(),
            transfer_program_mod_hash: NFT_ROYALTY_TRANSFER_PUZZLE_HASH.into(),
            royalty_puzzle_hash_hash,
            trade_price_percentage,
        }
    }
}

pub const CATALOG_REGISTER_PUZZLE: [u8; 1388] = hex!("ff02ffff01ff02ffff03ffff22ffff15ff82013fff8205bf80ffff15ff8217bfff82013f8080ffff01ff04ff5fffff02ff2affff04ff02ffff04ff05ffff04ff825fbfffff04ff8202bfffff04ffff02ff26ffff04ff02ffff04ff0bffff04ffff0bffff0101ff82013f80ff8080808080ffff04ffff04ffff04ff28ffff04ff825fbfff808080ffff04ffff02ff3effff04ff02ffff04ff2fffff04ffff04ff8205bfffff04ff820bbfff8217bf8080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff2fffff04ffff04ff8217bfffff04ff8205bfff822fbf8080ff8080808080ffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff04ff82013fffff04ff8205bfff8217bf8080ff8080808080ffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff04ff8205bfffff04ff820bbfff82013f8080ff8080808080ffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff04ff8217bfffff04ff82013fff822fbf8080ff8080808080ffff04ffff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff26ffff04ff02ffff04ff17ffff04ffff0bffff0102ffff0bffff0101ff8202bf80ff82013f80ffff04ffff0bffff0101ff819f80ff808080808080ff8080808080ff8080808080808080ff808080808080808080ffff01ff088080ff0180ffff04ffff01ffffff3dff4633ffff4202ffff04ff10ffff04ff0bffff04ffff02ff2effff04ff02ffff04ffff04ffff02ff26ffff04ff02ffff04ff15ffff04ffff02ff2effff04ff02ffff04ffff04ff15ffff04ff0bff098080ff80808080ffff04ffff02ff26ffff04ff02ffff04ff2dffff04ffff0bffff0101ff2d80ffff04ff42ffff04ff5dffff04ffff02ff26ffff04ff02ffff04ff81bdffff04ffff0bffff0101ff81bd80ffff04ff42ffff04ffff02ff26ffff04ff02ffff04ff82017dffff04ffff02ff2effff04ff02ffff04ffff04ff15ffff04ff0bff098080ff80808080ffff04ff8202fdffff04ffff0bffff0101ff8205fd80ff80808080808080ffff04ff17ff8080808080808080ff8080808080808080ff808080808080ffff01ff01ff808080ff80808080ff80808080ff02ffff03ff05ffff01ff0bff72ffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ffff04ff38ffff04ff2fffff01ff80808080ffff04ffff02ff2cffff04ff02ffff04ffff30ffff30ff0bff2fff8080ff09ffff010180ffff04ff17ff8080808080ff5f8080ff04ff38ffff04ffff02ff26ffff04ff02ffff04ff05ffff04ffff0bffff0101ffff02ff2effff04ff02ffff04ff0bff8080808080ff8080808080ffff04ff80ffff04ff13ff8080808080ffffff0bff52ffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ff0bff34ffff0bff34ff62ff0580ffff0bff34ff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff26ffff04ff02ffff04ff05ffff04ffff0bffff0101ffff02ff2effff04ff02ffff04ff0bff8080808080ff8080808080ff8080808080ff018080");

pub const CATALOG_REGISTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    4b10b2183f1d6faac42f1208fcdf1ea5539a8a2de76e1e8fd48e4279d461432b
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct CatalogRegisterActionArgs {
    pub nft_pack: NftPack,
    pub uniqueness_prelauncher_1st_curry_hash: Bytes32,
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl CatalogRegisterActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        royalty_address: Bytes32,
        trade_price_percentage: u16,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> Self {
        Self {
            nft_pack: NftPack::new(royalty_address.tree_hash().into(), trade_price_percentage),
            uniqueness_prelauncher_1st_curry_hash: UniquenessPrelauncher::<()>::first_curry_hash()
                .into(),
            precommit_1st_curry_hash: PrecommitCoin::<()>::first_curry_hash(
                launcher_id,
                relative_block_height,
                precommit_payout_puzzle_hash,
            )
            .into(),
            slot_1st_curry_hash: Slot::<CatalogSlotValue>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl CatalogRegisterActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        royalty_puzzle_hash_hash: Bytes32,
        trade_price_percentage: u16,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> TreeHash {
        CurriedProgram {
            program: CATALOG_REGISTER_PUZZLE_HASH,
            args: CatalogRegisterActionArgs::new(
                launcher_id,
                royalty_puzzle_hash_hash,
                trade_price_percentage,
                precommit_payout_puzzle_hash,
                relative_block_height,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct CatalogRegisterActionSolution {
    pub tail_hash: Bytes32,
    pub initial_nft_owner_ph: Bytes32,
    pub left_tail_hash: Bytes32,
    pub left_left_tail_hash: Bytes32,
    pub right_tail_hash: Bytes32,
    pub right_right_tail_hash: Bytes32,
    pub my_id: Bytes32,
}

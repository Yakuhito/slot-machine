use chia::{clvm_utils::{CurriedProgram, ToTreeHash, TreeHash}, protocol::Bytes32, puzzles::{nft::{NFT_OWNERSHIP_LAYER_PUZZLE_HASH, NFT_ROYALTY_TRANSFER_PUZZLE_HASH, NFT_STATE_LAYER_PUZZLE_HASH}, singleton::{SingletonStruct, SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH}}};
use chia_wallet_sdk::DriverError;
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::layers::SpendContextExt;

use super::Action;

/*
register:
    NFT_PACK ; see 'assert_launcher_ann' to see what this contains
     |- as follows:
    (
        LAUNCHER_HASH
        SINGLETON_MOD_HASH
        STATE_LAYER_MOD_HASH
        METADATA_UPDATER_HASH_HASH
        NFT_OWNERSHIP_LAYER_MOD_HASH
        TRANSFER_PROGRAM_MOD_HASH
        ROYALTY_ADDRESS_HASH    <-- ! Custom !
        TRADE_PRICE_PERCENTAGE  <-- ! Custom !
    ) ; NFT_PACK

    UNIQUENESS_PRELAUNCHER_1ST_CURRY_HASH ; after 1st curry
     |- curry with launcher hash

    PRECOMMIT_1ST_CURRY_HASH ; after 1st curry
     |- SINGLETON_STRUCT <-- ! Depends on launcher id !
     |- RELATIVE_BLOCK_HEIGHT ; 32
     |- PRECOMMIT_PAYOUT_ADDRESS <-- ! Custom !

    SLOT_1ST_CURRY_HASH ; after 1st curry
     |- depends on launcher id
*/

pub struct CatalogRegisterAction {
    pub launcher_id: Bytes32,
    pub royalty_address: Bytes32,
    pub trade_price_percentage: u8, 
    pub precommit_payout_puzzle_hash: Bytes32, 
}

impl CatalogRegisterAction {
    pub fn new(launcher_id: Bytes32, royalty_address: Bytes32, trade_price_percentage: u8, precommit_payout_puzzle_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            royalty_address,
            trade_price_percentage,
            precommit_payout_puzzle_hash,
        }
    }
}

impl<S> Action<DelegatedStateActionSolution<S>> for DelegatedStateAction where S: ToClvm<Allocator> {
    fn construct_puzzle(&self, ctx: &mut chia_wallet_sdk::SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.delegated_state_action_puzzle()?,
            args: DelegatedStateActionArgs::new(self.other_launcher_id),
        }.to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DelegatedStateActionSolution<S>,
    ) -> Result<NodePtr, DriverError> {
        solution.to_clvm(&mut ctx.allocator).map_err(DriverError::ToClvm)
    }
}

pub const ANY_METADATA_UPDATER: [u8; 23] = hex!("ff04ffff04ff0bffff04ff05ff808080ffff01ff808080");

pub const  ANY_METADATA_UPDATER_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e5759069429397243b808748e5bd5270ea0891953ea06df9a46b87ce4ade466
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct NftPack {
    pub launcher_hash: Bytes32,
    pub singleton_mod_hash: Bytes32,
    pub state_layer_mod_hash: Bytes32,
    pub metadata_updater_hash_hash: Bytes32,
    pub nft_ownership_layer_mod_hash: Bytes32,
    pub transfer_program_mod_hash: Bytes32,
    pub royalty_address_hash: Bytes32,
    pub trade_price_percentage: u8,
}

impl NftPack {
    pub fn new(
        royalty_address_hash: Bytes32,
        trade_price_percentage: u8,
    ) -> Self {
        Self {
            launcher_hash: SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            state_layer_mod_hash: NFT_STATE_LAYER_PUZZLE_HASH.into(),
            metadata_updater_hash_hash: ANY_METADATA_UPDATER_HASH.into(),
            nft_ownership_layer_mod_hash: NFT_OWNERSHIP_LAYER_PUZZLE_HASH.into(),
            transfer_program_mod_hash: NFT_ROYALTY_TRANSFER_PUZZLE_HASH.into(),
            royalty_address_hash,
            trade_price_percentage,
        }
    }
}

pub const CATALOG_REGISTER_PUZZLE: [u8; 1457] = hex!("ff02ffff01ff02ffff03ffff22ffff15ff82013fff820bbf80ffff15ff822fbfff82013f8080ffff01ff04ff5fffff02ff2affff04ff02ffff04ff05ffff04ff82bfbfffff04ff8202bfffff04ff8205bfffff04ffff02ff26ffff04ff02ffff04ff0bffff04ffff0bffff0101ff82013f80ff8080808080ffff04ffff04ffff04ff28ffff04ff82bfbfff808080ffff04ffff02ff3effff04ff02ffff04ff2fffff04ffff04ff820bbfffff04ff8217bfff822fbf8080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff2fffff04ffff04ff822fbfffff04ff820bbfff825fbf8080ff8080808080ffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff04ff82013fffff04ff820bbfff822fbf8080ff8080808080ffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff04ff820bbfffff04ff8217bfff82013f8080ff8080808080ffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff04ff822fbfffff04ff82013fff825fbf8080ff8080808080ffff04ffff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff26ffff04ff02ffff04ff17ffff04ffff0bffff0102ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff8205bf8080ff82013f80ffff04ffff0bffff0101ff819f80ff808080808080ff8080808080ff8080808080808080ff80808080808080808080ffff01ff088080ff0180ffff04ffff01ffffff3dff4633ffff4202ffff04ff10ffff04ff0bffff04ffff02ff2effff04ff02ffff04ffff04ffff02ff26ffff04ff02ffff04ff15ffff04ffff02ff2effff04ff02ffff04ffff04ff15ffff04ff0bff098080ff80808080ffff04ffff02ff26ffff04ff02ffff04ff2dffff04ffff0bffff0101ff2d80ffff04ff2fffff04ff5dffff04ffff02ff26ffff04ff02ffff04ff81bdffff04ffff0bffff0101ff81bd80ffff04ffff01a04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459affff04ffff02ff26ffff04ff02ffff04ff82017dffff04ffff02ff2effff04ff02ffff04ffff04ff15ffff04ff0bff098080ff80808080ffff04ff8202fdffff04ffff0bffff0101ff8205fd80ff80808080808080ffff04ff17ff8080808080808080ff8080808080808080ff808080808080ffff01ff01ff808080ff80808080ff80808080ff02ffff03ff05ffff01ff0bff72ffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ffff04ff38ffff04ff5fffff01ff80808080ffff04ffff02ff2cffff04ff02ffff04ffff30ffff30ff0bff5fff8080ff09ffff010180ffff04ff17ffff04ff2fff808080808080ff81bf8080ff04ff38ffff04ffff02ff26ffff04ff02ffff04ff05ffff04ffff0bffff0101ffff02ff2effff04ff02ffff04ff0bff8080808080ff8080808080ffff04ff80ffff04ff13ff8080808080ffffff0bff52ffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ff0bff34ffff0bff34ff62ff0580ffff0bff34ff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff26ffff04ff02ffff04ff05ffff04ffff0bffff0101ffff02ff2effff04ff02ffff04ff0bff8080808080ff8080808080ff8080808080ff018080");

pub const  CATALOG_REGISTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e5759069429397243b808748e5bd5270ea0891953ea06df9a46b87ce4ade466
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
    pub fn new(royalty_address_hash: Bytes32, trade_price_percentage: u8) -> Self {
        Self {
            nft_pack: NftPack::new(royalty_address_hash, trade_price_percentage),
            uniqueness_prelauncher_1st_curry_hash: todo,
            precommit_1st_curry_hash: todo,
            precommit_1st_curry_hash: todo,
        }
    }
}

impl DelegatedStateActionArgs { 
    pub fn curry_tree_hash(
        other_launcher_id: Bytes32
    ) -> TreeHash {
        CurriedProgram {
            program: DELEGATED_STATE_ACTION_PUZZLE_HASH,
            args: DelegatedStateActionArgs::new(other_launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct DelegatedStateActionSolution<S> {
    pub new_state: S,
    pub other_singleton_inner_puzzle_hash: Bytes32,
}

/*
pub struct CatalogRegisterAction {
    pub tail_hash: Bytes32,
    pub initial_nft_owner_ph: Bytes32,
    pub initial_nft_metadata_hash: Bytes32,
    pub left_tail_hash: Bytes32,
    pub left_left_tail_hash: Bytes32,
    pub right_tail_hash: Bytes32,
    pub right_right_tail_hash: Bytes32,
    pub my_id: Bytes32,
}

*/
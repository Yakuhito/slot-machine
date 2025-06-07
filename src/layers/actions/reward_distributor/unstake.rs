use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_puzzle_types::{
    nft::NftOwnershipLayerArgs,
    offer::{NotarizedPayment, Payment},
    singleton::SingletonArgs,
};
use chia_puzzles::{
    NFT_OWNERSHIP_LAYER_HASH, NFT_STATE_LAYER_HASH, SETTLEMENT_PAYMENT_HASH,
    SINGLETON_LAUNCHER_HASH, SINGLETON_TOP_LAYER_V1_1_HASH,
};
use chia_wallet_sdk::{
    driver::{DriverError, HashedPtr, Nft, NftStateLayer, SingletonLayer, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
use clvm_traits::{clvm_list, clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, P2DelegatedBySingletonLayerArgs, RewardDistributor, RewardDistributorConstants,
    RewardDistributorEntrySlotValue, RewardDistributorSlotNonce, RewardDistributorState, Slot,
    SpendContextExt, NONCE_WRAPPER_PUZZLE_HASH,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardDistributorUnstakeAction {
    pub launcher_id: Bytes32,
    pub max_second_offset: u64,
}

impl ToTreeHash for RewardDistributorUnstakeAction {
    fn tree_hash(&self) -> TreeHash {
        RewardDistributorUnstakeActionArgs::curry_tree_hash(
            self.launcher_id,
            self.max_second_offset,
        )
    }
}

impl Action<RewardDistributor> for RewardDistributorUnstakeAction {
    fn from_constants(constants: &RewardDistributorConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
            max_second_offset: constants.max_seconds_offset,
        }
    }
}

impl RewardDistributorUnstakeAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.reward_distributor_unstake_action_puzzle()?,
            args: RewardDistributorUnstakeActionArgs::new(self.launcher_id, self.max_second_offset),
        }
        .to_clvm(ctx)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_spent_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        solution: NodePtr,
    ) -> Result<(RewardDistributorSlotNonce, Bytes32), DriverError> {
        let solution = ctx.extract::<RewardDistributorUnstakeActionSolution>(solution)?;

        Ok((
            RewardDistributorSlotNonce::ENTRY,
            RewardDistributorEntrySlotValue {
                payout_puzzle_hash: solution.entry_custody_puzzle_hash,
                initial_cumulative_payout: solution.entry_initial_cumulative_payout,
                shares: 1,
            }
            .tree_hash()
            .into(),
        ))
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut RewardDistributor,
        entry_slot: Slot<RewardDistributorEntrySlotValue>,
        nft_launcher_id: Bytes32,
        nft_metadata_hash: Bytes32,
        nft_metadata_updater_hash_hash: Bytes32,
        nft_transfer_porgram_hash: Bytes32,
        locked_nft: Nft<HashedPtr>,
    ) -> Result<(Conditions, u64), DriverError> {
        // u64 = last payment amount

        // compute message that the custody puzzle needs to send
        let unstake_message: Bytes32 = nft_launcher_id;
        let mut unstake_message: Vec<u8> = unstake_message.to_vec();

        let remove_entry_conditions = Conditions::new()
            .send_message(
                18,
                unstake_message.into(),
                vec![ctx.alloc(&distributor.coin.puzzle_hash)?],
            )
            .assert_concurrent_puzzle(entry_slot.coin.puzzle_hash);

        // spend entry slot
        entry_slot.spend(ctx, distributor.info.inner_puzzle_hash().into())?;

        // spend NFT
        locked_nft.spend()?;

        // spend self
        let my_state = distributor.get_latest_pending_state(ctx)?;
        let entry_payout_amount = entry_slot.info.value.shares
            * (my_state.round_reward_info.cumulative_payout
                - entry_slot.info.value.initial_cumulative_payout);
        let action_solution = ctx.alloc(&RewardDistributorUnstakeActionSolution {
            nft_launcher_id,
            nft_metadata_hash,
            nft_metadata_updater_hash_hash,
            nft_transfer_porgram_hash,
            entry_initial_cumulative_payout: entry_slot.info.value.initial_cumulative_payout,
            entry_custody_puzzle_hash: entry_slot.info.value.payout_puzzle_hash,
        })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok((remove_entry_conditions, entry_payout_amount))
    }
}

pub const REWARD_DISTRIBUTOR_UNSTAKE_PUZZLE: [u8; 1114] = hex!("ff02ffff01ff04ffff04ff8209ffffff04ffff11ff8215ffffff11ff829dffff83017bff8080ffff04ffff11ff822dffffff010180ff823dff808080ffff04ffff04ff2cffff04ffff0117ffff04ffff02ff2effff04ff02ffff04ffff04ffff0101ffff04ffff04ff18ffff04ff8301fbffffff04ffff0101ffff04ffff04ff8301fbffff8080ff8080808080ff808080ff80808080ffff04ffff30ffff018d6e66745f706172656e745f6964ffff02ff3affff04ff02ffff04ff05ffff04ffff02ff2effff04ff02ffff04ffff04ff05ffff04ff8213ffff0b8080ff80808080ffff04ffff02ff3affff04ff02ffff04ff17ffff04ffff0bffff0101ff1780ffff04ff822bffffff04ff825bffffff04ffff02ff3affff04ff02ffff04ff2fffff04ffff0bffff0101ff2f80ffff04ffff01a04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459affff04ff82bbffffff04ffff02ff3affff04ff02ffff04ff5fffff04ffff0bffff0101ff8301fbff80ffff04ff81bfff808080808080ff8080808080808080ff8080808080808080ff808080808080ffff010180ff8080808080ffff04ffff04ff14ffff04ffff0112ffff04ff8213ffffff04ff8301fbffff8080808080ffff04ffff04ff10ffff04ffff10ff83013dffff8202ff80ff808080ffff04ffff02ff3effff04ff02ffff04ff82017fffff04ffff0bffff0102ffff0bffff0101ff8301fbff80ffff0bffff0102ffff0bffff0101ff83017bff80ffff01a09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b28080ff8080808080ffff04ffff04ffff0181d6ffff04ff18ffff04ff8301fbffffff04ffff11ff829dffff83017bff80ffff04ffff04ff8301fbffff8080ff808080808080ff80808080808080ffff04ffff01ffffff5533ff43ff4202ffffff02ffff03ff05ffff01ff0bff81eaffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181ca80ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff0bff81aaffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff3cffff0bff3cff81caff0580ffff0bff3cff0bff818a8080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff2cffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const REWARD_DISTRIBUTOR_UNSTAKE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    ed7e9d2409642a0bed6c0f33a17102dc75d2a09e7417288c045df608291b0c1c
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct RewardDistributorUnstakeActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub singleton_launcher_hash: Bytes32,
    pub nft_state_layer_mod_hash: Bytes32,
    pub nft_ownership_layer_mod_hash: Bytes32,
    pub nonce_mod_hash: Bytes32,
    pub my_p2_puzzle_hash: Bytes32,
    pub entry_slot_1st_curry_hash: Bytes32,
    pub max_second_offset: u64,
}

impl RewardDistributorUnstakeActionArgs {
    pub fn new(launcher_id: Bytes32, max_second_offset: u64) -> Self {
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_V1_1_HASH.into(),
            singleton_launcher_hash: SINGLETON_LAUNCHER_HASH.into(),
            nft_state_layer_mod_hash: NFT_STATE_LAYER_HASH.into(),
            nft_ownership_layer_mod_hash: NFT_OWNERSHIP_LAYER_HASH.into(),
            nonce_mod_hash: NONCE_WRAPPER_PUZZLE_HASH.into(),
            my_p2_puzzle_hash: Self::my_p2_puzzle_hash(launcher_id),
            entry_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                RewardDistributorSlotNonce::ENTRY.to_u64(),
            )
            .into(),
            max_second_offset,
        }
    }

    pub fn my_p2_puzzle_hash(launcher_id: Bytes32) -> Bytes32 {
        P2DelegatedBySingletonLayerArgs::curry_tree_hash(
            SingletonStruct::new(launcher_id).tree_hash().into(),
            1,
        )
        .into()
    }
}

impl RewardDistributorUnstakeActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32, max_second_offset: u64) -> TreeHash {
        CurriedProgram {
            program: REWARD_DISTRIBUTOR_UNSTAKE_PUZZLE_HASH,
            args: RewardDistributorUnstakeActionArgs::new(launcher_id, max_second_offset),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct RewardDistributorUnstakeActionSolution {
    pub nft_launcher_id: Bytes32,
    pub nft_metadata_hash: Bytes32,
    pub nft_metadata_updater_hash_hash: Bytes32,
    pub nft_transfer_porgram_hash: Bytes32,
    pub entry_initial_cumulative_payout: u64,
    #[clvm(rest)]
    pub entry_custody_puzzle_hash: Bytes32,
}

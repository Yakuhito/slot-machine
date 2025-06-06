use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_puzzle_types::LineageProof;
use chia_puzzles::{
    NFT_OWNERSHIP_LAYER_HASH, NFT_STATE_LAYER_HASH, SETTLEMENT_PAYMENT_HASH,
    SINGLETON_TOP_LAYER_V1_1_HASH,
};
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::Conditions,
};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, P2DelegatedBySingletonLayerArgs, RewardDistributor, RewardDistributorConstants,
    RewardDistributorEntrySlotValue, RewardDistributorSlotNonce, RewardDistributorState, Slot,
    SpendContextExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardDistributorAddEntryAction {
    pub launcher_id: Bytes32,
    pub manager_launcher_id: Bytes32,
    pub max_second_offset: u64,
}

impl ToTreeHash for RewardDistributorAddEntryAction {
    fn tree_hash(&self) -> TreeHash {
        RewardDistributorAddEntryActionArgs::curry_tree_hash(
            self.launcher_id,
            self.manager_launcher_id,
            self.max_second_offset,
        )
    }
}

impl Action<RewardDistributor> for RewardDistributorAddEntryAction {
    fn from_constants(constants: &RewardDistributorConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
            manager_launcher_id: constants.manager_launcher_id,
            max_second_offset: constants.max_seconds_offset,
        }
    }
}

impl RewardDistributorAddEntryAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.reward_distributor_add_entry_action_puzzle()?,
            args: RewardDistributorAddEntryActionArgs::new(
                self.launcher_id,
                self.manager_launcher_id,
                self.max_second_offset,
            ),
        }
        .to_clvm(ctx)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        state: &RewardDistributorState,
        solution: NodePtr,
    ) -> Result<RewardDistributorEntrySlotValue, DriverError> {
        let solution = ctx.extract::<RewardDistributorAddEntryActionSolution>(solution)?;

        Ok(RewardDistributorEntrySlotValue {
            payout_puzzle_hash: solution.entry_payout_puzzle_hash,
            initial_cumulative_payout: state.round_reward_info.cumulative_payout,
            shares: solution.entry_shares,
        })
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut RewardDistributor,
        payout_puzzle_hash: Bytes32,
        shares: u64,
        manager_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, Slot<RewardDistributorEntrySlotValue>), DriverError> {
        // calculate message that the manager needs to send
        let add_entry_message: Bytes32 = clvm_tuple!(payout_puzzle_hash, shares).tree_hash().into();
        let mut add_entry_message: Vec<u8> = add_entry_message.to_vec();
        add_entry_message.insert(0, b'a');
        let add_entry_message = Conditions::new().send_message(
            18,
            add_entry_message.into(),
            vec![ctx.alloc(&distributor.coin.puzzle_hash)?],
        );

        // spend self
        let action_solution = ctx.alloc(&RewardDistributorAddEntryActionSolution {
            manager_singleton_inner_puzzle_hash,
            entry_payout_puzzle_hash: payout_puzzle_hash,
            entry_shares: shares,
        })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        let my_state = distributor.get_latest_pending_state(ctx)?;
        let slot_value = self.get_slot_value_from_solution(ctx, &my_state, action_solution)?;
        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok((
            add_entry_message,
            distributor
                .created_slot_values_to_slots(vec![slot_value], RewardDistributorSlotNonce::ENTRY)
                [0],
        ))
    }
}

pub const REWARD_DISTRIBUTOR_STAKE_PUZZLE: [u8; 1241] = hex!("ff02ffff01ff04ffff04ffff10ff8209ffffff010180ffff04ff8215ffffff04ffff10ff822dffffff010180ffff04ff825dffffff04ff82bdffff808080808080ffff02ff3cffff04ff02ffff04ffff0bffff02ff3affff04ff02ffff04ff09ffff04ffff02ff3effff04ff02ffff04ffff04ff09ffff04ffff02ff36ffff04ff02ffff04ffff30ff83047bffffff02ff3affff04ff02ffff04ff09ffff04ffff02ff3effff04ff02ffff04ff05ff80808080ffff04ff830a7bffff808080808080ff830e7bff80ffff04ff83037bffff8080808080ff1d8080ff80808080ffff04ffff02ff3affff04ff02ffff04ff0bffff04ffff0bffff0101ff0b80ffff04ff822bffffff04ff825bffffff04ffff02ff3affff04ff02ffff04ff17ffff04ffff0bffff0101ff1780ffff04ffff01a04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459affff04ff82bbffffff04ff2fff8080808080808080ff8080808080808080ff808080808080ffff02ff3effff04ff02ffff04ffff04ffff02ff3effff04ff02ffff04ffff04ff8209ffff8213ff80ff80808080ffff04ffff02ff2effff04ff02ffff04ffff02ff3affff04ff02ffff04ff5fffff04ffff0bffff0101ff8301fbff80ffff04ff81bfff808080808080ff80808080ff808080ff8080808080ffff04ffff04ffff04ff28ffff04ff8213ffff808080ffff04ffff02ff2affff04ff02ffff04ff82017fffff04ffff0bffff0102ffff0bffff0101ff8301fbff80ffff0bffff0102ffff0bffff0101ff829dff80ffff01a09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b28080ffff04ff8301fbffff808080808080ffff04ffff04ff10ffff04ffff10ff83013dffff8202ff80ff808080ff80808080ff808080808080ffff04ffff01ffffff55ff463fffff333eff02ff04ffff04ff38ffff04ff05ff808080ffff04ffff04ff34ffff04ff05ff808080ff0b8080ffffffff02ffff03ff05ffff01ff0bff81f2ffff02ff26ffff04ff02ffff04ff09ffff04ffff02ff22ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181d280ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff24ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff17ff8080ff8080808080ff0bff81b2ffff02ff26ffff04ff02ffff04ff05ffff04ffff02ff22ffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bff2cffff0bff2cff81d2ff0580ffff0bff2cff0bff81928080ff02ffff03ff0bffff01ff30ffff02ff36ffff04ff02ffff04ff05ffff04ff1bff8080808080ff23ff3380ffff010580ff0180ffff04ff05ffff04ffff0101ffff04ffff04ff05ff8080ff80808080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff3effff04ff02ffff04ff09ff80808080ffff02ff3effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const REWARD_DISTRIBUTOR_STAKE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    20b8bbfef174cb1c631f4c272962b846ca8973193a3ccffacbb98b0485b34034
    "
));

// run '(mod (NONCE INNER_PUZZLE inner_solution) (a INNER_PUZZLE inner_solution))' -d
pub const NONCE_WRAPPER_PUZZLE: [u8; 7] = hex!("ff02ff05ff0b80");
pub const NONCE_WRAPPER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "5fd999d5030b5cbdaf0095d9d4e56c683fdb847c4b79dd01aa85f577a96e4027"
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct RewardDistributorStakeActionArgs {
    pub did_singleton_struct: SingletonStruct,
    pub nft_state_layer_mod_hash: Bytes32,
    pub nft_ownership_layer_mod_hash: Bytes32,
    pub offer_mod_hash: Bytes32,
    pub nonce_mod_hash: Bytes32,
    pub my_p2_puzzle_hash: Bytes32,
    pub entry_slot_1st_curry_hash: Bytes32,
    pub max_second_offset: u64,
}

impl RewardDistributorStakeActionArgs {
    pub fn new(launcher_id: Bytes32, did_launcher_id: Bytes32, max_second_offset: u64) -> Self {
        Self {
            did_singleton_struct: SingletonStruct::new(did_launcher_id),
            nft_state_layer_mod_hash: NFT_STATE_LAYER_HASH.into(),
            nft_ownership_layer_mod_hash: NFT_OWNERSHIP_LAYER_HASH.into(),
            offer_mod_hash: SETTLEMENT_PAYMENT_HASH.into(),
            nonce_mod_hash: NONCE_WRAPPER_PUZZLE_HASH.into(),
            my_p2_puzzle_hash: P2DelegatedBySingletonLayerArgs::curry_tree_hash(
                SingletonStruct::new(launcher_id).tree_hash().into(),
                1,
            )
            .into(),
            entry_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                RewardDistributorSlotNonce::ENTRY.to_u64(),
            )
            .into(),
            max_second_offset,
        }
    }
}

impl RewardDistributorStakeActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        did_launcher_id: Bytes32,
        max_second_offset: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: REWARD_DISTRIBUTOR_STAKE_PUZZLE_HASH,
            args: RewardDistributorStakeActionArgs::new(
                launcher_id,
                did_launcher_id,
                max_second_offset,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct IntermediaryCoinProof {
    pub full_puzzle_hash: Bytes32,
    pub amount: u64,
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct NftLauncherProof {
    pub did_proof: LineageProof,
    #[clvm(rest)]
    pub intermediary_coin_proofs: Vec<IntermediaryCoinProof>,
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct RewardDistributorStakeActionSolution {
    pub my_id: Bytes32,
    pub nft_metadata_hash: Bytes32,
    pub nft_metadata_updater_hash_hash: Bytes32,
    pub nft_transfer_porgram_hash: Bytes32,
    pub nft_launcher_proof: NftLauncherProof,
    #[clvm(rest)]
    pub entry_custody_puzzle_hash: u64,
}

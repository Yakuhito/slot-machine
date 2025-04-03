use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_puzzles::SINGLETON_TOP_LAYER_V1_1_HASH;
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DigMirrorSlotValue, DigRewardDistributor, DigRewardDistributorConstants,
    DigRewardDistributorState, DigSlotNonce, Slot, SpendContextExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigInitiatePayoutAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
    pub payout_threshold: u64,
}

impl ToTreeHash for DigInitiatePayoutAction {
    fn tree_hash(&self) -> TreeHash {
        DigInitiatePayoutAction::curry_tree_hash(
            self.launcher_id,
            self.validator_launcher_id,
            self.payout_threshold,
        )
    }
}

impl Action<DigRewardDistributor> for DigInitiatePayoutAction {
    fn from_constants(constants: &DigRewardDistributorConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
            validator_launcher_id: constants.validator_launcher_id,
            payout_threshold: constants.payout_threshold,
        }
    }
}

impl DigInitiatePayoutAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_initiate_payout_action_puzzle()?,
            args: DigInitiatePayoutActionArgs::new(
                self.launcher_id,
                self.validator_launcher_id,
                self.payout_threshold,
            ),
        }
        .to_clvm(ctx)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        my_state: &DigRewardDistributorState,
        solution: NodePtr,
    ) -> Result<(DigMirrorSlotValue, (DigSlotNonce, Bytes32)), DriverError> {
        let solution = ctx.extract::<DigInitiatePayoutActionSolution>(solution)?;

        let new_slot = DigMirrorSlotValue {
            payout_puzzle_hash: solution.mirror_payout_puzzle_hash,
            initial_cumulative_payout: my_state.round_reward_info.cumulative_payout,
            shares: solution.mirror_shares,
        };
        let old_slot = DigMirrorSlotValue {
            payout_puzzle_hash: solution.mirror_payout_puzzle_hash,
            initial_cumulative_payout: solution.mirror_initial_cumulative_payout,
            shares: solution.mirror_shares,
        };
        Ok((
            new_slot,
            (DigSlotNonce::MIRROR, old_slot.tree_hash().into()),
        ))
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut DigRewardDistributor,
        mirror_slot: Slot<DigMirrorSlotValue>,
    ) -> Result<(Conditions, Slot<DigMirrorSlotValue>, u64), DriverError> {
        let my_state = distributor.get_latest_pending_state(ctx)?;

        let withdrawal_amount = mirror_slot.info.value.shares
            * (my_state.round_reward_info.cumulative_payout
                - mirror_slot.info.value.initial_cumulative_payout);

        // this announcement should be asserted to ensure everything goes according to plan
        let initiate_payout_announcement: Bytes32 = clvm_tuple!(
            clvm_tuple!(
                mirror_slot.info.value.payout_puzzle_hash,
                mirror_slot.info.value.shares
            ),
            clvm_tuple!(
                mirror_slot.info.value.initial_cumulative_payout,
                my_state.round_reward_info.cumulative_payout
            ),
        )
        .tree_hash()
        .into();
        let mut initiate_payout_announcement: Vec<u8> = initiate_payout_announcement.to_vec();
        initiate_payout_announcement.insert(0, b'p');

        // spend mirror slot
        mirror_slot.spend(ctx, distributor.info.inner_puzzle_hash().into())?;

        // spend self
        let action_solution = ctx.alloc(&DigInitiatePayoutActionSolution {
            mirror_payout_amount: withdrawal_amount,
            mirror_payout_puzzle_hash: mirror_slot.info.value.payout_puzzle_hash,
            mirror_initial_cumulative_payout: mirror_slot.info.value.initial_cumulative_payout,
            mirror_shares: mirror_slot.info.value.shares,
        })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        let slot_value = self
            .get_slot_value_from_solution(ctx, &my_state, action_solution)?
            .0;
        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                distributor.coin.puzzle_hash,
                initiate_payout_announcement,
            )),
            distributor.created_slot_values_to_slots(vec![slot_value], DigSlotNonce::MIRROR)[0],
            withdrawal_amount,
        ))
    }
}

impl DigInitiatePayoutAction {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        payout_threshold: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_INITIATE_PAYOUT_PUZZLE_HASH,
            args: DigInitiatePayoutActionArgs::new(
                launcher_id,
                validator_launcher_id,
                payout_threshold,
            ),
        }
        .tree_hash()
    }
}

pub const DIG_INITIATE_PAYOUT_PUZZLE: [u8; 863] = hex!("ff02ffff01ff02ffff03ffff22ffff09ffff12ffff11ff8204dfff8205bf80ff8207bf80ff82013f80ffff20ffff15ff2fff82013f808080ffff01ff04ffff04ffff11ff819fff82013f80ffff04ff82015fffff04ff8202dfffff04ff8205dfff8080808080ffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff02ff16ffff04ff02ffff04ff8202bfffff04ff8205bfffff04ff8207bfff808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff17ffff04ffff02ff16ffff04ff02ffff04ff8202bfffff04ff8204dfffff04ff8207bfff808080808080ff8080808080ffff04ffff04ff18ffff04ffff0effff0170ffff0bffff0102ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff8207bf8080ffff0bffff0102ffff0bffff0101ff8205bf80ffff0bffff0101ff8204df80808080ff808080ffff04ffff04ffff0181d6ffff04ff10ffff04ff8202bfffff04ff82013fffff04ffff04ff8202bfff8080ff808080808080ff808080808080ffff01ff088080ff0180ffff04ffff01ffffff333eff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff10ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff17808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_INITIATE_PAYOUT_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    ce3b3f1af37fa98a37c9bb6549c7a073c80b9cc178014711b12f7b3ee57990ad
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigInitiatePayoutActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub validator_singleton_struct_hash: Bytes32,
    pub mirror_slot_1st_curry_hash: Bytes32,
    pub payout_threshold: u64,
}

impl DigInitiatePayoutActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        payout_threshold: u64,
    ) -> Self {
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_V1_1_HASH.into(),
            validator_singleton_struct_hash: SingletonStruct::new(validator_launcher_id)
                .tree_hash()
                .into(),
            mirror_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                DigSlotNonce::MIRROR.to_u64(),
            )
            .into(),
            payout_threshold,
        }
    }
}

impl DigInitiatePayoutActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        payout_threshold: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_INITIATE_PAYOUT_PUZZLE_HASH,
            args: DigInitiatePayoutActionArgs::new(
                launcher_id,
                validator_launcher_id,
                payout_threshold,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigInitiatePayoutActionSolution {
    pub mirror_payout_amount: u64,
    pub mirror_payout_puzzle_hash: Bytes32,
    pub mirror_initial_cumulative_payout: u64,
    #[clvm(rest)]
    pub mirror_shares: u64,
}

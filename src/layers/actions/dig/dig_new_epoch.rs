use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DigRewardDistributor, DigRewardDistributorConstants, DigRewardSlotValue, DigSlotNonce,
    Slot, SpendContextExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigNewEpochAction {
    pub launcher_id: Bytes32,
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
    pub epoch_seconds: u64,
}

impl ToTreeHash for DigNewEpochAction {
    fn tree_hash(&self) -> TreeHash {
        DigNewEpochAction::curry_tree_hash(
            self.launcher_id,
            self.validator_payout_puzzle_hash,
            self.validator_fee_bps,
            self.epoch_seconds,
        )
    }
}

impl Action<DigRewardDistributor> for DigNewEpochAction {
    fn from_constants(constants: &DigRewardDistributorConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
            validator_payout_puzzle_hash: constants.validator_payout_puzzle_hash,
            validator_fee_bps: constants.validator_fee_bps,
            epoch_seconds: constants.epoch_seconds,
        }
    }
}

impl DigNewEpochAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_new_epoch_action_puzzle()?,
            args: DigNewEpochActionArgs::new(
                self.launcher_id,
                self.validator_payout_puzzle_hash,
                self.validator_fee_bps,
                self.epoch_seconds,
            ),
        }
        .to_clvm(ctx)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        solution: NodePtr,
    ) -> Result<(DigRewardSlotValue, (DigSlotNonce, Bytes32)), DriverError> {
        let solution = ctx.extract::<DigNewEpochActionSolution>(solution)?;

        let slot_valie = DigRewardSlotValue {
            epoch_start: solution.slot_epoch_time,
            next_epoch_initialized: solution.slot_next_epoch_initialized,
            rewards: solution.slot_total_rewards,
        };
        Ok((
            slot_valie,
            (DigSlotNonce::REWARD, slot_valie.tree_hash().into()),
        ))
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut DigRewardDistributor,
        reward_slot: Slot<DigRewardSlotValue>,
    ) -> Result<(Conditions, Slot<DigRewardSlotValue>, u64), DriverError> {
        // also returns validator fee
        let my_state = distributor.get_latest_pending_state(ctx)?;

        let epoch_total_rewards =
            if my_state.round_time_info.epoch_end == reward_slot.info.value.epoch_start {
                reward_slot.info.value.rewards
            } else {
                0
            };
        let valdiator_fee =
            epoch_total_rewards * distributor.info.constants.validator_fee_bps / 10000;

        // calculate announcement needed to ensure everything's happening as expected
        let mut new_epoch_announcement: Vec<u8> =
            my_state.round_time_info.epoch_end.tree_hash().to_vec();
        new_epoch_announcement.insert(0, b'e');
        let new_epoch_conditions = Conditions::new()
            .assert_puzzle_announcement(announcement_id(
                distributor.coin.puzzle_hash,
                new_epoch_announcement,
            ))
            .assert_concurrent_puzzle(reward_slot.coin.puzzle_hash);

        // spend slots
        reward_slot.spend(ctx, distributor.info.inner_puzzle_hash().into())?;

        // spend self
        let action_solution = ctx.alloc(&DigNewEpochActionSolution {
            slot_epoch_time: reward_slot.info.value.epoch_start,
            slot_next_epoch_initialized: reward_slot.info.value.next_epoch_initialized,
            slot_total_rewards: reward_slot.info.value.rewards,
            epoch_total_rewards,
            validator_fee: valdiator_fee,
        })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        let slot_value = self.get_slot_value_from_solution(ctx, action_solution)?.0;
        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok((
            new_epoch_conditions,
            distributor.created_slot_values_to_slots(vec![slot_value], DigSlotNonce::REWARD)[0],
            valdiator_fee,
        ))
    }
}

impl DigNewEpochAction {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        validator_payout_puzzle_hash: Bytes32,
        validator_fee_bps: u64,
        epoch_seconds: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_NEW_EPOCH_PUZZLE_HASH,
            args: DigNewEpochActionArgs::new(
                launcher_id,
                validator_payout_puzzle_hash,
                validator_fee_bps,
                epoch_seconds,
            ),
        }
        .tree_hash()
    }
}

pub const DIG_NEW_EPOCH_PUZZLE: [u8; 903] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff8209dfff820ddf80ffff09ffff05ffff14ffff12ff820bbfff1780ffff018227108080ff820fbf80ffff21ffff22ffff09ff82013fff820ddf80ffff09ff820bbfff8205bf8080ffff22ffff15ff820ddfff82013f80ffff20ff8202bf80ffff09ff820bbfff8080808080ffff01ff04ffff04ffff11ff819fff820fbf80ffff04ff82015fffff04ffff04ff8204dfffff10ff8206dfffff11ff820bbfff820fbf808080ffff04ffff04ff820ddfffff10ff820ddfff2f8080ff8080808080ffff04ffff04ff18ffff04ffff0effff0165ffff0bffff0101ff820ddf8080ff808080ffff04ffff04ffff0181d6ffff04ff10ffff04ff0bffff04ff820fbfffff04ffff04ff0bff8080ff808080808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff0bffff0102ffff0bffff0101ff82013f80ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff8205bf808080ff8080808080ffff04ffff02ff1affff04ff02ffff04ff05ffff04ffff0bffff0102ffff0bffff0101ff82013f80ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff8205bf808080ffff04ffff0bffff0101ff82013f80ff808080808080ff808080808080ffff01ff088080ff0180ffff04ffff01ffffff333eff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff10ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff17ff8080ff8080808080ffff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_NEW_EPOCH_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    46e2c7d5e0ca6c28c2ca729b9fd57619f3d293251c3b2f881c730a328515b036
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigNewEpochActionArgs {
    pub reward_slot_1st_curry_hash: Bytes32,
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
    pub epoch_seconds: u64,
}

impl DigNewEpochActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        validator_payout_puzzle_hash: Bytes32,
        validator_fee_bps: u64,
        epoch_seconds: u64,
    ) -> Self {
        Self {
            reward_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                DigSlotNonce::REWARD.to_u64(),
            )
            .into(),
            validator_payout_puzzle_hash,
            validator_fee_bps,
            epoch_seconds,
        }
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigNewEpochActionSolution {
    pub slot_epoch_time: u64,
    pub slot_next_epoch_initialized: bool,
    pub slot_total_rewards: u64,
    pub epoch_total_rewards: u64,
    #[clvm(rest)]
    pub validator_fee: u64,
}

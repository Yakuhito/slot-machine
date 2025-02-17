use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{announcement_id, Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DigCommitmentSlotValue, DigRewardDistributor, DigRewardDistributorConstants,
    DigRewardSlotValue, DigSlotNonce, Slot, SpendContextExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigCommitIncentivesAction {
    pub launcher_id: Bytes32,
    pub epoch_seconds: u64,
}

impl ToTreeHash for DigCommitIncentivesAction {
    fn tree_hash(&self) -> TreeHash {
        DigCommitIncentivesActionArgs::curry_tree_hash(self.launcher_id, self.epoch_seconds)
    }
}

impl Action<DigRewardDistributor> for DigCommitIncentivesAction {
    fn from_constants(launcher_id: Bytes32, constants: &DigRewardDistributorConstants) -> Self {
        Self {
            launcher_id,
            epoch_seconds: constants.epoch_seconds,
        }
    }
}

impl DigCommitIncentivesAction {
    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_commit_incentives_action_puzzle()?,
            args: DigCommitIncentivesActionArgs::new(self.launcher_id, self.epoch_seconds),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_slot_values_from_solution(
        &self,
        ctx: &SpendContext,
        epoch_seconds: u64,
        solution: NodePtr,
    ) -> Result<(DigCommitmentSlotValue, Vec<DigRewardSlotValue>), DriverError> {
        let solution = DigCommitIncentivesActionSolution::from_clvm(&ctx.allocator, solution)?;

        let commitment_slot_value = DigCommitmentSlotValue {
            epoch_start: solution.epoch_start,
            clawback_ph: solution.clawback_ph,
            rewards: solution.rewards_to_add,
        };

        let mut reward_slot_values: Vec<DigRewardSlotValue> = vec![];

        if solution.slot_epoch_time == solution.epoch_start {
            reward_slot_values.push(DigRewardSlotValue {
                epoch_start: solution.epoch_start,
                next_epoch_initialized: solution.slot_next_epoch_initialized,
                rewards: solution.slot_total_rewards + solution.rewards_to_add,
            })
        } else {
            reward_slot_values.push(DigRewardSlotValue {
                epoch_start: solution.slot_epoch_time,
                next_epoch_initialized: true,
                rewards: solution.slot_total_rewards,
            });
            reward_slot_values.push(DigRewardSlotValue {
                epoch_start: solution.epoch_start,
                next_epoch_initialized: false,
                rewards: solution.rewards_to_add,
            });

            let mut start_epoch_time = solution.slot_epoch_time + epoch_seconds;
            let end_epoch_time = solution.epoch_start;
            while end_epoch_time > start_epoch_time {
                reward_slot_values.push(DigRewardSlotValue {
                    epoch_start: start_epoch_time,
                    next_epoch_initialized: true,
                    rewards: 0,
                });

                start_epoch_time += epoch_seconds;
            }
        }

        Ok((commitment_slot_value, reward_slot_values))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        my_puzzle_hash: Bytes32,
        my_inner_puzzle_hash: Bytes32,
        reward_slot: Slot<DigRewardSlotValue>,
        epoch_start: u64,
        clawback_ph: Bytes32,
        rewards_to_add: u64,
    ) -> Result<(Conditions, Spend), DriverError> {
        let Some(reward_slot_value) = reward_slot.info.value else {
            return Err(DriverError::Custom("Reward slot value is None".to_string()));
        };

        let new_commitment_slot_value = DigCommitmentSlotValue {
            epoch_start,
            clawback_ph,
            rewards: rewards_to_add,
        };

        // calculate announcement
        let mut commit_reward_announcement: Vec<u8> =
            new_commitment_slot_value.tree_hash().to_vec();
        commit_reward_announcement.insert(0, b'c');

        // spend reward slot
        reward_slot.spend(ctx, my_inner_puzzle_hash)?;

        // spend self
        let action_solution = DigCommitIncentivesActionSolution {
            slot_epoch_time: reward_slot_value.epoch_start,
            slot_next_epoch_initialized: reward_slot_value.next_epoch_initialized,
            slot_total_rewards: reward_slot_value.rewards,
            epoch_start,
            clawback_ph,
            rewards_to_add,
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                my_puzzle_hash,
                commit_reward_announcement,
            )),
            Spend::new(action_puzzle, action_solution),
        ))
    }
}

pub const DIG_COMMIT_INCENTIVES_PUZZLE: [u8; 1198] = hex!("ff02ffff01ff02ffff03ffff22ffff20ffff15ff8206efff8205df8080ffff15ff820fdfff808080ffff01ff04ffff04ffff10ff4fff820fdf80ffff04ff81afffff04ff82016fffff04ffff04ff8204efff8206ef80ff8080808080ffff02ff12ffff04ff02ffff04ff0bffff04ffff0bffff0102ffff0bffff0101ff8205df80ffff0bffff0102ffff0bffff0101ff820bdf80ffff0bffff0101ff820fdf808080ffff04ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ff82015fffff04ff8202dfff808080808080ff8080808080ffff02ffff03ffff09ff8205dfff819f80ffff01ff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ff82015fffff04ffff10ff8202dfff820fdf80ff808080808080ff8080808080ff8080ffff01ff02ffff03ff82015fffff01ff0880ffff01ff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff26ffff04ff02ffff04ff819fffff04ffff0101ffff04ff8202dfff808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff26ffff04ff02ffff04ff8205dfffff04ff80ffff04ff820fdfff808080808080ff8080808080ffff02ff2effff04ff02ffff04ff05ffff04ff17ffff04ffff10ff819fff1780ffff04ff8205dfff80808080808080808080ff018080ff018080ff80808080808080ffff01ff088080ff0180ffff04ffff01ffffff33ff3e42ff02ffff02ffff03ff05ffff01ff0bff81fcffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ffff04ff28ffff04ffff0effff0163ff0b80ff808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ff0bff8080808080ff178080ffff04ff10ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff81bcffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff17808080ff0bff14ffff0bff14ff81dcff0580ffff0bff14ff0bff819c8080ffff02ffff03ffff09ff17ff2f80ff80ffff01ff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff26ffff04ff02ffff04ff17ffff01ff01ff8080808080ff8080808080ffff02ff2effff04ff02ffff04ff05ffff04ff0bffff04ffff10ff17ff0b80ffff04ff2fff808080808080808080ff0180ff04ff38ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_COMMIT_INCENTIVES_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    a00248887e1f7a1b905172529b4f066edda695df6e01524c79226228a5d0e6b9
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigCommitIncentivesActionArgs {
    pub reward_slot_1st_curry_hash: Bytes32,
    pub commitment_slot_1st_curry_hash: Bytes32,
    pub epoch_seconds: u64,
}

impl DigCommitIncentivesActionArgs {
    pub fn new(launcher_id: Bytes32, epoch_seconds: u64) -> Self {
        Self {
            reward_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                Some(DigSlotNonce::REWARD.to_u64()),
            )
            .into(),
            commitment_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                Some(DigSlotNonce::COMMITMENT.to_u64()),
            )
            .into(),
            epoch_seconds,
        }
    }
}

impl DigCommitIncentivesActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32, epoch_seconds: u64) -> TreeHash {
        CurriedProgram {
            program: DIG_COMMIT_INCENTIVES_PUZZLE_HASH,
            args: DigCommitIncentivesActionArgs::new(launcher_id, epoch_seconds),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigCommitIncentivesActionSolution {
    pub slot_epoch_time: u64,
    pub slot_next_epoch_initialized: bool,
    pub slot_total_rewards: u64,
    pub epoch_start: u64,
    pub clawback_ph: Bytes32,
    #[clvm(rest)]
    pub rewards_to_add: u64,
}

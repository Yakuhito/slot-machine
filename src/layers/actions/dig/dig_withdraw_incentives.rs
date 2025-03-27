use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes, Bytes32},
};
use chia_wallet_sdk::{Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DigCommitmentSlotValue, DigRewardDistributor, DigRewardDistributorConstants,
    DigRewardSlotValue, DigSlotNonce, Slot, SpendContextExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigWithdrawIncentivesAction {
    pub launcher_id: Bytes32,
    pub withdrawal_share_bps: u64,
}

impl ToTreeHash for DigWithdrawIncentivesAction {
    fn tree_hash(&self) -> TreeHash {
        DigWithdrawIncentivesActionArgs::curry_tree_hash(
            self.launcher_id,
            self.withdrawal_share_bps,
        )
    }
}

impl Action<DigRewardDistributor> for DigWithdrawIncentivesAction {
    fn from_constants(constants: &DigRewardDistributorConstants) -> Self {
        Self {
            launcher_id: constants.launcher_id,
            withdrawal_share_bps: constants.withdrawal_share_bps,
        }
    }
}

impl DigWithdrawIncentivesAction {
    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_withdraw_incentives_action_puzzle()?,
            args: DigWithdrawIncentivesActionArgs::new(self.launcher_id, self.withdrawal_share_bps),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        my_constants: &DigRewardDistributorConstants,
        solution: NodePtr,
    ) -> Result<DigRewardSlotValue, DriverError> {
        let solution = DigWithdrawIncentivesActionSolution::from_clvm(&ctx.allocator, solution)?;
        let withdrawal_share = solution.committed_value * my_constants.withdrawal_share_bps / 10000;

        Ok(DigRewardSlotValue {
            epoch_start: solution.reward_slot_epoch_time,
            next_epoch_initialized: solution.reward_slot_next_epoch_initialized,
            rewards: solution.reward_slot_total_rewards - withdrawal_share,
        })
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut DigRewardDistributor,
        commitment_slot: Slot<DigCommitmentSlotValue>,
        reward_slot: Slot<DigRewardSlotValue>,
    ) -> Result<(Conditions, Slot<DigRewardSlotValue>, u64), DriverError> {
        // last u64 = withdrawn amount
        let withdrawal_share = commitment_slot.info.value.rewards
            * distributor.info.constants.withdrawal_share_bps
            / 10000;

        // calculate message that the validator needs to send
        let withdraw_incentives_conditions = Conditions::new()
            .send_message(
                18,
                Bytes::new(Vec::new()),
                vec![distributor.coin.puzzle_hash.to_clvm(&mut ctx.allocator)?],
            )
            .assert_concurrent_puzzle(commitment_slot.coin.puzzle_hash);

        // spend slots
        let my_inner_puzzle_hash: Bytes32 = distributor.info.inner_puzzle_hash().into();
        reward_slot.spend(ctx, my_inner_puzzle_hash)?;
        commitment_slot.spend(ctx, my_inner_puzzle_hash)?;

        // spend self
        let action_solution = DigWithdrawIncentivesActionSolution {
            reward_slot_epoch_time: reward_slot.info.value.epoch_start,
            reward_slot_next_epoch_initialized: reward_slot.info.value.next_epoch_initialized,
            reward_slot_total_rewards: reward_slot.info.value.rewards,
            clawback_ph: commitment_slot.info.value.clawback_ph,
            committed_value: commitment_slot.info.value.rewards,
            withdrawal_share,
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        let slot_value =
            self.get_slot_value_from_solution(ctx, &distributor.info.constants, action_solution)?;
        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok((
            withdraw_incentives_conditions,
            distributor.created_slot_values_to_slots(vec![slot_value], DigSlotNonce::REWARD)[0],
            withdrawal_share,
        ))
    }
}

pub const DIG_WITHDRAW_INCENTIVES_PUZZLE: [u8; 877] = hex!("ff02ffff01ff04ffff04ffff11ff4fffff02ffff03ffff09ff820fdfffff05ffff14ffff12ff17ff820bdf80ffff01822710808080ffff01820fdfffff01ff088080ff018080ffff04ff81afffff04ff82016fffff04ff8202efff8080808080ffff04ffff04ff10ffff04ff819fff808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff82015fffff04ff8202dfff808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff82015fffff04ffff11ff8202dfff820fdf80ff808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff16ffff04ff02ffff04ff819fffff04ff8205dfffff04ff820bdfff808080808080ff8080808080ffff04ffff04ff38ffff04ffff0112ffff04ff80ffff04ff8205dfff8080808080ffff04ffff04ffff0181d6ffff04ff28ffff04ff8205dfffff04ff820fdfffff04ffff04ff8205dfff8080ff808080808080ff8080808080808080ffff04ffff01ffffff55ff3343ff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff28ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ff0b80ffff0bffff0101ff17808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_WITHDRAW_INCENTIVES_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    3beccbe8f65880728c36a1c90754af5ad9acef0016733f860ea83ca7ff62a99b
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigWithdrawIncentivesActionArgs {
    pub reward_slot_1st_curry_hash: Bytes32,
    pub commitment_slot_1st_curry_hash: Bytes32,
    pub withdrawal_share_bps: u64,
}

impl DigWithdrawIncentivesActionArgs {
    pub fn new(launcher_id: Bytes32, withdrawal_share_bps: u64) -> Self {
        Self {
            reward_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                DigSlotNonce::REWARD.to_u64(),
            )
            .into(),
            commitment_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                DigSlotNonce::COMMITMENT.to_u64(),
            )
            .into(),
            withdrawal_share_bps,
        }
    }
}

impl DigWithdrawIncentivesActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32, withdrawal_share_bps: u64) -> TreeHash {
        CurriedProgram {
            program: DIG_WITHDRAW_INCENTIVES_PUZZLE_HASH,
            args: DigWithdrawIncentivesActionArgs::new(launcher_id, withdrawal_share_bps),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigWithdrawIncentivesActionSolution {
    pub reward_slot_epoch_time: u64,
    pub reward_slot_next_epoch_initialized: bool,
    pub reward_slot_total_rewards: u64,
    pub clawback_ph: Bytes32,
    pub committed_value: u64,
    #[clvm(rest)]
    pub withdrawal_share: u64,
}

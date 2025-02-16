use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{DigRewardDistributorInfo, DigSlotNonce, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigWithdrawIncentivesAction {
    pub launcher_id: Bytes32,
    pub withdrawal_share_bps: u64,
}

impl DigWithdrawIncentivesAction {
    pub fn from_info(info: &DigRewardDistributorInfo) -> Self {
        Self {
            launcher_id: info.launcher_id,
            withdrawal_share_bps: info.constants.withdrawal_share_bps,
        }
    }
}

impl Layer for DigWithdrawIncentivesAction {
    type Solution = DigWithdrawIncentivesActionSolution;

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

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigWithdrawIncentivesActionSolution,
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

impl DigWithdrawIncentivesAction {
    pub fn curry_tree_hash(launcher_id: Bytes32, withdrawal_share_bps: u64) -> TreeHash {
        CurriedProgram {
            program: DIG_WITHDRAW_INCENTIVES_PUZZLE_HASH,
            args: DigWithdrawIncentivesActionArgs::new(launcher_id, withdrawal_share_bps),
        }
        .tree_hash()
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
                Some(DigSlotNonce::REWARD.to_u64()),
            )
            .into(),
            commitment_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                Some(DigSlotNonce::COMMITMENT.to_u64()),
            )
            .into(),
            withdrawal_share_bps,
        }
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

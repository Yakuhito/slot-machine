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
pub struct DigNewEpochAction {
    pub launcher_id: Bytes32,
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
    pub epoch_seconds: u64,
}

impl DigNewEpochAction {
    pub fn from_info(info: &DigRewardDistributorInfo) -> Self {
        Self {
            launcher_id: info.launcher_id,
            validator_payout_puzzle_hash: info.constants.validator_payout_puzzle_hash,
            validator_fee_bps: info.constants.validator_fee_bps,
            epoch_seconds: info.constants.epoch_seconds,
        }
    }
}

impl Layer for DigNewEpochAction {
    type Solution = DigNewEpochActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_new_epoch_action_puzzle()?,
            args: DigNewEpochActionArgs::new(
                self.launcher_id,
                self.validator_payout_puzzle_hash,
                self.validator_fee_bps,
                self.epoch_seconds,
            ),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigNewEpochActionSolution,
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

pub const DIG_NEW_EPOCH_PUZZLE: [u8; 889] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff8209dfff820ddf80ffff09ffff05ffff14ffff12ff820bbfff1780ffff018227108080ff820fbf80ffff21ffff22ffff09ff82013fff820ddf80ffff09ff820bbfff8205bf8080ffff22ffff15ff820ddfff82013f80ffff09ff8202bfff8080ffff09ff820bbfff8080808080ffff01ff04ffff04ffff11ff819fff820fbf80ffff04ff82015fffff04ffff04ff8204dfffff10ff8206dfffff11ff820bbfff820fbf808080ffff04ffff04ff820ddfffff10ff820ddfff2f8080ff8080808080ffff04ffff04ff18ffff04ffff0effff0165ffff0bffff0101ff820ddf8080ff808080ffff04ffff04ffff0181d6ffff04ff10ffff04ff0bffff04ff820fbfffff04ffff04ff0bff8080ff808080808080ffff04ffff02ff3effff04ff02ffff04ff05ffff04ffff0bffff0102ffff0bffff0101ff82013f80ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff820bbf808080ff8080808080ffff04ffff02ff1affff04ff02ffff04ff05ffff04ffff0bffff0102ffff0bffff0101ff82013f80ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff820bbf808080ff8080808080ff808080808080ffff01ff088080ff0180ffff04ffff01ffffff333eff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff2effff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff10ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff52ffff02ff2effff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_NEW_EPOCH_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    921a0d2f1780edd2da69415ecb57372cb886170aa40e738495bd968235ff04b0
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
                Some(DigSlotNonce::REWARD.to_u64()),
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
    pub slot_next_epoch_time: u64,
    pub slot_total_rewards: u64,
    pub epoch_total_rewards: u64,
    #[clvm(rest)]
    pub validator_fee: u64,
}

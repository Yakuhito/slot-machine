use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{announcement_id, Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{Action, DigRewardDistributor, DigRewardDistributorConstants, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigAddIncentivesAction {
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
}

impl ToTreeHash for DigAddIncentivesAction {
    fn tree_hash(&self) -> TreeHash {
        DigAddIncentivesActionArgs::curry_tree_hash(
            self.validator_payout_puzzle_hash,
            self.validator_fee_bps,
        )
    }
}

impl Action<DigRewardDistributor> for DigAddIncentivesAction {
    fn from_constants(constants: &DigRewardDistributorConstants) -> Self {
        Self {
            validator_payout_puzzle_hash: constants.validator_payout_puzzle_hash,
            validator_fee_bps: constants.validator_fee_bps,
        }
    }
}

impl DigAddIncentivesAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_add_incentives_action_puzzle()?,
            args: DigAddIncentivesActionArgs {
                validator_payout_puzzle_hash: self.validator_payout_puzzle_hash,
                validator_fee_bps: self.validator_fee_bps,
            },
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut DigRewardDistributor,
        amount: u64,
    ) -> Result<Conditions, DriverError> {
        let my_state = distributor.get_latest_pending_state(&mut ctx.allocator)?;

        // calculate announcement needed to ensure everything's happening as expected
        let mut add_incentives_announcement: Vec<u8> =
            clvm_tuple!(amount, my_state.round_time_info.epoch_end)
                .tree_hash()
                .to_vec();
        add_incentives_announcement.insert(0, b'i');
        let add_incentives_announcement = Conditions::new().assert_puzzle_announcement(
            announcement_id(distributor.coin.puzzle_hash, add_incentives_announcement),
        );

        // spend self
        let action_solution = DigAddIncentivesActionSolution {
            amount,
            validator_fee: amount * distributor.info.constants.validator_fee_bps / 10000,
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok(add_incentives_announcement)
    }
}

pub const DIG_ADD_INCENTIVES_PUZZLE: [u8; 262] = hex!("ff02ffff01ff02ffff03ffff22ffff15ff820377ff82027780ffff15ff4fff8080ffff09ff6fffff05ffff14ffff12ff4fff0b80ffff0182271080808080ffff01ff04ffff04ffff10ff27ffff11ff4fff6f8080ffff04ff57ffff04ffff04ff820137ffff10ff8201b7ffff11ff4fff6f808080ffff04ffff04ff820277ff82037780ff8080808080ffff04ffff04ff06ffff04ffff0effff0169ffff0bffff0102ffff0bffff0101ff4f80ffff0bffff0101ff820377808080ff808080ffff04ffff04ffff0181d6ffff04ff04ffff04ff05ffff04ff6fffff04ffff04ff05ff8080ff808080808080ff80808080ffff01ff088080ff0180ffff04ffff01ff333eff018080");

pub const DIG_ADD_INCENTIVES_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    723650e5eadbf3631e366d7083479124a9ec1823dac069749949fb00dcb41835
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigAddIncentivesActionArgs {
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
}

impl DigAddIncentivesActionArgs {
    pub fn curry_tree_hash(
        validator_payout_puzzle_hash: Bytes32,
        validator_fee_bps: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_ADD_INCENTIVES_PUZZLE_HASH,
            args: DigAddIncentivesActionArgs {
                validator_payout_puzzle_hash,
                validator_fee_bps,
            },
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigAddIncentivesActionSolution {
    pub amount: u64,
    #[clvm(rest)]
    pub validator_fee: u64,
}

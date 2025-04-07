use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
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
        .to_clvm(ctx)
        .map_err(DriverError::ToClvm)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut DigRewardDistributor,
        amount: u64,
    ) -> Result<Conditions, DriverError> {
        let my_state = distributor.get_latest_pending_state(ctx)?;

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
        let action_solution = ctx.alloc(&DigAddIncentivesActionSolution {
            amount,
            validator_fee: amount * distributor.info.constants.validator_fee_bps / 10000,
        })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok(add_incentives_announcement)
    }
}

pub const DIG_ADD_INCENTIVES_PUZZLE: [u8; 275] = hex!("ff02ffff01ff02ffff03ffff22ffff15ff820377ff82027780ffff15ff819fff8080ffff09ff81dfffff05ffff14ffff12ff819fff0b80ffff0182271080808080ffff01ff04ffff04ffff10ff27ffff11ff819fff81df8080ffff04ff57ffff04ffff04ff820137ffff10ff8201b7ffff11ff819fff81df808080ffff04ff820177ff8080808080ffff04ff80ffff04ffff04ffff04ff06ffff04ffff0effff0169ffff0bffff0102ffff0bffff0101ff819f80ffff0bffff0101ff820377808080ff808080ffff04ffff04ffff0181d6ffff04ff04ffff04ff05ffff04ff81dfffff04ffff04ff05ff8080ff808080808080ff808080ff80808080ffff01ff088080ff0180ffff04ffff01ff333eff018080");

pub const DIG_ADD_INCENTIVES_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    b4f3bf213e84c928b40eb8cb1948ae9850cfb2964c6ae0f892c79fbf9e9677fc
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

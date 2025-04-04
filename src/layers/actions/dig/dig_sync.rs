use chia::clvm_utils::{ToTreeHash, TreeHash};
use chia_wallet_sdk::{
    driver::{DriverError, Spend, SpendContext},
    types::{announcement_id, Conditions},
};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{Action, DigRewardDistributor, DigRewardDistributorConstants, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigSyncAction {}

impl ToTreeHash for DigSyncAction {
    fn tree_hash(&self) -> TreeHash {
        DigSyncActionArgs::curry_tree_hash()
    }
}

impl Action<DigRewardDistributor> for DigSyncAction {
    fn from_constants(_constants: &DigRewardDistributorConstants) -> Self {
        Self {}
    }
}

impl DigSyncAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        ctx.dig_sync_action_puzzle()
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        distributor: &mut DigRewardDistributor,
        update_time: u64,
    ) -> Result<Conditions, DriverError> {
        // calculate announcement needed to ensure everything's happening as expected
        let my_state = distributor.get_latest_pending_state(ctx)?;
        let mut new_epoch_announcement: Vec<u8> =
            clvm_tuple!(update_time, my_state.round_time_info.epoch_end)
                .tree_hash()
                .to_vec();
        new_epoch_announcement.insert(0, b's');
        let new_epoch_conditions = Conditions::new().assert_puzzle_announcement(announcement_id(
            distributor.coin.puzzle_hash,
            new_epoch_announcement,
        ));

        // spend self
        let action_solution = ctx.alloc(&DigSyncActionSolution { update_time })?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        distributor.insert(Spend::new(action_puzzle, action_solution));
        Ok(new_epoch_conditions)
    }
}

pub const DIG_SYNC_PUZZLE: [u8; 292] = hex!("ff02ffff01ff02ffff03ffff22ffff20ffff15ff13ff81dd8080ffff15ff13ff819d8080ffff01ff04ffff04ff09ffff04ff15ffff04ffff02ff0effff04ff02ffff04ff15ffff04ff4dffff04ff6dffff04ffff02ffff03ffff15ff15ff8080ffff01ff05ffff14ffff12ff6dffff11ff13ff819d8080ffff12ff15ffff11ff81ddff819d80808080ff8080ff0180ff80808080808080ffff04ffff04ff13ff81dd80ff8080808080ffff04ffff04ff04ffff04ff13ff808080ffff04ffff04ff0affff04ffff0effff0173ffff0bffff0102ffff0bffff0101ff1380ffff0bffff0101ff81dd808080ff808080ff80808080ffff01ff088080ff0180ffff04ffff01ff51ff3eff04ffff10ff0bff2f80ffff11ff17ffff12ff2fff05808080ff018080");

pub const DIG_SYNC_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    3414a6bf0b4e676ef2b19a8c3468b538e509ee6a02f1ab3743b2e03c4a2575b2
    "
));

pub struct DigSyncActionArgs {}
impl DigSyncActionArgs {
    pub fn curry_tree_hash() -> TreeHash {
        DIG_SYNC_PUZZLE_HASH
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigSyncActionSolution {
    pub update_time: u64,
}

use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin},
    puzzles::singleton::{SingletonStruct, SINGLETON_TOP_LAYER_PUZZLE_HASH},
};
use chia_wallet_sdk::{Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{
    Action, DigMirrorSlotValue, DigRewardDistributor, DigRewardDistributorConstants,
    DigRewardDistributorState, DigSlotNonce, Slot, SpendContextExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigAddMirrorAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
    pub max_second_offset: u64,
}

impl ToTreeHash for DigAddMirrorAction {
    fn tree_hash(&self) -> TreeHash {
        DigAddMirrorActionArgs::curry_tree_hash(
            self.launcher_id,
            self.validator_launcher_id,
            self.max_second_offset,
        )
    }
}

impl Action<DigRewardDistributor> for DigAddMirrorAction {
    fn from_constants(launcher_id: Bytes32, constants: &DigRewardDistributorConstants) -> Self {
        Self {
            launcher_id,
            validator_launcher_id: constants.validator_launcher_id,
            max_second_offset: constants.max_seconds_offset,
        }
    }
}

impl DigAddMirrorAction {
    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_add_mirror_action_puzzle()?,
            args: DigAddMirrorActionArgs::new(
                self.launcher_id,
                self.validator_launcher_id,
                self.max_second_offset,
            ),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_slot_value_from_solution(
        &self,
        ctx: &SpendContext,
        state: &DigRewardDistributorState,
        solution: NodePtr,
    ) -> Result<DigMirrorSlotValue, DriverError> {
        let solution = DigAddMirrorActionSolution::from_clvm(&ctx.allocator, solution)?;

        Ok(DigMirrorSlotValue {
            payout_puzzle_hash: solution.mirror_payout_puzzle_hash,
            initial_cumulative_payout: state.round_reward_info.cumulative_payout,
            shares: solution.mirror_shares,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        my_coin: Coin,
        payout_puzzle_hash: Bytes32,
        shares: u64,
        validator_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, Spend), DriverError> {
        // calculate message that the validator needs to send
        let add_mirror_message: Bytes32 =
            clvm_tuple!(payout_puzzle_hash, shares).tree_hash().into();
        let mut add_mirror_message: Vec<u8> = add_mirror_message.to_vec();
        add_mirror_message.insert(0, b'a');
        let add_mirror_message = Conditions::new().send_message(
            18,
            add_mirror_message.into(),
            vec![my_coin.puzzle_hash.to_clvm(&mut ctx.allocator)?],
        );

        // spend self
        let action_solution = DigAddMirrorActionSolution {
            validator_singleton_inner_puzzle_hash,
            mirror_payout_puzzle_hash: payout_puzzle_hash,
            mirror_shares: shares,
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        Ok((
            add_mirror_message,
            Spend::new(action_puzzle, action_solution),
        ))
    }
}

pub const DIG_ADD_MIRROR_PUZZLE: [u8; 682] = hex!("ff02ffff01ff04ffff04ff819fffff04ffff10ff82015fff8203bf80ffff04ff8202dfffff04ff8205dfff8080808080ffff04ffff04ff14ffff04ffff0112ffff04ffff0effff0161ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0101ff8203bf808080ffff04ffff0bff5affff0bff1cffff0bff1cff6aff0580ffff0bff1cffff0bff7affff0bff1cffff0bff1cff6aff0b80ffff0bff1cffff0bff7affff0bff1cffff0bff1cff6aff82013f80ffff0bff1cff6aff4a808080ff4a808080ff4a808080ff8080808080ffff04ffff02ff16ffff04ff02ffff04ff17ffff04ffff0bffff0102ffff0bffff0101ff8202bf80ffff0bffff0102ffff0bffff0101ff8204df80ffff0bffff0101ff8203bf808080ff8080808080ffff04ffff04ff10ffff04ffff10ff8209dfff2f80ff808080ff8080808080ffff04ffff01ffffff5533ff4302ffffff02ffff03ff05ffff01ff0bff7affff02ff3effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016a80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff18ffff04ffff02ff2effff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff5affff02ff3effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ff0bff1cffff0bff1cff6aff0580ffff0bff1cff0bff4a8080ff018080");

pub const DIG_ADD_MIRROR_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    ea0fcd651f805040acb49b451f5a16a81e98430f277987530bff87cd2316e559
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigAddMirrorActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub validator_singleton_struct_hash: Bytes32,
    pub mirror_slot_1st_curry_hash: Bytes32,
    pub max_second_offset: u64,
}

impl DigAddMirrorActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        max_second_offset: u64,
    ) -> Self {
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            validator_singleton_struct_hash: SingletonStruct::new(validator_launcher_id)
                .tree_hash()
                .into(),
            mirror_slot_1st_curry_hash: Slot::<()>::first_curry_hash(
                launcher_id,
                Some(DigSlotNonce::MIRROR.to_u64()),
            )
            .into(),
            max_second_offset,
        }
    }
}

impl DigAddMirrorActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        max_second_offset: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_ADD_MIRROR_PUZZLE_HASH,
            args: DigAddMirrorActionArgs::new(
                launcher_id,
                validator_launcher_id,
                max_second_offset,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigAddMirrorActionSolution {
    pub validator_singleton_inner_puzzle_hash: Bytes32,
    pub mirror_payout_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub mirror_shares: u64,
}

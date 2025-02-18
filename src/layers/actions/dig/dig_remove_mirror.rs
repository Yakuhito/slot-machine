use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
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
pub struct DigRemoveMirrorAction {
    pub launcher_id: Bytes32,
    pub validator_launcher_id: Bytes32,
    pub max_seconds_offset: u64,
}

impl ToTreeHash for DigRemoveMirrorAction {
    fn tree_hash(&self) -> TreeHash {
        DigRemoveMirrorActionArgs::curry_tree_hash(
            self.launcher_id,
            self.validator_launcher_id,
            self.max_seconds_offset,
        )
    }
}

impl Action<DigRewardDistributor> for DigRemoveMirrorAction {
    fn from_constants(launcher_id: Bytes32, constants: &DigRewardDistributorConstants) -> Self {
        Self {
            launcher_id,
            validator_launcher_id: constants.validator_launcher_id,
            max_seconds_offset: constants.max_seconds_offset,
        }
    }
}

impl DigRemoveMirrorAction {
    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_remove_mirror_action_puzzle()?,
            args: DigRemoveMirrorActionArgs::new(
                self.launcher_id,
                self.validator_launcher_id,
                self.max_seconds_offset,
            ),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        my_puzzle_hash: Bytes32,
        my_state: &DigRewardDistributorState,
        my_inner_puzzle_hash: Bytes32,
        mirror_slot: Slot<DigMirrorSlotValue>,
        validator_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, Spend, u64), DriverError> {
        // u64 = last payment amount
        let Some(mirror_slot_value) = mirror_slot.info.value else {
            return Err(DriverError::Custom("Mirror slot value is None".to_string()));
        };

        // compute message that the validator needs to send
        let remove_mirror_message: Bytes32 = clvm_tuple!(
            mirror_slot_value.payout_puzzle_hash,
            mirror_slot_value.shares
        )
        .tree_hash()
        .into();
        let mut remove_mirror_message: Vec<u8> = remove_mirror_message.to_vec();
        remove_mirror_message.insert(0, b'r');

        let remove_mirror_conditions = Conditions::new()
            .send_message(
                18,
                remove_mirror_message.into(),
                vec![my_puzzle_hash.to_clvm(&mut ctx.allocator)?],
            )
            .assert_concurrent_puzzle(mirror_slot.coin.puzzle_hash);

        // spend mirror slot
        mirror_slot.spend(ctx, my_inner_puzzle_hash)?;

        // spend self
        let mirror_payout_amount = mirror_slot_value.shares
            * (my_state.round_reward_info.cumulative_payout
                - mirror_slot_value.initial_cumulative_payout);
        let action_solution = DigRemoveMirrorActionSolution {
            validator_singleton_inner_puzzle_hash,
            mirror_payout_amount,
            mirror_payout_puzzle_hash: mirror_slot_value.payout_puzzle_hash,
            mirror_initial_cumulative_payout: mirror_slot_value.initial_cumulative_payout,
            mirror_shares: mirror_slot_value.shares,
        }
        .to_clvm(&mut ctx.allocator)?;
        let action_puzzle = self.construct_puzzle(ctx)?;

        Ok((
            remove_mirror_conditions,
            Spend::new(action_puzzle, action_solution),
            mirror_payout_amount,
        ))
    }
}

pub const DIG_REMOVE_MIRROR_PUZZLE: [u8; 785] = hex!("ff02ffff01ff02ffff03ffff09ff8202bfffff12ffff11ff8204dfff820bbf80ff820fbf8080ffff01ff04ffff04ffff11ff819fff8202bf80ffff04ffff11ff82015fff820fbf80ffff04ff8202dfffff04ff8205dfff8080808080ffff04ffff04ff14ffff04ffff0112ffff04ffff0effff0172ffff0bffff0102ffff0bffff0101ff8205bf80ffff0bffff0101ff820fbf808080ffff04ffff0bff5affff0bff3cffff0bff3cff6aff0580ffff0bff3cffff0bff7affff0bff3cffff0bff3cff6aff0b80ffff0bff3cffff0bff7affff0bff3cffff0bff3cff6aff82013f80ffff0bff3cff6aff4a808080ff4a808080ff4a808080ff8080808080ffff04ffff04ff10ffff04ffff10ff8209dfff2f80ff808080ffff04ffff02ff3effff04ff02ffff04ff17ffff04ffff0bffff0102ffff0bffff0101ff8205bf80ffff0bffff0102ffff0bffff0101ff820bbf80ffff0bffff0101ff820fbf808080ff8080808080ffff04ffff04ffff0181d6ffff04ff18ffff04ff8205bfffff04ff8202bfffff04ffff04ff8205bfff8080ff808080808080ff808080808080ffff01ff088080ff0180ffff04ffff01ffffff5533ff43ff4202ffffff02ffff03ff05ffff01ff0bff7affff02ff2effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016a80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff5affff02ff2effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff3cffff0bff3cff6aff0580ffff0bff3cff0bff4a8080ff04ff2cffff04ffff0112ffff04ff80ffff04ffff02ff16ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const DIG_REMOVE_MIRROR_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    cead4cf3c010ce30e0e8d7976f65f8a738666b6192c4532567daaa63196ded5e
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigRemoveMirrorActionArgs {
    pub singleton_mod_hash: Bytes32,
    pub validator_singleton_struct_hash: Bytes32,
    pub mirror_slot_1st_curry_hash: Bytes32,
    pub max_seconds_offset: u64,
}

impl DigRemoveMirrorActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        max_seconds_offset: u64,
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
            max_seconds_offset,
        }
    }
}

impl DigRemoveMirrorActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        validator_launcher_id: Bytes32,
        max_seconds_offset: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_REMOVE_MIRROR_PUZZLE_HASH,
            args: DigRemoveMirrorActionArgs::new(
                launcher_id,
                validator_launcher_id,
                max_seconds_offset,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigRemoveMirrorActionSolution {
    pub validator_singleton_inner_puzzle_hash: Bytes32,
    pub mirror_payout_amount: u64,
    pub mirror_payout_puzzle_hash: Bytes32,
    pub mirror_initial_cumulative_payout: u64,
    #[clvm(rest)]
    pub mirror_shares: u64,
}

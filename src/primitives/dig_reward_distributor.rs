use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32, Coin},
    puzzles::{
        singleton::{SingletonSolution, SingletonStruct},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    announcement_id, Conditions, DriverError, Layer, Puzzle, Spend, SpendContext,
};
use clvm_traits::{clvm_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    ActionLayer, ActionLayerSolution, DigAddIncentivesAction, DigAddIncentivesActionSolution,
    DigAddMirrorAction, DigAddMirrorActionSolution, DigCommitIncentivesAction,
    DigCommitIncentivesActionSolution, DigInitiatePayoutAction, DigInitiatePayoutActionSolution,
    DigNewEpochAction, DigNewEpochActionSolution, DigRemoveMirrorAction,
    DigRemoveMirrorActionSolution, DigSyncAction, DigSyncActionSolution,
    DigWithdrawIncentivesAction, DigWithdrawIncentivesActionSolution, RawActionLayerSolution,
    ReserveFinalizerSolution,
};

use super::{
    DigCommitmentSlotValue, DigMirrorSlotValue, DigRewardDistributorConstants,
    DigRewardDistributorInfo, DigRewardDistributorState, DigRewardSlotValue, DigSlotNonce, Reserve,
    Slot, SlotInfo, SlotProof,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct DigRewardDistributor {
    pub coin: Coin,
    pub proof: Proof,
    pub info: DigRewardDistributorInfo,
}

impl DigRewardDistributor {
    pub fn new(coin: Coin, proof: Proof, info: DigRewardDistributorInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: DigRewardDistributorConstants,
    ) -> Result<Option<(Self, Reserve)>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) =
            DigRewardDistributorInfo::parse(allocator, parent_puzzle, constants)?
        else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let parent_solution = SingletonSolution::<NodePtr>::from_clvm(allocator, parent_solution)?;
        let new_state = ActionLayer::<DigRewardDistributorState>::get_new_state(
            allocator,
            parent_info.state,
            parent_solution.inner_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        let parent_inner_solution = RawActionLayerSolution::<
            NodePtr,
            NodePtr,
            ReserveFinalizerSolution,
        >::from_clvm(allocator, parent_solution.inner_solution)?;
        let parent_reserve = Coin::new(
            parent_inner_solution.finalizer_solution.reserve_parent_id,
            constants.reserve_full_puzzle_hash,
            parent_info.state.total_reserves,
        );
        let reserve = Reserve::new(
            parent_reserve.coin_id(),
            LineageProof {
                parent_parent_coin_info: parent_reserve.parent_coin_info,
                parent_inner_puzzle_hash: constants.reserve_inner_puzzle_hash,
                parent_amount: parent_reserve.amount,
            },
            constants.reserve_asset_id,
            SingletonStruct::new(parent_info.launcher_id)
                .tree_hash()
                .into(),
            0,
            new_state.total_reserves,
        );

        Ok(Some((
            DigRewardDistributor {
                coin: new_coin,
                proof,
                info: new_info,
            },
            reserve,
        )))
    }
}

#[allow(clippy::large_enum_variant)]
pub enum DigRewardDistributorAction {
    AddIncentives(DigAddIncentivesActionSolution),
    AddMirror(DigAddMirrorActionSolution),
    CommitIncentives(DigCommitIncentivesActionSolution),
    InitiatePayout(DigInitiatePayoutActionSolution),
    NewEpoch(DigNewEpochActionSolution),
    RemoveMirror(DigRemoveMirrorActionSolution),
    Sync(DigSyncActionSolution),
    WithdrawIncentives(DigWithdrawIncentivesActionSolution),
}

impl DigRewardDistributor {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        reserve_parent_id: Bytes32,
        actions: Vec<DigRewardDistributorAction>,
    ) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_spends: Vec<Spend> = actions
            .into_iter()
            .map(|action| match action {
                DigRewardDistributorAction::AddIncentives(solution) => {
                    let layer = DigAddIncentivesAction {};

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::AddMirror(solution) => {
                    let layer = DigAddMirrorAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::CommitIncentives(solution) => {
                    let layer = DigCommitIncentivesAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::InitiatePayout(solution) => {
                    let layer = DigInitiatePayoutAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::NewEpoch(solution) => {
                    let layer = DigNewEpochAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::RemoveMirror(solution) => {
                    let layer = DigRemoveMirrorAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::Sync(solution) => {
                    let layer = DigSyncAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::WithdrawIncentives(solution) => {
                    let layer = DigWithdrawIncentivesAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let action_puzzle_hashes = action_spends
            .iter()
            .map(|a| ctx.tree_hash(a.puzzle).into())
            .collect::<Vec<Bytes32>>();

        let finalizer_solution =
            ReserveFinalizerSolution { reserve_parent_id }.to_clvm(&mut ctx.allocator)?;

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(
                            &DigRewardDistributorInfo::action_puzzle_hashes(
                                self.info.launcher_id,
                                &self.info.constants,
                            ),
                            &action_puzzle_hashes,
                        )
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends,
                    finalizer_solution,
                },
            },
        )?;

        Ok(Spend::new(puzzle, solution))
    }

    pub fn add_mirror(
        self,
        ctx: &mut SpendContext,
        reserve: Reserve,
        payout_puzzle_hash: Bytes32,
        shares: u64,
        validator_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<
        (
            Conditions,
            DigRewardDistributor,
            Reserve,
            Slot<DigMirrorSlotValue>,
        ),
        DriverError,
    > {
        let new_slot = Slot::new(
            SlotProof {
                parent_parent_info: self.coin.parent_coin_info,
                parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            },
            SlotInfo::from_value(
                self.info.launcher_id,
                DigMirrorSlotValue {
                    payout_puzzle_hash,
                    initial_cumulative_payout: self.info.state.round_reward_info.cumulative_payout,
                    shares,
                },
                Some(DigSlotNonce::MIRROR.to_u64()),
            ),
        );

        // calculate message that the validator needs to send
        let add_mirror_message: Bytes32 =
            clvm_tuple!(payout_puzzle_hash, shares).tree_hash().into();
        let mut add_mirror_message: Vec<u8> = add_mirror_message.to_vec();
        add_mirror_message.insert(0, b'a');
        let add_mirror_message = Conditions::new().send_message(
            18,
            add_mirror_message.into(),
            vec![self.coin.puzzle_hash.to_clvm(&mut ctx.allocator)?],
        );

        // spend self
        let add_mirror = DigRewardDistributorAction::AddMirror(DigAddMirrorActionSolution {
            validator_singleton_inner_puzzle_hash,
            mirror_payout_puzzle_hash: payout_puzzle_hash,
            mirror_shares: shares,
        });

        let my_state = self.info.state;
        let my_inner_puzzle_hash = self.info.inner_puzzle_hash();

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, reserve.coin.parent_coin_info, vec![add_mirror])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        // spend reserve
        reserve.spend_for_reserve_finalizer_controller(
            ctx,
            my_state,
            new_reserve.coin.amount,
            my_inner_puzzle_hash.into(),
            my_spend.solution,
        )?;

        Ok((
            add_mirror_message,
            new_dig_reward_distributor,
            new_reserve,
            new_slot,
        ))
    }

    // does NOT spend reserve
    #[allow(clippy::type_complexity)]
    pub fn commit_incentives(
        self,
        ctx: &mut SpendContext,
        reserve_parent_id: Bytes32,
        reward_slot: Slot<DigRewardSlotValue>,
        epoch_start: u64,
        clawback_ph: Bytes32,
        rewards_to_add: u64,
    ) -> Result<
        (
            Conditions,
            DigRewardDistributor,
            Reserve,
            NodePtr,
            Slot<DigCommitmentSlotValue>,
            Vec<Slot<DigRewardSlotValue>>,
        ),
        DriverError,
    > {
        let Some(reward_slot_value) = reward_slot.info.value else {
            return Err(DriverError::Custom("Reward slot value is None".to_string()));
        };

        let new_slot_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        let new_commitment_slot_info = SlotInfo::from_value(
            self.info.launcher_id,
            DigCommitmentSlotValue {
                epoch_start,
                clawback_ph,
                rewards: rewards_to_add,
            },
            Some(DigSlotNonce::COMMITMENT.to_u64()),
        );
        let new_commitment_slot = Slot::new(new_slot_proof, new_commitment_slot_info);

        let mut new_reward_slots: Vec<Slot<DigRewardSlotValue>> = vec![];
        if epoch_start == reward_slot_value.epoch_start {
            new_reward_slots.push(Slot::new(
                new_slot_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    DigRewardSlotValue {
                        epoch_start,
                        next_epoch_start: reward_slot_value.next_epoch_start,
                        rewards: reward_slot_value.rewards + rewards_to_add,
                    },
                    Some(DigSlotNonce::REWARD.to_u64()),
                ),
            ));
        } else {
            new_reward_slots.push(Slot::new(
                new_slot_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    DigRewardSlotValue {
                        epoch_start: reward_slot_value.epoch_start,
                        next_epoch_start: reward_slot_value.epoch_start
                            + self.info.constants.epoch_seconds,
                        rewards: reward_slot_value.rewards,
                    },
                    Some(DigSlotNonce::REWARD.to_u64()),
                ),
            ));
            new_reward_slots.push(Slot::new(
                new_slot_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    DigRewardSlotValue {
                        epoch_start,
                        next_epoch_start: 0,
                        rewards: rewards_to_add,
                    },
                    Some(DigSlotNonce::REWARD.to_u64()),
                ),
            ));
            let mut start_epoch_time =
                reward_slot_value.epoch_start + self.info.constants.epoch_seconds;
            let end_epoch_time = epoch_start;
            while end_epoch_time > start_epoch_time {
                new_reward_slots.push(Slot::new(
                    new_slot_proof,
                    SlotInfo::from_value(
                        self.info.launcher_id,
                        DigRewardSlotValue {
                            epoch_start: start_epoch_time,
                            next_epoch_start: start_epoch_time + self.info.constants.epoch_seconds,
                            rewards: 0,
                        },
                        Some(DigSlotNonce::REWARD.to_u64()),
                    ),
                ));
                start_epoch_time += self.info.constants.epoch_seconds;
            }
        }

        // calculate announcement
        let mut commit_reward_announcement: Vec<u8> = new_commitment_slot_info.value_hash.to_vec();
        commit_reward_announcement.insert(0, b'c');

        // spend reward slot
        reward_slot.spend(ctx, self.info.inner_puzzle_hash().into())?;

        // spend self
        let commit_incentives =
            DigRewardDistributorAction::CommitIncentives(DigCommitIncentivesActionSolution {
                slot_epoch_time: reward_slot_value.epoch_start,
                slot_next_epoch_time: reward_slot_value.next_epoch_start,
                slot_total_rewards: reward_slot_value.rewards,
                epoch_start,
                clawback_ph,
                rewards_to_add,
            });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, reserve_parent_id, vec![commit_incentives])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                my_coin.puzzle_hash,
                commit_reward_announcement,
            )),
            new_dig_reward_distributor,
            new_reserve,
            my_spend.solution,
            new_commitment_slot,
            new_reward_slots,
        ))
    }

    pub fn withdraw_incentives(
        self,
        ctx: &mut SpendContext,
        reserve: Reserve,
        commitment_slot: Slot<DigCommitmentSlotValue>,
        reward_slot: Slot<DigRewardSlotValue>,
    ) -> Result<
        (
            Conditions,
            DigRewardDistributor,
            Reserve,
            u64, // withdrawn amount
            Slot<DigRewardSlotValue>,
        ),
        DriverError,
    > {
        let Some(reward_slot_value) = reward_slot.info.value else {
            return Err(DriverError::Custom("Reward slot value is None".to_string()));
        };
        let Some(commitment_slot_value) = commitment_slot.info.value else {
            return Err(DriverError::Custom(
                "Commitment slot value is None".to_string(),
            ));
        };

        let withdrawal_share =
            commitment_slot_value.rewards * self.info.constants.withdrawal_share_bps / 10000;
        let new_reward_slot = Slot::new(
            SlotProof {
                parent_parent_info: self.coin.parent_coin_info,
                parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            },
            SlotInfo::from_value(
                self.info.launcher_id,
                DigRewardSlotValue {
                    epoch_start: reward_slot_value.epoch_start,
                    next_epoch_start: reward_slot_value.next_epoch_start,
                    rewards: reward_slot_value.rewards - withdrawal_share,
                },
                Some(DigSlotNonce::REWARD.to_u64()),
            ),
        );

        // calculate message that the validator needs to send
        let withdraw_incentives_conditions = Conditions::new()
            .send_message(
                18,
                Bytes::new(Vec::new()),
                vec![self.coin.puzzle_hash.to_clvm(&mut ctx.allocator)?],
            )
            .assert_concurrent_puzzle(commitment_slot.coin.puzzle_hash);

        // spend slots
        let spender_inner_puzzle_hash = self.info.inner_puzzle_hash().into();
        reward_slot.spend(ctx, spender_inner_puzzle_hash)?;
        commitment_slot.spend(ctx, spender_inner_puzzle_hash)?;

        // spend self
        let withdraw_incentives =
            DigRewardDistributorAction::WithdrawIncentives(DigWithdrawIncentivesActionSolution {
                reward_slot_epoch_time: reward_slot_value.epoch_start,
                reward_slot_next_epoch_time: reward_slot_value.next_epoch_start,
                reward_slot_total_rewards: reward_slot_value.rewards,
                clawback_ph: commitment_slot_value.clawback_ph,
                committed_value: commitment_slot_value.rewards,
                withdrawal_share,
            });

        let my_state = self.info.state;
        let my_inner_puzzle_hash = self.info.inner_puzzle_hash();

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(
            ctx,
            reserve.coin.parent_coin_info,
            vec![withdraw_incentives],
        )?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        // spend reserve
        reserve.spend_for_reserve_finalizer_controller(
            ctx,
            my_state,
            new_reserve.coin.amount,
            my_inner_puzzle_hash.into(),
            my_spend.solution,
        )?;

        Ok((
            withdraw_incentives_conditions,
            new_dig_reward_distributor,
            new_reserve,
            withdrawal_share,
            new_reward_slot,
        ))
    }

    pub fn new_epoch(
        self,
        ctx: &mut SpendContext,
        reserve: Reserve,
        reward_slot: Slot<DigRewardSlotValue>,
        epoch_total_rewards: u64,
    ) -> Result<
        (
            Conditions,
            DigRewardDistributor,
            Reserve,
            u64, // validator fee
            Slot<DigRewardSlotValue>,
        ),
        DriverError,
    > {
        let Some(reward_slot_value) = reward_slot.info.value else {
            return Err(DriverError::Custom("Reward slot value is None".to_string()));
        };

        let valdiator_fee = epoch_total_rewards * self.info.constants.validator_fee_bps / 10000;
        let new_reward_slot = Slot::new(
            SlotProof {
                parent_parent_info: self.coin.parent_coin_info,
                parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            },
            reward_slot.info,
        );

        // calculate announcement needed to ensure everything's happening as expected
        let mut new_epoch_announcement: Vec<u8> = self
            .info
            .state
            .round_time_info
            .epoch_end
            .tree_hash()
            .to_vec();
        new_epoch_announcement.insert(0, b'e');
        let new_epoch_conditions = Conditions::new()
            .assert_puzzle_announcement(announcement_id(
                self.coin.puzzle_hash,
                new_epoch_announcement,
            ))
            .assert_concurrent_puzzle(reward_slot.coin.puzzle_hash);

        // spend slots
        reward_slot.spend(ctx, self.info.inner_puzzle_hash().into())?;

        // spend self
        let new_epoch = DigRewardDistributorAction::NewEpoch(DigNewEpochActionSolution {
            slot_epoch_time: reward_slot_value.epoch_start,
            slot_next_epoch_time: reward_slot_value.next_epoch_start,
            slot_total_rewards: reward_slot_value.rewards,
            epoch_total_rewards,
            validator_fee: valdiator_fee,
        });

        let my_state = self.info.state;
        let my_inner_puzzle_hash = self.info.inner_puzzle_hash();

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, reserve.coin.parent_coin_info, vec![new_epoch])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        // spend reserve
        reserve.spend_for_reserve_finalizer_controller(
            ctx,
            my_state,
            new_reserve.coin.amount,
            my_inner_puzzle_hash.into(),
            my_spend.solution,
        )?;

        Ok((
            new_epoch_conditions,
            new_dig_reward_distributor,
            new_reserve,
            valdiator_fee,
            new_reward_slot,
        ))
    }

    pub fn sync(
        self,
        ctx: &mut SpendContext,
        reserve: Reserve,
        update_time: u64,
    ) -> Result<(Conditions, DigRewardDistributor, Reserve), DriverError> {
        // calculate announcement needed to ensure everything's happening as expected
        let mut new_epoch_announcement: Vec<u8> =
            clvm_tuple!(update_time, self.info.state.round_time_info.epoch_end)
                .tree_hash()
                .to_vec();
        new_epoch_announcement.insert(0, b's');
        let new_epoch_conditions = Conditions::new().assert_puzzle_announcement(announcement_id(
            self.coin.puzzle_hash,
            new_epoch_announcement,
        ));

        // spend self
        let sync_action = DigRewardDistributorAction::Sync(DigSyncActionSolution { update_time });

        let my_state = self.info.state;
        let my_inner_puzzle_hash = self.info.inner_puzzle_hash();

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, reserve.coin.parent_coin_info, vec![sync_action])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        // spend reserve
        reserve.spend_for_reserve_finalizer_controller(
            ctx,
            my_state,
            new_reserve.coin.amount,
            my_inner_puzzle_hash.into(),
            my_spend.solution,
        )?;

        Ok((
            new_epoch_conditions,
            new_dig_reward_distributor,
            new_reserve,
        ))
    }

    pub fn add_incentives(
        self,
        ctx: &mut SpendContext,
        reserve: &Reserve,
        amount: u64,
    ) -> Result<(Conditions, DigRewardDistributor, Reserve, NodePtr), DriverError> {
        // calculate announcement needed to ensure everything's happening as expected
        let mut add_incentives_announcement: Vec<u8> =
            clvm_tuple!(amount, self.info.state.round_time_info.epoch_end)
                .tree_hash()
                .to_vec();
        add_incentives_announcement.insert(0, b'c');
        let add_incentives_announcement = Conditions::new().assert_puzzle_announcement(
            announcement_id(self.coin.puzzle_hash, add_incentives_announcement),
        );

        // spend self
        let add_incentives_action =
            DigRewardDistributorAction::AddIncentives(DigAddIncentivesActionSolution { amount });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(
            ctx,
            reserve.coin.parent_coin_info,
            vec![add_incentives_action],
        )?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            add_incentives_announcement,
            new_dig_reward_distributor,
            new_reserve,
            my_spend.solution,
        ))
    }

    pub fn initiate_payout(
        self,
        ctx: &mut SpendContext,
        reserve: Reserve,
        mirror_slot: Slot<DigMirrorSlotValue>,
    ) -> Result<
        (
            Conditions,
            DigRewardDistributor,
            Reserve,
            Slot<DigMirrorSlotValue>,
            u64, // payout amount
        ),
        DriverError,
    > {
        let Some(mirror_slot_value) = mirror_slot.info.value else {
            return Err(DriverError::Custom("Mirror slot value is None".to_string()));
        };

        let new_slot = Slot::new(
            SlotProof {
                parent_parent_info: self.coin.parent_coin_info,
                parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            },
            SlotInfo::from_value(
                self.info.launcher_id,
                DigMirrorSlotValue {
                    payout_puzzle_hash: mirror_slot_value.payout_puzzle_hash,
                    initial_cumulative_payout: self.info.state.round_reward_info.cumulative_payout,
                    shares: mirror_slot_value.shares,
                },
                Some(DigSlotNonce::MIRROR.to_u64()),
            ),
        );

        let withdrawal_amount = mirror_slot_value.shares
            * (self.info.state.round_reward_info.cumulative_payout
                - mirror_slot_value.initial_cumulative_payout);

        // this announcement should be asserted to ensure everything goes according to plan
        let initiate_payout_announcement: Bytes32 = clvm_tuple!(
            clvm_tuple!(
                mirror_slot_value.payout_puzzle_hash,
                mirror_slot_value.shares
            ),
            clvm_tuple!(
                mirror_slot_value.initial_cumulative_payout,
                self.info.state.round_reward_info.cumulative_payout
            ),
        )
        .tree_hash()
        .into();
        let mut initiate_payout_announcement: Vec<u8> = initiate_payout_announcement.to_vec();
        initiate_payout_announcement.insert(0, b'p');

        // spend mirror slot
        mirror_slot.spend(ctx, self.info.inner_puzzle_hash().into())?;

        // spend self
        let initiate_payout_action =
            DigRewardDistributorAction::InitiatePayout(DigInitiatePayoutActionSolution {
                mirror_payout_puzzle_hash: mirror_slot_value.payout_puzzle_hash,
                mirror_initial_cumulative_payout: mirror_slot_value.initial_cumulative_payout,
                mirror_shares: mirror_slot_value.shares,
            });

        let my_state = self.info.state;
        let my_inner_puzzle_hash = self.info.inner_puzzle_hash();

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(
            ctx,
            reserve.coin.parent_coin_info,
            vec![initiate_payout_action],
        )?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        // spend reserve
        reserve.spend_for_reserve_finalizer_controller(
            ctx,
            my_state,
            new_reserve.coin.amount,
            my_inner_puzzle_hash.into(),
            my_spend.solution,
        )?;

        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                my_coin.puzzle_hash,
                initiate_payout_announcement,
            )),
            new_dig_reward_distributor,
            new_reserve,
            new_slot,
            withdrawal_amount,
        ))
    }

    pub fn remove_mirror(
        self,
        ctx: &mut SpendContext,
        reserve: Reserve,
        mirror_slot: Slot<DigMirrorSlotValue>,
        validator_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, DigRewardDistributor, Reserve), DriverError> {
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
                vec![self.coin.puzzle_hash.to_clvm(&mut ctx.allocator)?],
            )
            .assert_concurrent_puzzle(mirror_slot.coin.puzzle_hash);

        // spend mirror slot
        mirror_slot.spend(ctx, self.info.inner_puzzle_hash().into())?;

        // spend self
        let remove_mirror_action =
            DigRewardDistributorAction::RemoveMirror(DigRemoveMirrorActionSolution {
                validator_singleton_inner_puzzle_hash,
                mirror_payout_puzzle_hash: mirror_slot_value.payout_puzzle_hash,
                mirror_shares: mirror_slot_value.shares,
            });

        let my_state = self.info.state;
        let my_inner_puzzle_hash = self.info.inner_puzzle_hash();

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(
            ctx,
            reserve.coin.parent_coin_info,
            vec![remove_mirror_action],
        )?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let (new_dig_reward_distributor, new_reserve) = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child DIG reward distributor".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        // spend reserve
        reserve.spend_for_reserve_finalizer_controller(
            ctx,
            my_state,
            new_reserve.coin.amount,
            my_inner_puzzle_hash.into(),
            my_spend.solution,
        )?;

        Ok((
            remove_mirror_conditions,
            new_dig_reward_distributor,
            new_reserve,
        ))
    }
}

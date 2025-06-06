use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::{SingletonSolution, SingletonStruct},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    driver::{DriverError, Layer, Puzzle, Spend, SpendContext},
    prelude::{Cat, CatSpend},
    types::run_puzzle,
};
use clvm_traits::{clvm_list, match_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    Action, ActionLayer, ActionLayerSolution, RawActionLayerSolution, Registry,
    ReserveFinalizerSolution, RewardDistributorAddEntryAction,
    RewardDistributorCommitIncentivesAction, RewardDistributorInitiatePayoutAction,
    RewardDistributorNewEpochAction, RewardDistributorRemoveEntryAction,
    RewardDistributorWithdrawIncentivesAction, Slot, SlotInfo, SlotProof,
};

use super::{
    Reserve, RewardDistributorCommitmentSlotValue, RewardDistributorConstants,
    RewardDistributorEntrySlotValue, RewardDistributorInfo, RewardDistributorRewardSlotValue,
    RewardDistributorSlotNonce, RewardDistributorState,
};

#[derive(Debug, Clone, Default)]
pub struct RewardDistributorPendingItems {
    pub pending_actions: Vec<Spend>,

    pub pending_spent_slots: Vec<(RewardDistributorSlotNonce, Bytes32)>, // (nonce, value hash)

    pub pending_reward_slot_values: Vec<RewardDistributorRewardSlotValue>,
    pub pending_commitment_slot_values: Vec<RewardDistributorCommitmentSlotValue>,
    pub pending_mirror_slot_values: Vec<RewardDistributorEntrySlotValue>,
}

#[derive(Debug, Clone)]
#[must_use]
pub struct RewardDistributor {
    pub coin: Coin,
    pub proof: Proof,
    pub info: RewardDistributorInfo,
    pub reserve: Reserve,

    pub pending_items: RewardDistributorPendingItems,
}

impl RewardDistributor {
    pub fn new(coin: Coin, proof: Proof, info: RewardDistributorInfo, reserve: Reserve) -> Self {
        Self {
            coin,
            proof,
            info,
            reserve,
            pending_items: RewardDistributorPendingItems::default(),
        }
    }
}

impl RewardDistributor {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: RewardDistributorConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) = RewardDistributorInfo::parse(allocator, parent_puzzle, constants)?
        else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let parent_solution = SingletonSolution::<NodePtr>::from_clvm(allocator, parent_solution)?;
        let new_state = ActionLayer::<RewardDistributorState>::get_new_state(
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
            SingletonStruct::new(parent_info.constants.launcher_id)
                .tree_hash()
                .into(),
            0,
            new_state.total_reserves,
        );

        Ok(Some(RewardDistributor {
            coin: new_coin,
            proof,
            info: new_info,
            reserve,
            pending_items: RewardDistributorPendingItems::default(),
        }))
    }
}

impl Registry for RewardDistributor {
    type State = RewardDistributorState;
    type Constants = RewardDistributorConstants;
}

impl RewardDistributor {
    pub fn finish_spend(
        self,
        ctx: &mut SpendContext,
        other_cat_spends: Vec<CatSpend>,
    ) -> Result<Self, DriverError> {
        let layers = self.info.into_layers(ctx)?;

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_puzzle_hashes = self
            .pending_items
            .pending_actions
            .iter()
            .map(|a| ctx.tree_hash(a.puzzle).into())
            .collect::<Vec<Bytes32>>();

        let finalizer_solution = ctx.alloc(&ReserveFinalizerSolution {
            reserve_parent_id: self.reserve.coin.parent_coin_info,
        })?;

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(
                            &RewardDistributorInfo::action_puzzle_hashes(&self.info.constants),
                            &action_puzzle_hashes,
                        )
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends: self.pending_items.pending_actions,
                    finalizer_solution,
                },
            },
        )?;

        let my_spend = Spend::new(puzzle, solution);
        ctx.spend(self.coin, my_spend)?;

        let cat_spend = self.reserve.cat_spend_for_reserve_finalizer_controller(
            ctx,
            self.info.state,
            self.info.inner_puzzle_hash().into(),
            solution,
        )?;

        let mut cat_spends = other_cat_spends;
        cat_spends.push(cat_spend);
        Cat::spend_all(ctx, &cat_spends)?;

        let my_puzzle = Puzzle::parse(ctx, my_spend.puzzle);
        let new_reward_distributor = RewardDistributor::from_parent_spend(
            ctx,
            self.coin,
            my_puzzle,
            solution,
            self.info.constants,
        )?
        .unwrap();

        Ok(new_reward_distributor)
    }

    pub fn insert(&mut self, action_spend: Spend) {
        self.pending_items.pending_actions.push(action_spend);
    }

    pub fn insert_multiple(&mut self, action_spends: Vec<Spend>) {
        self.pending_items.pending_actions.extend(action_spends);
    }

    pub fn new_action<A>(&self) -> A
    where
        A: Action<Self>,
    {
        A::from_constants(&self.info.constants)
    }

    pub fn created_slot_values_to_slots<SlotValue>(
        &self,
        slot_values: Vec<SlotValue>,
        nonce: RewardDistributorSlotNonce,
    ) -> Vec<Slot<SlotValue>>
    where
        SlotValue: Copy + ToTreeHash,
    {
        let proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        slot_values
            .into_iter()
            .map(|slot_value| {
                Slot::new(
                    proof,
                    SlotInfo::from_value(
                        self.info.constants.launcher_id,
                        nonce.to_u64(),
                        slot_value,
                    ),
                )
            })
            .collect()
    }

    pub fn get_latest_pending_state(
        &self,
        allocator: &mut Allocator,
    ) -> Result<RewardDistributorState, DriverError> {
        let mut state = (NodePtr::NIL, self.info.state);

        for action in self.pending_items.pending_actions.iter() {
            let actual_solution = clvm_list!(state, action.solution).to_clvm(allocator)?;

            let output = run_puzzle(allocator, action.puzzle, actual_solution)?;
            (state, _) = <match_tuple!((NodePtr, RewardDistributorState), NodePtr)>::from_clvm(
                allocator, output,
            )?;
        }

        Ok(state.1)
    }

    pub fn get_latest_pending_ephemeral_state(
        &self,
        allocator: &mut Allocator,
    ) -> Result<u64, DriverError> {
        let mut state = (0, self.info.state);

        for action in self.pending_items.pending_actions.iter() {
            let actual_solution = clvm_list!(state, action.solution).to_clvm(allocator)?;

            let output = run_puzzle(allocator, action.puzzle, actual_solution)?;
            (state, _) = <match_tuple!((u64, RewardDistributorState), NodePtr)>::from_clvm(
                allocator, output,
            )?;
        }

        Ok(state.0)
    }

    pub fn get_pending_items_from_spend(
        &self,
        ctx: &mut SpendContext,
        solution: NodePtr,
    ) -> Result<RewardDistributorPendingItems, DriverError> {
        let solution = ctx.extract::<SingletonSolution<NodePtr>>(solution)?;
        let inner_solution = ActionLayer::<RewardDistributorState, NodePtr>::parse_solution(
            ctx,
            solution.inner_solution,
        )?;

        let mut actions: Vec<Spend> = vec![];
        let mut reward_slot_values: Vec<RewardDistributorRewardSlotValue> = vec![];
        let mut commitment_slot_values: Vec<RewardDistributorCommitmentSlotValue> = vec![];
        let mut mirror_slot_values: Vec<RewardDistributorEntrySlotValue> = vec![];
        let mut spent_slots: Vec<(RewardDistributorSlotNonce, Bytes32)> = vec![];

        let new_epoch_action =
            RewardDistributorNewEpochAction::from_constants(&self.info.constants);
        let new_epoch_hash = new_epoch_action.tree_hash();

        let commit_incentives_action =
            RewardDistributorCommitIncentivesAction::from_constants(&self.info.constants);
        let commit_incentives_hash = commit_incentives_action.tree_hash();

        let add_entry_action =
            RewardDistributorAddEntryAction::from_constants(&self.info.constants);
        let add_entry_hash = add_entry_action.tree_hash();

        let remove_entry_action =
            RewardDistributorRemoveEntryAction::from_constants(&self.info.constants);
        let remove_entry_hash = remove_entry_action.tree_hash();

        let withdraw_incentives_action =
            RewardDistributorWithdrawIncentivesAction::from_constants(&self.info.constants);
        let withdraw_incentives_hash = withdraw_incentives_action.tree_hash();

        let initiate_payout_action =
            RewardDistributorInitiatePayoutAction::from_constants(&self.info.constants);
        let initiate_payout_hash = initiate_payout_action.tree_hash();

        let mut current_state = (NodePtr::NIL, self.info.state);
        for raw_action in inner_solution.action_spends {
            actions.push(Spend::new(raw_action.puzzle, raw_action.solution));

            let actual_solution = ctx.alloc(&clvm_list!(current_state, raw_action.solution))?;

            let action_output = run_puzzle(ctx, raw_action.puzzle, actual_solution)?;
            (current_state, _) = ctx
                .extract::<match_tuple!((NodePtr, RewardDistributorState), NodePtr)>(
                    action_output,
                )?;

            let raw_action_hash = ctx.tree_hash(raw_action.puzzle);

            if raw_action_hash == new_epoch_hash {
                let (rew, spent) =
                    new_epoch_action.get_slot_value_from_solution(ctx, raw_action.solution)?;

                reward_slot_values.push(rew);
                spent_slots.push(spent);
            } else if raw_action_hash == commit_incentives_hash {
                let (comm, rews, spent_slot) = commit_incentives_action
                    .get_slot_values_from_solution(
                        ctx,
                        self.info.constants.epoch_seconds,
                        raw_action.solution,
                    )?;

                commitment_slot_values.push(comm);
                reward_slot_values.extend(rews);
                spent_slots.push(spent_slot);
            } else if raw_action_hash == add_entry_hash {
                mirror_slot_values.push(add_entry_action.get_slot_value_from_solution(
                    ctx,
                    &current_state.1,
                    raw_action.solution,
                )?);
            } else if raw_action_hash == remove_entry_hash {
                spent_slots.push(
                    remove_entry_action
                        .get_spent_slot_value_from_solution(ctx, raw_action.solution)?,
                );
            } else if raw_action_hash == withdraw_incentives_hash {
                let (rew, spnt) = withdraw_incentives_action.get_slot_value_from_solution(
                    ctx,
                    &self.info.constants,
                    raw_action.solution,
                )?;

                reward_slot_values.push(rew);
                spent_slots.extend(spnt);
            } else if raw_action_hash == initiate_payout_hash {
                let (mirr, spent) = initiate_payout_action.get_slot_value_from_solution(
                    ctx,
                    &current_state.1,
                    raw_action.solution,
                )?;

                mirror_slot_values.push(mirr);
                spent_slots.push(spent);
            }
        }

        Ok(RewardDistributorPendingItems {
            pending_actions: actions,
            pending_spent_slots: spent_slots,
            pending_reward_slot_values: reward_slot_values,
            pending_commitment_slot_values: commitment_slot_values,
            pending_mirror_slot_values: mirror_slot_values,
        })
    }
}

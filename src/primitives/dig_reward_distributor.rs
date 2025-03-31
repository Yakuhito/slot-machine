use chia::{
    clvm_utils::{tree_hash, ToTreeHash},
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::{SingletonSolution, SingletonStruct},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{run_puzzle, Cat, CatSpend, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{clvm_list, match_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    Action, ActionLayer, ActionLayerSolution, DigAddMirrorAction, DigCommitIncentivesAction,
    DigInitiatePayoutAction, DigNewEpochAction, DigRemoveMirrorAction, DigWithdrawIncentivesAction,
    RawActionLayerSolution, Registry, ReserveFinalizerSolution, Slot, SlotInfo, SlotProof,
};

use super::{
    CatalogSlotValue, DigCommitmentSlotValue, DigMirrorSlotValue, DigRewardDistributorConstants,
    DigRewardDistributorInfo, DigRewardDistributorState, DigRewardSlotValue, DigSlotNonce, Reserve,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct DigRewardDistributor {
    pub coin: Coin,
    pub proof: Proof,
    pub info: DigRewardDistributorInfo,
    pub reserve: Reserve,

    pub pending_actions: Vec<Spend>,
    pub pending_spent_slots: Vec<(DigSlotNonce, Bytes32)>, // (nonce, value hash)
    pub pending_reward_slot_values: Vec<DigRewardSlotValue>,
    pub pending_commitment_slot_values: Vec<DigCommitmentSlotValue>,
    pub pending_mirror_slot_values: Vec<DigMirrorSlotValue>,
}

impl DigRewardDistributor {
    pub fn new(coin: Coin, proof: Proof, info: DigRewardDistributorInfo, reserve: Reserve) -> Self {
        Self {
            coin,
            proof,
            info,
            reserve,
            pending_actions: Vec::new(),
            pending_spent_slots: Vec::new(),
            pending_reward_slot_values: Vec::new(),
            pending_commitment_slot_values: Vec::new(),
            pending_mirror_slot_values: Vec::new(),
        }
    }
}

impl DigRewardDistributor {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: DigRewardDistributorConstants,
    ) -> Result<Option<Self>, DriverError>
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
            SingletonStruct::new(parent_info.constants.launcher_id)
                .tree_hash()
                .into(),
            0,
            new_state.total_reserves,
        );

        Ok(Some(DigRewardDistributor {
            coin: new_coin,
            proof,
            info: new_info,
            reserve,
            pending_actions: Vec::new(),
            pending_spent_slots: Vec::new(),
            pending_reward_slot_values: Vec::new(),
            pending_commitment_slot_values: Vec::new(),
            pending_mirror_slot_values: Vec::new(),
        }))
    }
}

impl Registry for DigRewardDistributor {
    type State = DigRewardDistributorState;
    type Constants = DigRewardDistributorConstants;
}

impl DigRewardDistributor {
    pub fn finish_spend(
        self,
        ctx: &mut SpendContext,
        other_cat_spends: Vec<CatSpend>,
    ) -> Result<Self, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_puzzle_hashes = self
            .pending_actions
            .iter()
            .map(|a| ctx.tree_hash(a.puzzle).into())
            .collect::<Vec<Bytes32>>();

        let finalizer_solution = ReserveFinalizerSolution {
            reserve_parent_id: self.reserve.coin.parent_coin_info,
        }
        .to_clvm(&mut ctx.allocator)?;

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(
                            &DigRewardDistributorInfo::action_puzzle_hashes(&self.info.constants),
                            &action_puzzle_hashes,
                        )
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends: self.pending_actions,
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

        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_reward_distributor = DigRewardDistributor::from_parent_spend(
            &mut ctx.allocator,
            self.coin,
            my_puzzle,
            solution,
            self.info.constants,
        )?
        .unwrap();

        Ok(new_reward_distributor)
    }

    pub fn insert(&mut self, action_spend: Spend) {
        self.pending_actions.push(action_spend);
    }

    pub fn insert_multiple(&mut self, action_spends: Vec<Spend>) {
        self.pending_actions.extend(action_spends);
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
        nonce: DigSlotNonce,
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
    ) -> Result<DigRewardDistributorState, DriverError> {
        let mut state = self.info.state;

        for action in self.pending_actions.iter() {
            let actual_solution = clvm_list!(state, action.solution).to_clvm(allocator)?;

            let output = run_puzzle(allocator, action.puzzle, actual_solution)?;
            (state, _) =
                <match_tuple!(DigRewardDistributorState, NodePtr)>::from_clvm(allocator, output)?;
        }

        Ok(state)
    }

    pub fn get_new_slots_from_spend(
        &self,
        ctx: &mut SpendContext,
        solution: NodePtr,
    ) -> Result<Vec<Slot<CatalogSlotValue>>, DriverError> {
        let solution =
            SingletonSolution::<RawActionLayerSolution<NodePtr, NodePtr, NodePtr>>::from_clvm(
                &ctx.allocator,
                solution,
            )?;

        let mut reward_slot_values: Vec<DigRewardSlotValue> = vec![];
        let mut commitment_slot_values: Vec<DigCommitmentSlotValue> = vec![];
        let mut mirror_slot_values: Vec<DigMirrorSlotValue> = vec![];
        let mut spent_slots: Vec<(DigSlotNonce, Bytes32)> = vec![];

        let new_epoch_action = DigNewEpochAction::from_constants(&self.info.constants);
        let new_epoch_hash = new_epoch_action.tree_hash();

        let commit_incentives_action =
            DigCommitIncentivesAction::from_constants(&self.info.constants);
        let commit_incentives_hash = commit_incentives_action.tree_hash();

        let add_mirror_action = DigAddMirrorAction::from_constants(&self.info.constants);
        let add_mirror_hash = add_mirror_action.tree_hash();

        let remove_mirror_action = DigRemoveMirrorAction::from_constants(&self.info.constants);
        let remove_mirror_hash = remove_mirror_action.tree_hash();

        let withdraw_incentives_action =
            DigWithdrawIncentivesAction::from_constants(&self.info.constants);
        let withdraw_incentives_hash = withdraw_incentives_action.tree_hash();

        let initiate_payout_action = DigInitiatePayoutAction::from_constants(&self.info.constants);
        let initiate_payout_hash = initiate_payout_action.tree_hash();

        let mut current_state = self.info.state;
        for raw_action in solution.inner_solution.actions {
            let actual_solution = clvm_list!(current_state, raw_action.action_solution)
                .to_clvm(&mut ctx.allocator)?;

            let action_output = run_puzzle(
                &mut ctx.allocator,
                raw_action.action_puzzle_reveal,
                actual_solution,
            )?;
            (current_state, _) = <match_tuple!(DigRewardDistributorState, NodePtr)>::from_clvm(
                &ctx.allocator,
                action_output,
            )?;

            let raw_action_hash = tree_hash(&ctx.allocator, raw_action.action_puzzle_reveal);

            if raw_action_hash == new_epoch_hash {
                reward_slot_values.push(
                    new_epoch_action
                        .get_slot_value_from_solution(ctx, raw_action.action_solution)?,
                );
            } else if raw_action_hash == commit_incentives_hash {
                let (comm, rews, spent_slot) = commit_incentives_action
                    .get_slot_values_from_solution(
                        ctx,
                        self.info.constants.epoch_seconds,
                        raw_action.action_solution,
                    )?;

                commitment_slot_values.push(comm);
                reward_slot_values.extend(rews);
                spent_slots.push(spent_slot);
            } else if raw_action_hash == add_mirror_hash {
                mirror_slot_values.push(add_mirror_action.get_slot_value_from_solution(
                    ctx,
                    &current_state,
                    raw_action.action_solution,
                )?);
            } else if raw_action_hash == remove_mirror_hash {
                todo!("Mark mirror slot as removed")
            } else if raw_action_hash == withdraw_incentives_hash {
                let reward_slot = withdraw_incentives_action.get_slot_value_from_solution(
                    ctx,
                    &self.info.constants,
                    raw_action.action_solution,
                )?;

                reward_slot_values.push(reward_slot);
                todo!("Withdraw incentives also spends the commitment slot")
            } else if raw_action_hash == initiate_payout_hash {
                let mirror_slot = initiate_payout_action.get_slot_value_from_solution(
                    ctx,
                    &current_state,
                    raw_action.action_solution,
                )?;

                mirror_slot_values.push(mirror_slot);
            }
        }

        // Ok(self.created_slot_values_to_slots(slot_infos))
        todo!("return the slots");
    }
}

// pub fn add_pending_slots(&mut self, slots: Vec<Slot<CatalogSlotValue>>) {
//     for slot in slots {
//         self.pending_slots
//             .retain(|s| s.info.value.asset_id != slot.info.value.asset_id);
//         self.pending_slots.push(slot);
//     }
// }

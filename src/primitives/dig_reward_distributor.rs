use chia::{
    clvm_utils::ToTreeHash,
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
    Action, ActionLayer, ActionLayerSolution, RawActionLayerSolution, Registry,
    ReserveFinalizerSolution, Slot, SlotInfo, SlotProof,
};

use super::{
    DigRewardDistributorConstants, DigRewardDistributorInfo, DigRewardDistributorState,
    DigSlotNonce, Reserve,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct DigRewardDistributor {
    pub coin: Coin,
    pub proof: Proof,
    pub info: DigRewardDistributorInfo,
    pub reserve: Reserve,

    pub pending_actions: Vec<Spend>,
}

impl DigRewardDistributor {
    pub fn new(coin: Coin, proof: Proof, info: DigRewardDistributorInfo, reserve: Reserve) -> Self {
        Self {
            coin,
            proof,
            info,
            reserve,
            pending_actions: Vec::new(),
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
            SingletonStruct::new(parent_info.launcher_id)
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
                            &DigRewardDistributorInfo::action_puzzle_hashes(
                                self.info.launcher_id,
                                &self.info.constants,
                            ),
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
        A::from_constants(self.info.launcher_id, &self.info.constants)
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
                    SlotInfo::from_value(self.info.launcher_id, slot_value, Some(nonce.to_u64())),
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
}

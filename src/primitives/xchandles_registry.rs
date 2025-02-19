use chia::{
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_wallet_sdk::{run_puzzle, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{clvm_list, match_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{Action, ActionLayer, ActionLayerSolution, Registry};

use super::{
    Slot, SlotInfo, SlotProof, XchandlesConstants, XchandlesRegistryInfo, XchandlesRegistryState,
    XchandlesSlotValue,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct XchandlesRegistry {
    pub coin: Coin,
    pub proof: Proof,
    pub info: XchandlesRegistryInfo,

    pub pending_actions: Vec<Spend>,
}

impl XchandlesRegistry {
    pub fn new(coin: Coin, proof: Proof, info: XchandlesRegistryInfo) -> Self {
        Self {
            coin,
            proof,
            info,
            pending_actions: vec![],
        }
    }
}

impl Registry for XchandlesRegistry {
    type State = XchandlesRegistryState;
    type Constants = XchandlesConstants;
}

impl XchandlesRegistry {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: XchandlesConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) = XchandlesRegistryInfo::parse(allocator, parent_puzzle, constants)?
        else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let parent_solution = SingletonSolution::<NodePtr>::from_clvm(allocator, parent_solution)?;
        let new_state = ActionLayer::<XchandlesRegistryState>::get_new_state(
            allocator,
            parent_info.state,
            parent_solution.inner_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        Ok(Some(XchandlesRegistry {
            coin: new_coin,
            proof,
            info: new_info,
            pending_actions: vec![],
        }))
    }
}

impl XchandlesRegistry {
    pub fn spend(self, ctx: &mut SpendContext) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_puzzle_hashes = self
            .pending_actions
            .iter()
            .map(|a| ctx.tree_hash(a.puzzle).into())
            .collect::<Vec<Bytes32>>();

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(
                            &XchandlesRegistryInfo::action_puzzle_hashes(
                                self.info.launcher_id,
                                &self.info.constants,
                            ),
                            &action_puzzle_hashes,
                        )
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends: self.pending_actions,
                    finalizer_solution: NodePtr::NIL,
                },
            },
        )?;

        Ok(Spend::new(puzzle, solution))
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

    pub fn created_slot_values_to_slots(
        &self,
        slot_values: Vec<XchandlesSlotValue>,
    ) -> Vec<Slot<XchandlesSlotValue>> {
        let proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        slot_values
            .into_iter()
            .map(|slot_value| {
                Slot::new(
                    proof,
                    SlotInfo::from_value(self.info.launcher_id, slot_value, None),
                )
            })
            .collect()
    }

    pub fn get_latest_pending_state(
        &self,
        allocator: &mut Allocator,
    ) -> Result<XchandlesRegistryState, DriverError> {
        let mut state = self.info.state;

        for action in self.pending_actions.iter() {
            let actual_solution = clvm_list!(state, action.solution).to_clvm(allocator)?;

            let output = run_puzzle(allocator, action.puzzle, actual_solution)?;
            (state, _) =
                <match_tuple!(XchandlesRegistryState, NodePtr)>::from_clvm(allocator, output)?;
        }

        Ok(state)
    }
}

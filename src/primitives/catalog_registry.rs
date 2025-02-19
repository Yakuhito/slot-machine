use chia::{
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::FromClvm;
use clvmr::{Allocator, NodePtr};

use crate::{Action, ActionLayer, ActionLayerSolution, Registry};

use super::{
    CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState, CatalogSlotValue, Slot,
    SlotInfo, SlotProof,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct CatalogRegistry {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogRegistryInfo,

    pub pending_actions: Vec<Spend>,
}

impl CatalogRegistry {
    pub fn new(coin: Coin, proof: Proof, info: CatalogRegistryInfo) -> Self {
        Self {
            coin,
            proof,
            info,
            pending_actions: vec![],
        }
    }
}

impl CatalogRegistry {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: CatalogRegistryConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) = CatalogRegistryInfo::parse(allocator, parent_puzzle, constants)?
        else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let parent_solution = SingletonSolution::<NodePtr>::from_clvm(allocator, parent_solution)?;
        let new_state = ActionLayer::<CatalogRegistryState>::get_new_state(
            allocator,
            parent_info.state,
            parent_solution.inner_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        Ok(Some(CatalogRegistry {
            coin: new_coin,
            proof,
            info: new_info,
            pending_actions: vec![],
        }))
    }
}

impl Registry for CatalogRegistry {
    type State = CatalogRegistryState;
    type Constants = CatalogRegistryConstants;
}

impl CatalogRegistry {
    pub fn finish_spend(self, ctx: &mut SpendContext) -> Result<Self, DriverError> {
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
                            &CatalogRegistryInfo::action_puzzle_hashes(
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

        let my_spend = Spend::new(puzzle, solution);
        ctx.spend(self.coin, my_spend)?;

        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_self = CatalogRegistry::from_parent_spend(
            &mut ctx.allocator,
            self.coin,
            my_puzzle,
            my_spend.solution,
            self.info.constants,
        )?
        .ok_or(DriverError::Custom(
            "Couldn't parse child registry".to_string(),
        ))?;

        Ok(new_self)
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
        slot_values: Vec<CatalogSlotValue>,
    ) -> Vec<Slot<CatalogSlotValue>> {
        let proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        slot_values
            .into_iter()
            .map(|slot_value| {
                Slot::new(
                    proof,
                    SlotInfo::from_value(self.info.launcher_id, 0, slot_value),
                )
            })
            .collect()
    }
}

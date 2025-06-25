use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_wallet_sdk::driver::{DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{clvm_list, match_tuple, FromClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    Action, ActionLayer, ActionLayerSolution, CatalogRefundAction, CatalogRegisterAction,
    DelegatedStateAction, Registry,
};

use super::{
    CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState, CatalogSlotValue, Slot,
    SlotInfo, SlotProof,
};

#[derive(Debug, Clone)]
pub struct CatalogPendingSpendInfo {
    pub actions: Vec<Spend>,
    pub created_slots: Vec<CatalogSlotValue>,
    pub spent_slots: Vec<CatalogSlotValue>,

    pub latest_state: (NodePtr, CatalogRegistryState),
}

impl CatalogPendingSpendInfo {
    pub fn new(latest_state: CatalogRegistryState) -> Self {
        Self {
            actions: vec![],
            created_slots: vec![],
            spent_slots: vec![],
            latest_state: (NodePtr::NIL, latest_state),
        }
    }
}

#[derive(Debug, Clone)]
#[must_use]
pub struct CatalogRegistry {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogRegistryInfo,

    pub pending_spend: CatalogPendingSpendInfo,
}

impl CatalogRegistry {
    pub fn new(coin: Coin, proof: Proof, info: CatalogRegistryInfo) -> Self {
        Self {
            coin,
            proof,
            info,
            pending_spend: CatalogPendingSpendInfo::new(info.state),
        }
    }
}

impl CatalogRegistry {
    pub fn pending_info_delta_from_spend(
        ctx: &mut SpendContext,
        action_spend: Spend,
        current_state_and_ephemeral: (NodePtr, CatalogRegistryState),
        constants: CatalogRegistryConstants,
    ) -> Result<
        (
            (NodePtr, CatalogRegistryState),
            Vec<CatalogSlotValue>, // created slot values
            Vec<CatalogSlotValue>, // spent slot values
        ),
        DriverError,
    > {
        let mut created_slots = vec![];
        let mut spent_slots = vec![];

        let register_action = CatalogRegisterAction::from_constants(&constants);
        let register_hash = register_action.tree_hash();

        let refund_action = CatalogRefundAction::from_constants(&constants);
        let refund_hash = refund_action.tree_hash();

        let delegated_state_action =
            <DelegatedStateAction as Action<CatalogRegistry>>::from_constants(&constants);
        let delegated_state_hash = delegated_state_action.tree_hash();

        let actual_solution = ctx.alloc(&clvm_list!(
            current_state_and_ephemeral,
            action_spend.solution
        ))?;

        let output = ctx.run(action_spend.puzzle, actual_solution)?;
        let (new_state_and_ephemeral, _) =
            ctx.extract::<match_tuple!((NodePtr, CatalogRegistryState), NodePtr)>(output)?;

        let raw_action_hash = ctx.tree_hash(action_spend.puzzle);

        if raw_action_hash == register_hash {
            spent_slots.extend(
                register_action.get_spent_slot_values_from_solution(ctx, action_spend.solution)?,
            );

            created_slots.extend(
                register_action
                    .get_created_slot_values_from_solution(ctx, action_spend.solution)?,
            );
        } else if raw_action_hash == refund_hash {
            if let Some(spent_slot) =
                refund_action.spent_slot_value_from_solution(ctx, action_spend.solution)?
            {
                spent_slots.push(spent_slot);
            }
        } else if raw_action_hash != delegated_state_hash {
            // delegated state action has no effect on slots
            return Err(DriverError::InvalidMerkleProof);
        }

        Ok((new_state_and_ephemeral, created_slots, spent_slots))
    }

    pub fn pending_info_from_spend(
        ctx: &mut SpendContext,
        solution: NodePtr,
        initial_state: CatalogRegistryState,
        constants: CatalogRegistryConstants,
    ) -> Result<CatalogPendingSpendInfo, DriverError> {
        let solution = ctx.extract::<SingletonSolution<NodePtr>>(solution)?;

        let mut created_slots = vec![];
        let mut spent_slots = vec![];

        let mut state_incl_ephemeral: (NodePtr, CatalogRegistryState) =
            (NodePtr::NIL, initial_state);

        let inner_solution = ActionLayer::<CatalogRegistryState, NodePtr>::parse_solution(
            ctx,
            solution.inner_solution,
        )?;

        for raw_action in inner_solution.action_spends.iter() {
            let res = Self::pending_info_delta_from_spend(
                ctx,
                *raw_action,
                state_incl_ephemeral,
                constants,
            )?;

            state_incl_ephemeral = res.0;
            created_slots.extend(res.1);
            spent_slots.extend(res.2);
        }

        Ok(CatalogPendingSpendInfo {
            actions: inner_solution.action_spends,
            created_slots,
            spent_slots,
            latest_state: state_incl_ephemeral,
        })
    }

    pub fn from_spend(
        ctx: &mut SpendContext,
        spend: CoinSpend,
        constants: CatalogRegistryConstants,
    ) -> Result<Option<Self>, DriverError> {
        let coin = spend.coin;
        let puzzle_ptr = ctx.alloc(&spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = ctx.alloc(&spend.solution)?;

        let Some(info) = CatalogRegistryInfo::parse(ctx, puzzle, constants)? else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: coin.parent_coin_info,
            parent_inner_puzzle_hash: info.inner_puzzle_hash().into(),
            parent_amount: coin.amount,
        });

        let pending_spend =
            Self::pending_info_from_spend(ctx, solution_ptr, info.state, constants)?;

        Ok(Some(CatalogRegistry {
            coin,
            proof,
            info,
            pending_spend,
        }))
    }

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
            pending_slots: vec![],
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
                            &CatalogRegistryInfo::action_puzzle_hashes(&self.info.constants),
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

        let my_puzzle = Puzzle::parse(ctx, my_spend.puzzle);
        let new_self = CatalogRegistry::from_parent_spend(
            ctx,
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
        A::from_constants(&self.info.constants)
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
                    SlotInfo::from_value(self.info.constants.launcher_id, 0, slot_value),
                )
            })
            .collect()
    }

    pub fn add_pending_slots(&mut self, slots: Vec<Slot<CatalogSlotValue>>) {
        for slot in slots {
            self.pending_slots
                .retain(|s| s.info.value.asset_id != slot.info.value.asset_id);
            self.pending_slots.push(slot);
        }
    }

    pub fn actual_neigbors(
        &self,
        new_tail_hash: Bytes32,
        on_chain_left_slot: Slot<CatalogSlotValue>,
        on_chain_right_slot: Slot<CatalogSlotValue>,
    ) -> (Slot<CatalogSlotValue>, Slot<CatalogSlotValue>) {
        let mut left = on_chain_left_slot;
        let mut right = on_chain_right_slot;

        let new_slot_value =
            CatalogSlotValue::new(new_tail_hash, Bytes32::default(), Bytes32::default());

        for slot in self.pending_slots.iter() {
            if slot.info.value < new_slot_value && slot.info.value >= left.info.value {
                left = slot.clone();
            }

            if slot.info.value > new_slot_value && slot.info.value <= right.info.value {
                right = slot.clone();
            }
        }

        (left, right)
    }
}

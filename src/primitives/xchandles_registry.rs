use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes, Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_puzzle_types::singleton::{LauncherSolution, SingletonArgs};
use chia_wallet_sdk::{
    driver::{DriverError, Layer, Puzzle, Spend, SpendContext},
    types::run_puzzle,
};
use clvm_traits::{clvm_list, match_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    eve_singleton_inner_puzzle, Action, ActionLayer, ActionLayerSolution, CliError,
    DelegatedStateAction, Registry, XchandlesExpireAction, XchandlesExtendAction,
    XchandlesOracleAction, XchandlesRefundAction, XchandlesRegisterAction, XchandlesUpdateAction,
};

use super::{
    Slot, SlotInfo, SlotProof, XchandlesConstants, XchandlesRegistryInfo, XchandlesRegistryState,
    XchandlesSlotValue,
};

#[derive(Debug, Clone, Default)]
pub struct XchandlesRegistryPendingItems {
    pub actions: Vec<Spend>,

    pub spent_slots: Vec<XchandlesSlotValue>,
    pub created_slots: Vec<XchandlesSlotValue>,
}

#[derive(Debug, Clone)]
#[must_use]
pub struct XchandlesRegistry {
    pub coin: Coin,
    pub proof: Proof,
    pub info: XchandlesRegistryInfo,

    pub pending_items: XchandlesRegistryPendingItems,
}

impl XchandlesRegistry {
    pub fn new(coin: Coin, proof: Proof, info: XchandlesRegistryInfo) -> Self {
        Self {
            coin,
            proof,
            info,
            pending_items: XchandlesRegistryPendingItems::default(),
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
        println!("before new state"); // todo: debug
        let new_state = ActionLayer::<XchandlesRegistryState>::get_new_state(
            allocator,
            parent_info.state,
            parent_solution.inner_solution,
        )?;
        println!("after new state"); // todo: debug

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        Ok(Some(XchandlesRegistry {
            coin: new_coin,
            proof,
            info: new_info,
            pending_items: XchandlesRegistryPendingItems::default(),
        }))
    }

    // Also returns initial registration asset id
    #[allow(clippy::type_complexity)]
    pub fn from_launcher_solution(
        ctx: &mut SpendContext,
        launcher_coin: Coin,
        launcher_solution: NodePtr,
    ) -> Result<Option<(Self, [Slot<XchandlesSlotValue>; 2], Bytes32, u64)>, DriverError>
    where
        Self: Sized,
    {
        let Ok(launcher_solution) = ctx.extract::<LauncherSolution<(
            Bytes32,
            (
                u64,
                (u64, (XchandlesRegistryState, (XchandlesConstants, ()))),
            ),
        )>>(launcher_solution) else {
            return Ok(None);
        };

        let launcher_id = launcher_coin.coin_id();
        let (
            initial_registration_asset_id,
            (initial_base_price, (initial_registration_period, (initial_state, (constants, ())))),
        ) = launcher_solution.key_value_list;

        let info = XchandlesRegistryInfo::new(
            initial_state,
            constants.with_launcher_id(launcher_coin.coin_id()),
        );
        if info.state
            != XchandlesRegistryState::from(
                initial_registration_asset_id.tree_hash().into(),
                initial_base_price,
                initial_registration_period,
            )
        {
            return Ok(None);
        }

        let registry_inner_puzzle_hash: Bytes32 = info.inner_puzzle_hash().into();
        let eve_singleton_inner_puzzle = eve_singleton_inner_puzzle(
            ctx,
            launcher_id,
            XchandlesSlotValue::initial_left_end(),
            XchandlesSlotValue::initial_right_end(),
            NodePtr::NIL,
            registry_inner_puzzle_hash,
        )?;
        let eve_singleton_inner_puzzle_hash = ctx.tree_hash(eve_singleton_inner_puzzle);

        let eve_coin = Coin::new(
            launcher_id,
            SingletonArgs::curry_tree_hash(launcher_id, eve_singleton_inner_puzzle_hash).into(),
            1,
        );
        let registry_coin = Coin::new(
            eve_coin.coin_id(),
            SingletonArgs::curry_tree_hash(launcher_id, registry_inner_puzzle_hash.into()).into(),
            1,
        );

        if eve_coin.puzzle_hash != launcher_solution.singleton_puzzle_hash {
            return Ok(None);
        }

        // proof for registry, which is created by eve singleton
        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: eve_coin.parent_coin_info,
            parent_inner_puzzle_hash: eve_singleton_inner_puzzle_hash.into(),
            parent_amount: eve_coin.amount,
        });

        let slot_proof = SlotProof {
            parent_parent_info: eve_coin.parent_coin_info,
            parent_inner_puzzle_hash: eve_singleton_inner_puzzle_hash.into(),
        };
        let slots = [
            Slot::new(
                slot_proof,
                SlotInfo::from_value(launcher_id, 0, XchandlesSlotValue::initial_left_end()),
            ),
            Slot::new(
                slot_proof,
                SlotInfo::from_value(launcher_id, 0, XchandlesSlotValue::initial_right_end()),
            ),
        ];

        Ok(Some((
            XchandlesRegistry {
                coin: registry_coin,
                proof,
                info,
                pending_items: XchandlesRegistryPendingItems::default(),
            },
            slots,
            initial_registration_asset_id,
            initial_base_price,
        )))
    }
}

impl XchandlesRegistry {
    pub fn finish_spend(self, ctx: &mut SpendContext) -> Result<Self, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_puzzle_hashes = self
            .pending_items
            .actions
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
                            &XchandlesRegistryInfo::action_puzzle_hashes(&self.info.constants),
                            &action_puzzle_hashes,
                        )
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends: self.pending_items.actions,
                    finalizer_solution: NodePtr::NIL,
                },
            },
        )?;

        let my_spend = Spend::new(puzzle, solution);
        ctx.spend(self.coin, my_spend)?;

        let my_puzzle = Puzzle::parse(ctx, my_spend.puzzle);
        let new_self = XchandlesRegistry::from_parent_spend(
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
        self.pending_items.actions.push(action_spend);
    }

    pub fn insert_multiple(&mut self, action_spends: Vec<Spend>) {
        self.pending_items.actions.extend(action_spends);
    }

    pub fn new_action<A>(&self) -> A
    where
        A: Action<Self>,
    {
        A::from_constants(&self.info.constants)
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
                    SlotInfo::from_value(self.info.constants.launcher_id, 0, slot_value),
                )
            })
            .collect()
    }

    pub fn get_latest_pending_state(
        &self,
        allocator: &mut Allocator,
    ) -> Result<XchandlesRegistryState, DriverError> {
        let mut state = (NodePtr::NIL, self.info.state);

        for action in self.pending_items.actions.iter() {
            let actual_solution = clvm_list!(state, action.solution).to_clvm(allocator)?;

            let output = run_puzzle(allocator, action.puzzle, actual_solution)?;
            (state, _) = <match_tuple!((NodePtr, XchandlesRegistryState), NodePtr)>::from_clvm(
                allocator, output,
            )?;
        }

        Ok(state.1)
    }

    pub async fn get_pending_items_from_spend(
        &self,
        ctx: &mut SpendContext,
        solution: NodePtr,
    ) -> Result<XchandlesRegistryPendingItems, CliError> {
        let solution = ctx.extract::<SingletonSolution<NodePtr>>(solution)?;
        let inner_solution = ActionLayer::<XchandlesRegistryState, NodePtr>::parse_solution(
            ctx,
            solution.inner_solution,
        )?;

        let mut actions: Vec<Spend> = vec![];
        let mut spent_slots: Vec<XchandlesSlotValue> = vec![];
        let mut created_slots: Vec<XchandlesSlotValue> = vec![];

        let expire_action = XchandlesExpireAction::from_constants(&self.info.constants);
        let expire_action_hash = expire_action.tree_hash();

        let extend_action = XchandlesExtendAction::from_constants(&self.info.constants);
        let extend_action_hash = extend_action.tree_hash();

        let oracle_action = XchandlesOracleAction::from_constants(&self.info.constants);
        let oracle_action_hash = oracle_action.tree_hash();

        let register_action = XchandlesRegisterAction::from_constants(&self.info.constants);
        let register_action_hash = register_action.tree_hash();

        let update_action = XchandlesUpdateAction::from_constants(&self.info.constants);
        let update_action_hash = update_action.tree_hash();

        let refund_action = XchandlesRefundAction::from_constants(&self.info.constants);
        let refund_action_hash = refund_action.tree_hash();

        let delegated_state_action =
            <DelegatedStateAction as Action<XchandlesRegistry>>::from_constants(
                &self.info.constants,
            );
        let delegated_state_action_hash = delegated_state_action.tree_hash();

        let mut current_state = (NodePtr::NIL, self.info.state);
        for raw_action in inner_solution.action_spends {
            actions.push(Spend::new(raw_action.puzzle, raw_action.solution));

            let actual_solution = ctx.alloc(&clvm_list!(current_state, raw_action.solution))?;

            let action_output =
                run_puzzle(ctx, raw_action.puzzle, actual_solution).map_err(DriverError::from)?;
            (current_state, _) = ctx
                .extract::<match_tuple!((NodePtr, XchandlesRegistryState), NodePtr)>(
                    action_output,
                )?;

            let raw_action_hash = ctx.tree_hash(raw_action.puzzle);

            if raw_action_hash == delegated_state_action_hash {
                // slots were not created or spent
                continue;
            }

            if raw_action_hash == extend_action_hash {
                let spent_slot_value = XchandlesExtendAction::get_spent_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                let new_slot_value = XchandlesExtendAction::get_created_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                spent_slots.push(spent_slot_value);
                created_slots.push(new_slot_value);
            } else if raw_action_hash == oracle_action_hash {
                let spent_slot_value = XchandlesOracleAction::get_spent_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                spent_slots.push(spent_slot_value.clone());
                created_slots.push(spent_slot_value);
            } else if raw_action_hash == update_action_hash {
                let spent_slot_value = XchandlesUpdateAction::get_spent_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                let new_slot_value = XchandlesUpdateAction::get_created_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                spent_slots.push(spent_slot_value);
                created_slots.push(new_slot_value);
            } else if raw_action_hash == refund_action_hash {
                let Some(spent_slot_value) =
                    XchandlesRefundAction::get_spent_slot_value_from_solution(
                        ctx,
                        raw_action.solution,
                    )?
                else {
                    continue;
                };

                spent_slots.push(spent_slot_value.clone());
                created_slots.push(spent_slot_value);
            } else if raw_action_hash == expire_action_hash {
                let spent_slot_value = XchandlesExpireAction::get_spent_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                let new_slot_value = XchandlesExpireAction::get_created_slot_value_from_solution(
                    ctx,
                    raw_action.solution,
                )?;

                spent_slots.push(spent_slot_value);
                created_slots.push(new_slot_value);
            } else if raw_action_hash == register_action_hash {
                // register
                let spent_slot_values =
                    XchandlesRegisterAction::get_spent_slot_values_from_solution(
                        ctx,
                        raw_action.solution,
                    )?;

                let new_slot_values =
                    XchandlesRegisterAction::get_created_slot_values_from_solution(
                        ctx,
                        raw_action.solution,
                    )?;

                spent_slots.extend(spent_slot_values);
                created_slots.extend(new_slot_values);
            } else {
                return Err(CliError::Custom("Unknown action".to_string()));
            }
        }

        Ok(XchandlesRegistryPendingItems {
            actions,
            spent_slots,
            created_slots,
        })
    }

    pub fn actual_neigbors(
        &self,
        new_handle_hash: Bytes32,
        on_chain_left_slot: Slot<XchandlesSlotValue>,
        on_chain_right_slot: Slot<XchandlesSlotValue>,
    ) -> (Slot<XchandlesSlotValue>, Slot<XchandlesSlotValue>) {
        let mut left = on_chain_left_slot;
        let mut right = on_chain_right_slot;

        let new_slot_value = XchandlesSlotValue::new(
            new_handle_hash,
            Bytes32::default(),
            Bytes32::default(),
            0,
            Bytes32::default(),
            Bytes::default(),
        );

        for slot_value in self.pending_items.created_slots.iter() {
            if slot_value.handle_hash < new_slot_value.handle_hash
                && slot_value.handle_hash >= left.info.value.handle_hash
            {
                left = self
                    .created_slot_values_to_slots(vec![slot_value.clone()])
                    .remove(0);
            }

            if slot_value.handle_hash > new_slot_value.handle_hash
                && slot_value.handle_hash <= right.info.value.handle_hash
            {
                right = self
                    .created_slot_values_to_slots(vec![slot_value.clone()])
                    .remove(0);
            }
        }

        (left, right)
    }
}

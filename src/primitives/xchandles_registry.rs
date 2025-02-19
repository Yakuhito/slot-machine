use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        offer::{NotarizedPayment, Payment},
        singleton::SingletonSolution,
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    announcement_id, run_puzzle, Conditions, DriverError, Layer, Puzzle, Spend, SpendContext,
};
use clvm_traits::{clvm_list, clvm_tuple, match_tuple, FromClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    Action, ActionLayer, ActionLayerSolution, Registry, XchandlesExpireAction,
    XchandlesExpireActionSolution, XchandlesExponentialPremiumRenewPuzzleSolution,
    XchandlesExtendAction, XchandlesExtendActionSolution, XchandlesFactorPricingPuzzleArgs,
    XchandlesFactorPricingSolution, XchandlesOracleAction, XchandlesOracleActionSolution,
    XchandlesPrecommitValue, XchandlesRefundAction, XchandlesRefundActionSolution,
    XchandlesRegisterAction, XchandlesRegisterActionSolution, XchandlesUpdateAction,
    XchandlesUpdateActionSolution,
};

use super::{
    DefaultCatMakerArgs, PrecommitCoin, Slot, SlotInfo, SlotProof, XchandlesConstants,
    XchandlesRegistryInfo, XchandlesRegistryState, XchandlesSlotValue,
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

    #[allow(clippy::too_many_arguments)]
    pub fn register_handle(
        self,
        ctx: &mut SpendContext,
        left_slot: Slot<XchandlesSlotValue>,
        right_slot: Slot<XchandlesSlotValue>,
        precommit_coin: PrecommitCoin<XchandlesPrecommitValue>,
        base_handle_price: u64,
    ) -> Result<(Conditions, XchandlesRegistry, Vec<Slot<XchandlesSlotValue>>), DriverError> {
        // spend slots
        let Some(left_slot_value) = left_slot.info.value else {
            return Err(DriverError::Custom("Missing left slot value".to_string()));
        };
        let Some(right_slot_value) = right_slot.info.value else {
            return Err(DriverError::Custom("Missing right slot value".to_string()));
        };

        let spender_inner_puzzle_hash: Bytes32 = self.info.inner_puzzle_hash().into();

        left_slot.spend(ctx, spender_inner_puzzle_hash)?;
        right_slot.spend(ctx, spender_inner_puzzle_hash)?;

        let handle: String = precommit_coin.value.secret_and_handle.handle.clone();
        let handle_hash: Bytes32 = handle.tree_hash().into();

        let secret = precommit_coin.value.secret_and_handle.secret;

        let start_time = precommit_coin.value.start_time;

        let num_years = precommit_coin.coin.amount
            / XchandlesFactorPricingPuzzleArgs::get_price(base_handle_price, &handle, 1);
        let expiration = precommit_coin.value.start_time + num_years * 366 * 24 * 60 * 60;

        let new_slots_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };
        let new_slots = vec![
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    left_slot_value
                        .with_neighbors(left_slot_value.neighbors.left_value, handle_hash),
                    None,
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    XchandlesSlotValue::new(
                        handle_hash,
                        left_slot_value.handle_hash,
                        right_slot_value.handle_hash,
                        expiration,
                        precommit_coin.value.owner_launcher_id,
                        precommit_coin.value.resolved_launcher_id,
                    ),
                    None,
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    right_slot_value
                        .with_neighbors(handle_hash, right_slot_value.neighbors.right_value),
                    None,
                ),
            ),
        ];

        // calculate announcement
        let register_announcement: Bytes32 = clvm_tuple!(
            handle.clone(),
            clvm_tuple!(
                expiration,
                clvm_tuple!(
                    precommit_coin.value.owner_launcher_id,
                    precommit_coin.value.resolved_launcher_id
                )
            )
        )
        .tree_hash()
        .into();
        let mut register_announcement: Vec<u8> = register_announcement.to_vec();
        register_announcement.insert(0, b'r');

        // spend precommit coin
        precommit_coin.spend(
            ctx,
            1, // mode 1 = register/expire (use value)
            spender_inner_puzzle_hash,
        )?;

        // finally, spend self
        let register = XchandlesRegistryAction::Register(XchandlesRegisterActionSolution {
            handle_hash,
            left_value: left_slot_value.handle_hash,
            right_value: right_slot_value.handle_hash,
            pricing_puzzle_reveal: XchandlesFactorPricingPuzzleArgs::get_puzzle(
                ctx,
                base_handle_price,
            )?,
            pricing_puzzle_solution: XchandlesFactorPricingSolution {
                current_expiration: 0,
                handle: handle.clone(),
                num_years,
            },
            cat_maker_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                precommit_coin.asset_id.tree_hash().into(),
            )?,
            cat_maker_solution: (),
            rest_data_hash: clvm_tuple!(
                precommit_coin.value.owner_launcher_id,
                precommit_coin.value.resolved_launcher_id
            )
            .tree_hash()
            .into(),
            start_time,
            secret_hash: secret.tree_hash().into(),
            refund_puzzle_hash_hash: precommit_coin.refund_puzzle_hash.tree_hash().into(),
            left_left_value_hash: left_slot_value.neighbors.left_value.tree_hash().into(),
            left_data_hash: left_slot_value.after_neigbors_data_hash().into(),
            right_right_value_hash: right_slot_value.neighbors.right_value.tree_hash().into(),
            right_data_hash: right_slot_value.after_neigbors_data_hash().into(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![register])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_xchandles = XchandlesRegistry::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child XCHandles singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                my_coin.puzzle_hash,
                register_announcement,
            )),
            new_xchandles,
            new_slots,
        ))
    }

    pub fn update(
        self,
        ctx: &mut SpendContext,
        slot: Slot<XchandlesSlotValue>,
        new_owner_launcher_id: Bytes32,
        new_resolved_launcher_id: Bytes32,
        announcer_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, XchandlesRegistry, Vec<Slot<XchandlesSlotValue>>), DriverError> {
        // spend slots
        let Some(slot_value) = slot.info.value else {
            return Err(DriverError::Custom("Missing slot value".to_string()));
        };

        let spender_inner_puzzle_hash: Bytes32 = self.info.inner_puzzle_hash().into();

        slot.spend(ctx, spender_inner_puzzle_hash)?;

        let new_slots_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        let new_slots = vec![Slot::new(
            new_slots_proof,
            SlotInfo::from_value(
                self.info.launcher_id,
                slot_value.with_launcher_ids(new_owner_launcher_id, new_resolved_launcher_id),
                None,
            ),
        )];

        // spend self
        let update = XchandlesRegistryAction::Update(XchandlesUpdateActionSolution {
            value_hash: slot_value.handle_hash.tree_hash().into(),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            expiration: slot_value.expiration,
            current_owner_launcher_id: slot_value.owner_launcher_id,
            current_resolved_launcher_id: slot_value.resolved_launcher_id,
            new_owner_launcher_id,
            new_resolved_launcher_id,
            announcer_inner_puzzle_hash,
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![update])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_xchandles = XchandlesRegistry::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child XCHandles singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        let msg: Bytes32 = clvm_tuple!(
            slot_value.handle_hash,
            clvm_tuple!(new_owner_launcher_id, new_resolved_launcher_id)
        )
        .tree_hash()
        .into();
        Ok((
            Conditions::new().send_message(18, msg.into(), vec![ctx.alloc(&my_coin.puzzle_hash)?]),
            new_xchandles,
            new_slots,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn refund(
        self,
        ctx: &mut SpendContext,
        precommit_coin: PrecommitCoin<XchandlesPrecommitValue>,
        precommited_pricing_puzzle_reveal: NodePtr,
        precommited_pricing_puzzle_solution: NodePtr,
        slot: Option<Slot<XchandlesSlotValue>>,
    ) -> Result<(Conditions, XchandlesRegistry), DriverError> {
        // calculate announcement
        let refund_announcement = precommit_coin.value.after_refund_info_hash();
        let mut refund_announcement: Vec<u8> = refund_announcement.to_vec();
        refund_announcement.insert(0, b'$');

        // spend precommit coin
        let spender_inner_puzzle_hash: Bytes32 = self.info.inner_puzzle_hash().into();
        precommit_coin.spend(
            ctx,
            0, // mode 0 = refund
            spender_inner_puzzle_hash,
        )?;

        // if there's a slot, spend it
        if let Some(slot) = slot {
            slot.spend(ctx, spender_inner_puzzle_hash)?;
        }

        // then, spend self
        let refund = XchandlesRegistryAction::Refund(XchandlesRefundActionSolution {
            handle_hash: precommit_coin
                .value
                .secret_and_handle
                .handle
                .tree_hash()
                .into(),
            precommited_cat_maker_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                precommit_coin.asset_id.tree_hash().into(),
            )?,
            precommited_cat_maker_hash: DefaultCatMakerArgs::curry_tree_hash(
                precommit_coin.asset_id.tree_hash().into(),
            )
            .into(),
            precommited_cat_maker_solution: (),
            precommited_pricing_puzzle_reveal,
            precommited_pricing_puzzle_hash: ctx
                .tree_hash(precommited_pricing_puzzle_reveal)
                .into(),
            precommited_pricing_puzzle_solution,
            secret_hash: precommit_coin
                .value
                .secret_and_handle
                .secret
                .tree_hash()
                .into(),
            precommit_value_rest_hash: precommit_coin.value.after_secret_and_handle_hash().into(),
            refund_puzzle_hash_hash: precommit_coin.refund_puzzle_hash.tree_hash().into(),
            precommit_amount: precommit_coin.coin.amount,
            rest_hash: if let Some(slot) = slot {
                slot.info
                    .value
                    .ok_or(DriverError::Custom(
                        "Slot does not contain value".to_string(),
                    ))?
                    .after_handle_data_hash()
                    .into()
            } else {
                Bytes32::default()
            },
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![refund])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_xchandles = XchandlesRegistry::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child xchandles registry".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            Conditions::new().assert_puzzle_announcement(announcement_id(
                my_coin.puzzle_hash,
                refund_announcement,
            )),
            new_xchandles,
        ))
    }
}

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
    announcement_id, Conditions, DriverError, Layer, Puzzle, Spend, SpendContext,
};
use clvm_traits::{clvm_tuple, FromClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    ActionLayer, ActionLayerSolution, DelegatedStateAction, DelegatedStateActionSolution,
    XchandlesExpireAction, XchandlesExpireActionSolution,
    XchandlesExponentialPremiumRenewPuzzleArgs, XchandlesExtendAction,
    XchandlesExtendActionSolution, XchandlesFactorPricingPuzzleArgs, XchandlesOracleAction,
    XchandlesOracleActionSolution, XchandlesPrecommitValue, XchandlesRegisterAction,
    XchandlesRegisterActionSolution, XchandlesUpdateAction, XchandlesUpdateActionSolution,
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
}

impl XchandlesRegistry {
    pub fn new(coin: Coin, proof: Proof, info: XchandlesRegistryInfo) -> Self {
        Self { coin, proof, info }
    }
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
        }))
    }
}

pub enum XchandlesRegistryAction {
    Expire(XchandlesExpireActionSolution<NodePtr, (), NodePtr, u64>),
    Extend(XchandlesExtendActionSolution<NodePtr, u64, NodePtr, ()>),
    Oracle(XchandlesOracleActionSolution),
    Register(XchandlesRegisterActionSolution<NodePtr, u64, NodePtr, ()>),
    Update(XchandlesUpdateActionSolution),
    UpdateState(DelegatedStateActionSolution<XchandlesRegistryState>),
}

impl XchandlesRegistry {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        actions: Vec<XchandlesRegistryAction>,
    ) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_spends: Vec<Spend> = actions
            .into_iter()
            .map(|action| match action {
                XchandlesRegistryAction::Expire(solution) => {
                    let layer = XchandlesExpireAction::new(
                        self.info.launcher_id,
                        self.info.constants.relative_block_height,
                        self.info.constants.precommit_payout_puzzle_hash,
                    );

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                XchandlesRegistryAction::Extend(solution) => {
                    let layer = XchandlesExtendAction::new(
                        self.info.launcher_id,
                        self.info.constants.precommit_payout_puzzle_hash,
                    );

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                XchandlesRegistryAction::Oracle(solution) => {
                    let layer = XchandlesOracleAction::new(self.info.launcher_id);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                XchandlesRegistryAction::Register(solution) => {
                    let layer = XchandlesRegisterAction::new(
                        self.info.launcher_id,
                        self.info.constants.relative_block_height,
                        self.info.constants.precommit_payout_puzzle_hash,
                    );

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                XchandlesRegistryAction::Update(solution) => {
                    let layer = XchandlesUpdateAction::new(self.info.launcher_id);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                XchandlesRegistryAction::UpdateState(solution) => {
                    let layer =
                        DelegatedStateAction::new(self.info.constants.price_singleton_launcher_id);

                    let puzzle = layer.construct_puzzle(ctx)?;

                    let new_state_ptr = ctx.alloc(&solution.new_state)?;
                    let solution = layer.construct_solution(
                        ctx,
                        DelegatedStateActionSolution::<NodePtr> {
                            new_state: new_state_ptr,
                            other_singleton_inner_puzzle_hash: solution
                                .other_singleton_inner_puzzle_hash,
                        },
                    )?;

                    Ok(Spend::new(puzzle, solution))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let action_puzzle_hashes = action_spends
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
                    action_spends,
                },
            },
        )?;

        Ok(Spend::new(puzzle, solution))
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
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    right_slot_value
                        .with_neighbors(handle_hash, right_slot_value.neighbors.right_value),
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
            handle_reveal: handle.clone(),
            left_value: left_slot_value.handle_hash,
            right_value: right_slot_value.handle_hash,
            pricing_puzzle_reveal: XchandlesFactorPricingPuzzleArgs::get_puzzle(
                ctx,
                base_handle_price,
            )?,
            pricing_puzzle_solution: num_years,
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
            refund_info_hash_hash: precommit_coin.refund_info_hash.tree_hash().into(),
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

    #[allow(clippy::too_many_arguments)]
    pub fn expire_handle(
        self,
        ctx: &mut SpendContext,
        slot: Slot<XchandlesSlotValue>,
        num_years: u64,
        base_handle_price: u64,
        precommit_coin: PrecommitCoin<XchandlesPrecommitValue>,
    ) -> Result<(Conditions, XchandlesRegistry, Vec<Slot<XchandlesSlotValue>>), DriverError> {
        // spend slot
        let Some(slot_value) = slot.info.value else {
            return Err(DriverError::Custom("Missing slot value".to_string()));
        };

        let spender_inner_puzzle_hash: Bytes32 = self.info.inner_puzzle_hash().into();

        slot.spend(ctx, spender_inner_puzzle_hash)?;

        let new_slots_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        let new_expiration = precommit_coin.value.start_time + num_years * 366 * 24 * 60 * 60;

        let new_slots = vec![Slot::new(
            new_slots_proof,
            SlotInfo::from_value(
                self.info.launcher_id,
                XchandlesSlotValue::new(
                    slot_value.handle_hash,
                    slot_value.neighbors.left_value,
                    slot_value.neighbors.right_value,
                    new_expiration,
                    precommit_coin.value.owner_launcher_id,
                    precommit_coin.value.resolved_launcher_id,
                ),
            ),
        )];

        // calculate part of announcement now
        let expire_ann: Bytes32 = clvm_tuple!(
            precommit_coin.value.secret_and_handle.handle.clone(),
            clvm_tuple!(
                new_expiration,
                clvm_tuple!(
                    precommit_coin.value.owner_launcher_id,
                    precommit_coin.value.resolved_launcher_id
                )
            )
        )
        .tree_hash()
        .into();

        // spend precommit coin
        precommit_coin.spend(
            ctx,
            1, // mode 1 = register/expire (use value)
            spender_inner_puzzle_hash,
        )?;

        // finally, spend self
        let expire = XchandlesRegistryAction::Expire(XchandlesExpireActionSolution {
            handle: precommit_coin.value.secret_and_handle.handle,
            cat_maker_puzzle_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                precommit_coin.asset_id.tree_hash().into(),
            )?,
            cat_maker_puzzle_solution: (),
            expired_handle_pricing_puzzle_reveal:
                XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
                    ctx,
                    base_handle_price,
                    1000,
                )?
                .get_puzzle(ctx)?,
            expired_handle_pricing_puzzle_solution: num_years,
            refund_puzzle_hash_hash: precommit_coin.refund_puzzle_hash.tree_hash().into(),
            refund_info_hash_hash: precommit_coin.refund_info_hash.tree_hash().into(),
            secret_hash: precommit_coin
                .value
                .secret_and_handle
                .secret
                .tree_hash()
                .into(),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            current_expiration: slot_value.expiration,
            buy_time: precommit_coin.value.start_time,
            old_rest_hash: slot_value.launcher_ids_data_hash().into(),
            new_rest_hash: clvm_tuple!(
                precommit_coin.value.owner_launcher_id,
                precommit_coin.value.resolved_launcher_id
            )
            .tree_hash()
            .into(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![expire])?;
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

        let mut expire_ann: Vec<u8> = expire_ann.to_vec();
        expire_ann.insert(0, b'x');
        Ok((
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(my_coin.puzzle_hash, expire_ann)),
            new_xchandles,
            new_slots,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::type_complexity)]
    pub fn extend(
        self,
        ctx: &mut SpendContext,
        handle: String,
        slot: Slot<XchandlesSlotValue>,
        payment_asset_id: Bytes32,
        base_handle_price: u64,
        num_years: u64,
    ) -> Result<
        (
            NotarizedPayment,
            Conditions,
            XchandlesRegistry,
            Vec<Slot<XchandlesSlotValue>>,
        ),
        DriverError,
    > {
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

        let new_expiration = slot_value.expiration + num_years * 60 * 60 * 24 * 366;
        let new_slots = vec![Slot::new(
            new_slots_proof,
            SlotInfo::from_value(
                self.info.launcher_id,
                slot_value.with_expiration(new_expiration),
            ),
        )];

        // finally, spend self
        let extend = XchandlesRegistryAction::Extend(XchandlesExtendActionSolution {
            handle: handle.clone(),
            pricing_puzzle_reveal: XchandlesFactorPricingPuzzleArgs::get_puzzle(
                ctx,
                base_handle_price,
            )?,
            pricing_solution: num_years,
            cat_maker_puzzle_reveal: DefaultCatMakerArgs::get_puzzle(
                ctx,
                payment_asset_id.tree_hash().into(),
            )?,
            cat_maker_solution: (),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            expiration: slot_value.expiration,
            rest_hash: slot_value.launcher_ids_data_hash().into(),
        });

        let renew_amount =
            XchandlesFactorPricingPuzzleArgs::get_price(base_handle_price, &handle, num_years);

        let notarized_payment = NotarizedPayment {
            nonce: clvm_tuple!(handle.clone(), slot_value.expiration)
                .tree_hash()
                .into(),
            payments: vec![Payment {
                puzzle_hash: self.info.constants.precommit_payout_puzzle_hash,
                amount: renew_amount,
                memos: None,
            }],
        };

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![extend])?;
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

        let mut extend_ann: Vec<u8> = clvm_tuple!(renew_amount, handle).tree_hash().to_vec();
        extend_ann.insert(0, b'e');
        Ok((
            notarized_payment,
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(my_coin.puzzle_hash, extend_ann)),
            new_xchandles,
            new_slots,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn oracle(
        self,
        ctx: &mut SpendContext,
        slot: Slot<XchandlesSlotValue>,
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
            SlotInfo::from_value(self.info.launcher_id, slot_value),
        )];

        // finally, spend self
        let oracle = XchandlesRegistryAction::Oracle(XchandlesOracleActionSolution {
            data_treehash: slot_value.tree_hash().into(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![oracle])?;
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

        let mut oracle_ann = slot_value.tree_hash().to_vec();
        oracle_ann.insert(0, b'o');
        Ok((
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(my_coin.puzzle_hash, oracle_ann)),
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
}

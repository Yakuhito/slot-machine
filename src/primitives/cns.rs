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
use hex::encode;

use crate::{
    ActionLayer, ActionLayerSolution, CnsExpireAction, CnsExpireActionSolution, CnsExtendAction,
    CnsExtendActionSolution, CnsOracleAction, CnsOracleActionSolution, CnsRegisterAction,
    CnsRegisterActionSolution, CnsUpdateAction, CnsUpdateActionSolution, DelegatedStateAction,
    DelegatedStateActionSolution,
};

use super::{
    CnsConstants, CnsInfo, CnsPrecommitValue, CnsSlotValue, CnsState, PrecommitCoin, Slot,
    SlotInfo, SlotProof,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct Cns {
    pub coin: Coin,
    pub proof: Proof,

    pub info: CnsInfo,
}

impl Cns {
    pub fn new(coin: Coin, proof: Proof, info: CnsInfo) -> Self {
        Self { coin, proof, info }
    }
}

impl Cns {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: CnsConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) = CnsInfo::parse(allocator, parent_puzzle, constants)? else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let parent_solution = SingletonSolution::<NodePtr>::from_clvm(allocator, parent_solution)?;
        let new_state = ActionLayer::<CnsState>::get_new_state(
            allocator,
            parent_info.state,
            parent_solution.inner_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        Ok(Some(Cns {
            coin: new_coin,
            proof,
            info: new_info,
        }))
    }
}

pub enum CnsAction {
    Expire(CnsExpireActionSolution),
    Extend(CnsExtendActionSolution),
    Oracle(CnsOracleActionSolution),
    Register(CnsRegisterActionSolution),
    Update(CnsUpdateActionSolution),
    UpdatePrice(DelegatedStateActionSolution<CnsState>),
}

impl Cns {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        actions: Vec<CnsAction>,
    ) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_spends: Vec<Spend> = actions
            .into_iter()
            .map(|action| match action {
                CnsAction::Expire(solution) => {
                    let layer = CnsExpireAction::new(self.info.launcher_id);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                CnsAction::Extend(solution) => {
                    let layer = CnsExtendAction::new(
                        self.info.launcher_id,
                        self.info.constants.precommit_payout_puzzle_hash,
                    );

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                CnsAction::Oracle(solution) => {
                    let layer = CnsOracleAction::new(self.info.launcher_id);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                CnsAction::Register(solution) => {
                    let layer = CnsRegisterAction::new(
                        self.info.launcher_id,
                        self.info.constants.precommit_payout_puzzle_hash,
                        self.info.constants.relative_block_height,
                    );

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                CnsAction::Update(solution) => {
                    let layer = CnsUpdateAction::new(self.info.launcher_id);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                CnsAction::UpdatePrice(solution) => {
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
                            &CnsInfo::action_puzzle_hashes(
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
    pub fn register_name(
        self,
        ctx: &mut SpendContext,
        left_slot: Slot<CnsSlotValue>,
        right_slot: Slot<CnsSlotValue>,
        precommit_coin: PrecommitCoin<CnsPrecommitValue>,
        price_update: Option<CnsAction>,
    ) -> Result<(Conditions, Cns, Vec<Slot<CnsSlotValue>>), DriverError> {
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

        let name: String = precommit_coin.value.secret_and_name.name.clone();
        let name_hash: Bytes32 = name.tree_hash().into();

        let version = precommit_coin.value.version_and_launcher.version;
        let secret = precommit_coin.value.secret_and_name.secret;

        let start_time = precommit_coin.value.start_time;
        let precommitment_amount = precommit_coin.coin.amount;

        let base_price = if let Some(CnsAction::UpdatePrice(ref price_update)) = price_update {
            price_update.new_state.registration_base_price
        } else {
            self.info.state.registration_base_price
        };
        let expiration = start_time
            + (precommitment_amount
                / (base_price * CnsRegisterAction::get_price_factor(&name).unwrap_or(1)))
                * 60
                * 60
                * 24
                * 366;

        let name_nft_launcher_id = precommit_coin
            .value
            .version_and_launcher
            .name_nft_launcher_id;

        let new_slots_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };
        let new_slots = vec![
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    left_slot_value.with_neighbors(left_slot_value.neighbors.left_value, name_hash),
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    CnsSlotValue::new(
                        name_hash,
                        left_slot_value.name_hash,
                        right_slot_value.name_hash,
                        expiration,
                        version,
                        name_nft_launcher_id,
                    ),
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    right_slot_value
                        .with_neighbors(name_hash, right_slot_value.neighbors.right_value),
                ),
            ),
        ];

        // spend precommit coin
        let precommit_coin_id = precommit_coin.coin.coin_id();
        precommit_coin.spend(ctx, spender_inner_puzzle_hash)?;

        // finally, spend self
        let register = CnsAction::Register(CnsRegisterActionSolution {
            name_hash,
            name_reveal: name.clone(),
            left_value: left_slot_value.name_hash,
            right_value: right_slot_value.name_hash,
            name_nft_launcher_id,
            version,
            start_time,
            secret_hash: secret.tree_hash().into(),
            precommitment_amount,
            left_left_value_hash: left_slot_value.neighbors.left_value.tree_hash().into(),
            left_data_hash: left_slot_value.after_neigbors_data_hash().into(),
            right_right_value_hash: right_slot_value.neighbors.right_value.tree_hash().into(),
            right_data_hash: right_slot_value.after_neigbors_data_hash().into(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(
            ctx,
            if let Some(price_update) = price_update {
                vec![price_update, register]
            } else {
                vec![register]
            },
        )?;
        println!(
            "my puzzle: {:?}",
            encode(ctx.serialize(&my_spend.puzzle)?.into_bytes())
        );
        println!(
            "my solution: {:?}",
            encode(ctx.serialize(&my_spend.solution)?.into_bytes())
        );
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_cns = Cns::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child CNS singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            Conditions::new().assert_concurrent_spend(precommit_coin_id),
            new_cns,
            new_slots,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn expire_name(
        self,
        ctx: &mut SpendContext,
        slot: Slot<CnsSlotValue>,
        left_slot: Slot<CnsSlotValue>,
        right_slot: Slot<CnsSlotValue>,
    ) -> Result<(Conditions, Cns, Vec<Slot<CnsSlotValue>>), DriverError> {
        // spend slots
        let Some(slot_value) = slot.info.value else {
            return Err(DriverError::Custom("Missing slot value".to_string()));
        };
        let Some(left_slot_value) = left_slot.info.value else {
            return Err(DriverError::Custom("Missing left slot value".to_string()));
        };
        let Some(right_slot_value) = right_slot.info.value else {
            return Err(DriverError::Custom("Missing right slot value".to_string()));
        };

        let spender_inner_puzzle_hash: Bytes32 = self.info.inner_puzzle_hash().into();

        slot.spend(ctx, spender_inner_puzzle_hash)?;
        left_slot.spend(ctx, spender_inner_puzzle_hash)?;
        right_slot.spend(ctx, spender_inner_puzzle_hash)?;

        let new_slots_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };

        let new_slots = vec![
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    left_slot_value.with_neighbors(
                        left_slot_value.neighbors.left_value,
                        right_slot_value.name_hash,
                    ),
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    right_slot_value.with_neighbors(
                        left_slot_value.name_hash,
                        right_slot_value.neighbors.right_value,
                    ),
                ),
            ),
        ];

        // finally, spend self
        let expire = CnsAction::Expire(CnsExpireActionSolution {
            value: slot_value.name_hash,
            left_value: left_slot_value.name_hash,
            left_left_value: left_slot_value.neighbors.left_value,
            left_rest_hash: left_slot_value.after_neigbors_data_hash().into(),
            right_value: right_slot_value.name_hash,
            right_right_value: right_slot_value.neighbors.right_value,
            right_rest_hash: right_slot_value.after_neigbors_data_hash().into(),
            expiration: slot_value.expiration,
            data_hash: clvm_tuple!(slot_value.version, slot_value.launcher_id)
                .tree_hash()
                .into(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![expire])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_cns = Cns::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child CNS singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            Conditions::new().assert_concurrent_spend(slot.coin.coin_id()),
            new_cns,
            new_slots,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::type_complexity)]
    pub fn extend(
        self,
        ctx: &mut SpendContext,
        name: String,
        slot: Slot<CnsSlotValue>,
        renew_amount: u64,
    ) -> Result<(NotarizedPayment, Conditions, Cns, Vec<Slot<CnsSlotValue>>), DriverError> {
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

        let new_expiration = slot_value.expiration
            + (renew_amount
                / (CnsRegisterAction::get_price_factor(&name).unwrap_or(1)
                    * self.info.state.registration_base_price))
                * 60
                * 60
                * 24
                * 366;
        let new_slots = vec![Slot::new(
            new_slots_proof,
            SlotInfo::from_value(
                self.info.launcher_id,
                slot_value.with_expiration(new_expiration),
            ),
        )];

        // finally, spend self
        let extend = CnsAction::Extend(CnsExtendActionSolution {
            renew_amount,
            name: name.clone(),
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            expiration: slot_value.expiration,
            rest_hash: clvm_tuple!(slot_value.version, slot_value.launcher_id)
                .tree_hash()
                .into(),
        });

        let notarized_payment = NotarizedPayment {
            nonce: clvm_tuple!(name, slot_value.expiration).tree_hash().into(),
            payments: vec![Payment {
                puzzle_hash: self.info.constants.precommit_payout_puzzle_hash,
                amount: renew_amount,
                memos: vec![],
            }],
        };

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![extend])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);

        let new_cns = Cns::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child CNS singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            notarized_payment,
            Conditions::new().assert_concurrent_spend(slot.coin.coin_id()), // todo: not really secure
            new_cns,
            new_slots,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn oracle(
        self,
        ctx: &mut SpendContext,
        slot: Slot<CnsSlotValue>,
    ) -> Result<(Conditions, Cns, Vec<Slot<CnsSlotValue>>), DriverError> {
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
        let oracle = CnsAction::Oracle(CnsOracleActionSolution {
            value: slot_value.name_hash,
            rest_hash: clvm_tuple!(slot_value.neighbors, slot_value.after_neigbors_data_hash())
                .tree_hash()
                .into(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![oracle])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_cns = Cns::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child CNS singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        let slot_value_hash: Bytes32 = slot_value.tree_hash().into();
        Ok((
            Conditions::new()
                .assert_puzzle_announcement(announcement_id(my_coin.puzzle_hash, slot_value_hash)),
            new_cns,
            new_slots,
        ))
    }

    pub fn update(
        self,
        ctx: &mut SpendContext,
        slot: Slot<CnsSlotValue>,
        new_version: u32,
        new_launcher_id: Bytes32,
        announcer_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, Cns, Vec<Slot<CnsSlotValue>>), DriverError> {
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
                slot_value.with_version_and_launcher_id(new_version, new_launcher_id),
            ),
        )];

        // spend self
        let update = CnsAction::Update(CnsUpdateActionSolution {
            value: slot_value.name_hash,
            neighbors_hash: slot_value.neighbors.tree_hash().into(),
            expiration: slot_value.expiration,
            current_version: slot_value.version,
            current_launcher_id: slot_value.launcher_id,
            new_version,
            new_launcher_id,
            announcer_inner_puzzle_hash,
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants;
        let my_spend = self.spend(ctx, vec![update])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_cns = Cns::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child CNS singleton".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        let msg: Bytes32 = clvm_tuple!(
            slot_value.name_hash,
            clvm_tuple!(new_version, new_launcher_id)
        )
        .tree_hash()
        .into();
        Ok((
            Conditions::new()
                .assert_concurrent_spend(slot.coin.coin_id()) // todo: not secure
                .send_message(18, msg.into(), vec![ctx.alloc(&my_coin.puzzle_hash)?]),
            new_cns,
            new_slots,
        ))
    }
}

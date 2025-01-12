use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::FromClvm;
use clvmr::{Allocator, NodePtr};

use crate::{
    ActionLayer, ActionLayerSolution, CatalogPrecommitValue, CatalogRegisterAction,
    CatalogRegisterActionSolution, DelegatedStateAction, DelegatedStateActionSolution,
    ANY_METADATA_UPDATER_HASH,
};

use super::{
    CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState, CatalogSlotValue,
    DefaultCatMakerArgs, PrecommitCoin, Slot, SlotInfo, SlotProof, UniquenessPrelauncher,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct CatalogRegistry {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogRegistryInfo,
}

impl CatalogRegistry {
    pub fn new(coin: Coin, proof: Proof, info: CatalogRegistryInfo) -> Self {
        Self { coin, proof, info }
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
        }))
    }
}

pub enum CatalogRegistryAction {
    Register(CatalogRegisterActionSolution<NodePtr, ()>),
    UpdatePrice(DelegatedStateActionSolution<CatalogRegistryState>),
}

impl CatalogRegistry {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        actions: Vec<CatalogRegistryAction>,
    ) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_spends: Vec<Spend> = actions
            .into_iter()
            .map(|action| match action {
                CatalogRegistryAction::Register(solution) => {
                    let layer = CatalogRegisterAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                CatalogRegistryAction::UpdatePrice(solution) => {
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
                            &CatalogRegistryInfo::action_puzzle_hashes(
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
    pub fn register_cat(
        self,
        ctx: &mut SpendContext,
        tail_hash: Bytes32,
        left_slot: Slot<CatalogSlotValue>,
        right_slot: Slot<CatalogSlotValue>,
        precommit_coin: PrecommitCoin<CatalogPrecommitValue>,
        eve_nft_inner_spend: Spend,
        price_update: Option<CatalogRegistryAction>,
    ) -> Result<(Conditions, CatalogRegistry, Vec<Slot<CatalogSlotValue>>), DriverError> {
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

        let new_slots_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
        };
        let new_slots = vec![
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    CatalogSlotValue::new(
                        left_slot_value.asset_id,
                        left_slot_value.neighbors.left_value,
                        tail_hash,
                    ),
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    CatalogSlotValue::new(
                        tail_hash,
                        left_slot_value.asset_id,
                        right_slot_value.asset_id,
                    ),
                ),
            ),
            Slot::new(
                new_slots_proof,
                SlotInfo::from_value(
                    self.info.launcher_id,
                    CatalogSlotValue::new(
                        right_slot_value.asset_id,
                        tail_hash,
                        right_slot_value.neighbors.right_value,
                    ),
                ),
            ),
        ];

        // spend precommit coin
        let initial_inner_puzzle_hash = precommit_coin.value.initial_inner_puzzle_hash;
        precommit_coin.spend(
            ctx,
            self.info.constants.precommit_payout_puzzle_hash,
            spender_inner_puzzle_hash,
        )?;

        // spend uniqueness prelauncher
        let uniqueness_prelauncher = UniquenessPrelauncher::<Bytes32>::new(
            &mut ctx.allocator,
            self.coin.coin_id(),
            tail_hash,
        )?;
        let nft_launcher = uniqueness_prelauncher.spend(ctx)?;

        // launch eve nft
        let (_, nft) = nft_launcher.mint_eve_nft(
            ctx,
            initial_inner_puzzle_hash,
            (),
            ANY_METADATA_UPDATER_HASH.into(),
            self.info.constants.royalty_address,
            self.info.constants.royalty_ten_thousandths,
        )?;

        // spend nft launcher
        let nft_coin_id = nft.coin.coin_id();
        nft.spend(ctx, eve_nft_inner_spend)?;

        // finally, spend self
        let register = CatalogRegistryAction::Register(CatalogRegisterActionSolution {
            cat_maker_reveal: DefaultCatMakerArgs::get_puzzle(ctx, tail_hash.tree_hash().into())?,
            cat_maker_solution: (),
            tail_hash,
            initial_nft_owner_ph: initial_inner_puzzle_hash,
            refund_puzzle_hash_hash: precommit_coin.refund_puzzle_hash.tree_hash().into(),
            left_tail_hash: left_slot_value.asset_id,
            left_left_tail_hash: left_slot_value.neighbors.left_value,
            right_tail_hash: right_slot_value.asset_id,
            right_right_tail_hash: right_slot_value.neighbors.right_value,
            my_id: self.coin.coin_id(),
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
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_catalog = CatalogRegistry::from_parent_spend(
            &mut ctx.allocator,
            my_coin,
            my_puzzle,
            my_spend.solution,
            my_constants,
        )?
        .ok_or(DriverError::Custom(
            "Could not parse child catalog".to_string(),
        ))?;

        ctx.spend(my_coin, my_spend)?;

        Ok((
            Conditions::new().assert_concurrent_spend(nft_coin_id),
            new_catalog,
            new_slots,
        ))
    }
}

use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::ToClvm;
use clvmr::{Allocator, NodePtr};

use crate::{
    Action, ActionLayer, ActionLayerSolution, CatalogRegisterAction, CatalogRegisterActionSolution,
    ANY_METADATA_UPDATER_HASH,
};

use super::{
    CatalogAction, CatalogActionSolution, CatalogConstants, CatalogInfo, CatalogPrecommitValue,
    CatalogSlotValue, CatalogState, PrecommitCoin, Slot, SlotInfo, SlotProof,
    UniquenessPrelauncher,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct Catalog {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogInfo,
}

impl Catalog {
    pub fn new(coin: Coin, proof: Proof, info: CatalogInfo) -> Self {
        Self { coin, proof, info }
    }
}

impl Catalog {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: CatalogConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) = CatalogInfo::parse(allocator, parent_puzzle, constants)? else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let new_state = ActionLayer::<CatalogState>::get_new_state(
            allocator,
            parent_info.state.clone(),
            parent_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        Ok(Some(Catalog {
            coin: new_coin,
            proof,
            info: new_info,
        }))
    }
}

impl Catalog {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        actions: Vec<CatalogAction>,
        solutions: Vec<CatalogActionSolution>,
    ) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let actions = actions
            .into_iter()
            .map(|a| a.construct_puzzle(ctx))
            .collect::<Result<Vec<_>, _>>()?;
        let action_puzzle_hashes = actions
            .iter()
            .map(|a| ctx.tree_hash(*a).into())
            .collect::<Vec<Bytes32>>();

        let solutions = solutions
            .into_iter()
            .map(|sol| match sol {
                CatalogActionSolution::Register(solution) => solution.to_clvm(&mut ctx.allocator),
                CatalogActionSolution::UpdatePrice(solution) => {
                    solution.to_clvm(&mut ctx.allocator)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(&action_puzzle_hashes)
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends: actions
                        .into_iter()
                        .zip(solutions)
                        .map(|(a, s)| Spend {
                            puzzle: a,
                            solution: s,
                        })
                        .collect(),
                },
            },
        )?;

        Ok(Spend::new(puzzle, solution))
    }

    pub fn register_cat(
        self,
        ctx: &mut SpendContext,
        tail_hash: Bytes32,
        left_slot: Slot<CatalogSlotValue>,
        right_slot: Slot<CatalogSlotValue>,
        precommit_coin: PrecommitCoin<CatalogPrecommitValue>,
        eve_nft_inner_spend: Spend,
    ) -> Result<(Conditions, Catalog, Vec<Slot<CatalogSlotValue>>), DriverError> {
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
                        left_slot_value.neighbors.left_asset_id,
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
                        right_slot_value.neighbors.right_asset_id,
                    ),
                ),
            ),
        ];

        // spend precommit coin
        let initial_inner_puzzle_hash = precommit_coin.value.initial_inner_puzzle_hash;
        precommit_coin.spend(ctx, spender_inner_puzzle_hash)?;

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
        let register_action = CatalogAction::Register(CatalogRegisterAction {
            launcher_id: self.info.launcher_id,
            royalty_puzzle_hash_hash: self.info.constants.royalty_address.tree_hash().into(),
            trade_price_percentage: self.info.constants.royalty_ten_thousandths,
            precommit_payout_puzzle_hash: self.info.constants.precommit_payout_puzzle_hash,
            relative_block_height: self.info.constants.relative_block_height,
        });

        let register_solution = CatalogActionSolution::Register(CatalogRegisterActionSolution {
            tail_hash,
            initial_nft_owner_ph: initial_inner_puzzle_hash,
            left_tail_hash: left_slot_value.asset_id,
            left_left_tail_hash: left_slot_value.neighbors.left_asset_id,
            right_tail_hash: right_slot_value.asset_id,
            right_right_tail_hash: right_slot_value.neighbors.right_asset_id,
            my_id: self.coin.coin_id(),
        });

        let my_coin = self.coin;
        let my_constants = self.info.constants.clone();
        let my_spend = self.spend(ctx, vec![register_action], vec![register_solution])?;
        let my_puzzle = Puzzle::parse(&ctx.allocator, my_spend.puzzle);
        let new_catalog = Catalog::from_parent_spend(
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

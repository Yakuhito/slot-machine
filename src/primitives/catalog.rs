use chia::{
    protocol::Coin,
    puzzles::{LineageProof, Proof},
};
use chia_wallet_sdk::{DriverError, Puzzle, SpendContext};
use clvmr::{Allocator, NodePtr};

use crate::ActionLayer;

use super::{CatalogAction, CatalogActionSolution, CatalogConstants, CatalogInfo, CatalogState};

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
    ) -> Result<(), DriverError> {
    }
}

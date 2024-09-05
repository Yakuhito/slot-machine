use chia::{
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::{SingletonArgs, SingletonSolution},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{DriverError, Layer, Spend, SpendContext};

use super::{AddCat, CatalogPrerollerInfo};

/// Used to create slots & then transition to either a new
/// slot launcher or the main logic singleton innerpuzzle
#[derive(Debug, Clone)]
#[must_use]
pub struct SlotLauncher {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogPrerollerInfo,
}

impl SlotLauncher {
    pub fn new(
        coin: Coin,
        proof: Proof,
        launcher_id: Bytes32,
        to_launch: Vec<AddCat>,
        next_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            coin,
            proof,
            info: CatalogPrerollerInfo::new(launcher_id, to_launch, next_puzzle_hash),
        }
    }

    pub fn child(
        &self,
        ctx: &mut SpendContext,
        next_to_launch: Vec<AddCat>,
        next_next_puzzle_hash: Bytes32,
    ) -> Result<Option<Self>, DriverError> {
        let child_info =
            CatalogPrerollerInfo::new(self.info.launcher_id, next_to_launch, next_next_puzzle_hash);

        let child_inner_puzzle_hash = child_info
            .clone()
            .inner_puzzle_hash(ctx, Bytes32::default())?;
        let child_puzzle_hash =
            SingletonArgs::curry_tree_hash(self.info.launcher_id, child_inner_puzzle_hash);

        if child_puzzle_hash != self.info.next_puzzle_hash.into() {
            return Ok(None);
        }

        let child_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self
                .info
                .clone()
                .inner_puzzle_hash(ctx, self.coin.coin_id())?
                .into(),
            parent_amount: self.coin.amount,
        });
        let child_coin = Coin::new(self.coin.coin_id(), child_puzzle_hash.into(), 1);

        Ok(Some(Self {
            coin: child_coin,
            proof: child_proof,
            info: child_info,
        }))
    }

    pub fn spend(self, ctx: &mut SpendContext) -> Result<(), DriverError> {
        let layers = self
            .info
            .into_layers(&mut ctx.allocator, self.coin.coin_id())?;

        let puzzle = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: (),
            },
        )?;

        ctx.spend(self.coin, Spend::new(puzzle, solution))?;

        Ok(())
    }
}

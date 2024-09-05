use chia::{
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, Proof},
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

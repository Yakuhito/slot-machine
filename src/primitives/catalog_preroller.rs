use chia::{
    protocol::Coin,
    puzzles::{singleton::SingletonSolution, Proof},
};
use chia_wallet_sdk::{DriverError, Layer, Spend, SpendContext};

use crate::CatalogPrerollerSolution;

use super::CatalogPrerollerInfo;

/// Used to create slots & then transition to either a new
/// slot launcher or the main logic singleton innerpuzzle
#[derive(Debug, Clone)]
#[must_use]
pub struct CatalogPreroller {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogPrerollerInfo,
}

impl CatalogPreroller {
    pub fn new(coin: Coin, proof: Proof, info: CatalogPrerollerInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn spend(self, ctx: &mut SpendContext) -> Result<(), DriverError> {
        let layers = self.info.into_layers()?;

        let puzzle = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: CatalogPrerollerSolution {
                    my_coin_id: self.coin.coin_id(),
                },
            },
        )?;

        ctx.spend(self.coin, Spend::new(puzzle, solution))?;

        Ok(())
    }
}

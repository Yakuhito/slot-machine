use chia::{protocol::Coin, puzzles::Proof};
use chia_wallet_sdk::{DriverError, Primitive, Puzzle};
use clvmr::{Allocator, NodePtr};

use super::{CatalogConstants, CatalogInfo};

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
    fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        coin: Coin,
        constants: CatalogConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        todo!()
    }
}

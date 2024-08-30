use chia::{protocol::Coin, puzzles::Proof};

use super::CatalogInfo;

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

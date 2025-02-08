use chia::{protocol::Coin, puzzles::Proof};

use super::DigRewardDistributorInfo;

#[derive(Debug, Clone)]
#[must_use]
pub struct DigRewardDistributor {
    pub coin: Coin,
    pub proof: Proof,
    pub info: DigRewardDistributorInfo,
}

impl DigRewardDistributor {
    pub fn new(coin: Coin, proof: Proof, info: DigRewardDistributorInfo) -> Self {
        Self { coin, proof, info }
    }
}

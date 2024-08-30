use chia::{
    protocol::{Bytes32, Coin},
    puzzles::Proof,
};

use super::SlotLauncherInfo;

/// Used to create slots & then transition to either a new
/// slot launcher or the main logic singleton innerpuzzle
#[derive(Debug, Clone)]
#[must_use]
pub struct SlotLauncher {
    pub coin: Coin,
    pub proof: Proof,
    pub info: SlotLauncherInfo,
}

impl SlotLauncher {
    pub fn new(
        coin: Coin,
        proof: Proof,
        launcher_id: Bytes32,
        slot_value_hashes: Vec<Bytes32>,
        next_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            coin,
            proof,
            info: SlotLauncherInfo::new(launcher_id, slot_value_hashes, next_puzzle_hash),
        }
    }
}

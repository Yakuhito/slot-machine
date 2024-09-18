use chia::{
    protocol::Coin,
    puzzles::{singleton::SingletonSolution, Proof},
};
use chia_wallet_sdk::{DriverError, Layer, Spend, SpendContext};

use super::{CnsPrerollerInfo, CnsSlotValue, Slot, SlotProof};

/// Used to create slots & then transition to either a new
/// slot launcher or the main logic singleton innerpuzzle
#[derive(Debug, Clone)]
#[must_use]
pub struct CnsPreroller {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CnsPrerollerInfo,
}

impl CnsPreroller {
    pub fn new(coin: Coin, proof: Proof, info: CnsPrerollerInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn spend(self, ctx: &mut SpendContext) -> Result<Vec<Slot<CnsSlotValue>>, DriverError> {
        let slot_proof = SlotProof {
            parent_parent_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.clone().inner_puzzle_hash(ctx)?.into(),
        };

        let slots = CnsPrerollerInfo::get_slots(
            self.info.to_launch.clone(),
            self.info.launcher_id,
            slot_proof,
        )?;
        let layers = self.info.into_layers()?;

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

        Ok(slots)
    }
}

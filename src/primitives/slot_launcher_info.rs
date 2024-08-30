use chia::{clvm_utils::TreeHash, protocol::Bytes32};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, SingletonLayer, SpendContext};
use clvmr::Allocator;

use crate::SlotLauncherLayer;

type SlotLauncherLayers = SingletonLayer<SlotLauncherLayer>;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotLauncherInfo {
    pub launcher_id: Bytes32,
    pub slot_value_hashes: Vec<Bytes32>,
    pub next_puzzle_hash: Bytes32,
}

impl SlotLauncherInfo {
    pub fn new(
        launcher_id: Bytes32,
        slot_value_hashes: Vec<Bytes32>,
        next_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            slot_value_hashes,
            next_puzzle_hash,
        }
    }

    pub fn parse(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(layers) = SlotLauncherLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        Ok(Some(Self::from_layers(layers)))
    }

    pub fn from_layers(layers: SlotLauncherLayers) -> Self {
        Self {
            launcher_id: layers.launcher_id,
            slot_value_hashes: layers.inner_puzzle.slot_value_hashes,
            next_puzzle_hash: layers.inner_puzzle.next_puzzle_hash,
        }
    }

    #[must_use]
    pub fn into_layers(self) -> SlotLauncherLayers {
        SingletonLayer::new(
            self.launcher_id,
            SlotLauncherLayer::new(
                self.launcher_id,
                self.slot_value_hashes,
                self.next_puzzle_hash,
            ),
        )
    }

    pub fn inner_puzzle_hash(&self, ctx: &mut SpendContext) -> Result<TreeHash, DriverError> {
        let inner_puzzle = SlotLauncherLayer::new(
            self.launcher_id,
            self.slot_value_hashes.clone(),
            self.next_puzzle_hash,
        )
        .construct_puzzle(ctx)?;

        Ok(ctx.tree_hash(inner_puzzle))
    }
}

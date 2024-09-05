use chia::{clvm_utils::TreeHash, protocol::Bytes32};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, SingletonLayer, SpendContext};
use clvmr::{Allocator, NodePtr};

use crate::ConditionsLayer;

pub type SlotLauncherLayers = SingletonLayer<ConditionsLayer<NodePtr>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCatInfo {
    pub asset_id_left: Bytes32,
    pub asset_id_right: Bytes32,

    pub code: String,
    pub name: String,
    pub description: String,

    pub image_urls: Vec<String>,
    pub image_hash: Bytes32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCat {
    pub asset_id: Bytes32,
    pub info: Option<AddCatInfo>,
}

impl AddCat {
    pub fn new(asset_id: Bytes32, info: AddCatInfo) -> Self {
        Self {
            asset_id,
            info: Some(info),
        }
    }

    pub fn from_asset_id(asset_id: Bytes32) -> Self {
        Self {
            asset_id,
            info: None,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPrerollInfo {
    pub launcher_id: Bytes32,
    pub to_launch: Vec<AddCat>,
}

impl CatalogPrerollInfo {
    pub fn new(launcher_id: Bytes32, to_launch: Vec<AddCat>) -> Self {
        Self {
            launcher_id,
            to_launch,
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

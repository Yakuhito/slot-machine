use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes, Bytes32},
};
use chia_wallet_sdk::{
    Condition, Conditions, DriverError, Layer, Puzzle, SingletonLayer, SpendContext,
};
use clvmr::{Allocator, NodePtr};

use crate::ConditionsLayer;

use super::{Slot, UniquenessPrelauncher};

pub type CatalogPrerollLayers = SingletonLayer<ConditionsLayer<NodePtr>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCatInfo {
    pub asset_id_left: Bytes32,
    pub asset_id_right: Bytes32,

    pub code: String,
    pub name: String,
    pub description: String,

    pub image_urls: Vec<String>,
    pub image_hash: Bytes32,

    pub metadata_urls: Vec<String>,
    pub metadata_hash: Bytes32,
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
    pub next_puzzle_hash: Bytes32,
}

pub fn get_hint(memos: &[Bytes]) -> Option<Bytes32> {
    let hint = memos.first()?;

    let Ok(hint) = hint.try_into() else {
        return None;
    };

    Some(hint)
}

impl CatalogPrerollInfo {
    pub fn new(launcher_id: Bytes32, to_launch: Vec<AddCat>, next_puzzle_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            to_launch,
            next_puzzle_hash,
        }
    }

    pub fn parse(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(layers) = CatalogPrerollLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        Self::from_layers(layers)
    }

    pub fn from_layers(layers: CatalogPrerollLayers) -> Result<Option<Self>, DriverError> {
        let Some(Condition::CreateCoin(recreate_condition)) =
            layers.inner_puzzle.conditions.as_ref().iter().find(|c| {
                let Condition::CreateCoin(cc) = c else {
                    return false;
                };

                cc.amount % 2 == 1
            })
        else {
            return Ok(None);
        };

        let Some(launcher_id) = get_hint(&recreate_condition.memos) else {
            return Ok(None);
        };

        let next_puzzle_hash = recreate_condition.puzzle_hash;
        let to_launch = layers
            .inner_puzzle
            .conditions
            .into_iter()
            .filter_map(|cond| {
                let Condition::CreateCoin(create_coin) = cond else {
                    return None;
                };

                if create_coin.amount != 0 {
                    return None;
                }

                // we get the asset_id from slot launches
                // uniqueness prelauncher would not have any memos
                let asset_id = get_hint(&create_coin.memos)?;

                Some(AddCat::from_asset_id(asset_id))
            })
            .collect();

        Ok(Some(Self {
            launcher_id,
            to_launch,
            next_puzzle_hash,
        }))
    }

    pub fn get_prelaunchers_and_slots(
        allocator: &mut Allocator,
        to_launch: Vec<AddCat>,
        my_coin_id: Bytes32,
        my_launcher_id: Bytes32,
    ) -> Result<Vec<(AddCat, UniquenessPrelauncher<Bytes32>, Slot)>, DriverError> {
        let mut res = Vec::with_capacity(to_launch.len());

        for add_cat in to_launch {
            let asset_id = add_cat.asset_id;

            // uniqueness prelauncher
            let uniq_prelauncher =
                UniquenessPrelauncher::<Bytes32>::new(allocator, my_coin_id, asset_id)?;

            // slot
            let asset_id_hash: Bytes32 = asset_id.tree_hash().into();
            let slot = Slot::new(my_coin_id, my_launcher_id, asset_id_hash)?;

            res.push((add_cat, uniq_prelauncher, slot));
        }

        Ok(res)
    }

    #[must_use]
    pub fn into_layers(
        self,
        allocator: &mut Allocator,
        my_coin_id: Bytes32,
    ) -> Result<CatalogPrerollLayers, DriverError> {
        let mut conditions =
            Conditions::new().create_coin(self.next_puzzle_hash, 1, vec![self.launcher_id.into()]);

        for (add_cat, uniq_prelauncher, slot) in CatalogPrerollInfo::get_prelaunchers_and_slots(
            allocator,
            self.to_launch,
            my_coin_id,
            self.launcher_id,
        )? {
            let asset_id = add_cat.asset_id;
            let Some(info) = add_cat.info else {
                return Err(DriverError::Custom(
                    "Missing CAT launch info (required to build puzzle)".to_string(),
                ));
            };
            // todo: secure NFT launch via annpuncement

            conditions = conditions
                .create_coin(
                    uniq_prelauncher.coin.puzzle_hash,
                    uniq_prelauncher.coin.amount,
                    vec![],
                )
                .create_coin(
                    slot.coin.puzzle_hash,
                    slot.coin.amount,
                    vec![slot.value_hash.into()],
                );
        }

        Ok(SingletonLayer::new(
            self.launcher_id,
            ConditionsLayer::new(conditions),
        ))
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

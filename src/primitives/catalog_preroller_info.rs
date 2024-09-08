use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes, Bytes32, Program},
};
use chia_wallet_sdk::{
    Condition, Conditions, DriverError, Layer, Puzzle, SingletonLayer, SpendContext,
};
use clvm_traits::ToClvm;
use clvmr::{Allocator, NodePtr};

use crate::{
    CatNftMetadata, CatalogPrerollerLayer, CatalogPrerollerNftInfo, ConditionsLayer,
    ANY_METADATA_UPDATER,
};

use super::{CatalogSlotValue, Slot, UniquenessPrelauncher, SLOT32_MAX_VALUE, SLOT32_MIN_VALUE};

pub type CatalogPrerollerLayers = SingletonLayer<CatalogPrerollerLayer>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCatInfo {
    pub asset_id_left: Bytes32,
    pub asset_id_right: Bytes32,

    pub owner_puzzle_hash: Bytes32,
    pub metadata: CatNftMetadata,
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
pub struct CatalogPrerollerInfo {
    pub launcher_id: Bytes32,
    pub to_launch: Vec<AddCat>,
    pub next_puzzle_hash: Bytes32,

    pub royalty_puzzle_hash: Bytes32,
    pub royalty_ten_thousandths: u16,
}

pub fn get_hint(memos: &[Bytes]) -> Option<Bytes32> {
    let hint = memos.first()?;

    let Ok(hint) = hint.try_into() else {
        return None;
    };

    Some(hint)
}

impl CatalogPrerollerInfo {
    pub fn new(
        launcher_id: Bytes32,
        to_launch: Vec<AddCat>,
        next_puzzle_hash: Bytes32,
        royalty_puzzle_hash: Bytes32,
        royalty_ten_thousandths: u16,
    ) -> Self {
        Self {
            launcher_id,
            to_launch,
            next_puzzle_hash,
            royalty_puzzle_hash,
            royalty_ten_thousandths,
        }
    }

    pub fn parse(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(layers) = CatalogPrerollerLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        Self::from_layers(layers)
    }

    pub fn from_layers(layers: CatalogPrerollerLayers) -> Result<Option<Self>, DriverError> {
        let Some(Condition::CreateCoin(recreate_condition)) = layers
            .inner_puzzle
            .base_conditions
            .as_ref()
            .iter()
            .find(|c| {
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
            .base_conditions
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
            royalty_puzzle_hash: layers.inner_puzzle.royalty_address_hash,
            royalty_ten_thousandths: layers.inner_puzzle.trade_price_percentage,
        }))
    }

    pub fn get_prelaunchers_and_slots(
        allocator: &mut Allocator,
        to_launch: Vec<AddCat>,
        my_launcher_id: Bytes32,
        my_coin_id: Bytes32,
    ) -> Result<Vec<(AddCat, UniquenessPrelauncher<Bytes32>, Slot)>, DriverError> {
        let mut res = Vec::with_capacity(to_launch.len());

        for add_cat in to_launch {
            let asset_id = add_cat.asset_id;
            let Some((asset_id_left, asset_id_right)) = add_cat
                .info
                .as_ref()
                .map(|i| (i.asset_id_left, i.asset_id_right))
            else {
                return Err(DriverError::Custom(
                    "Missing CAT launch info (required to build slot)".to_string(),
                ));
            };

            // uniqueness prelauncher
            let uniq_prelauncher =
                UniquenessPrelauncher::<Bytes32>::new(allocator, my_coin_id, asset_id)?;

            // slot
            let value = CatalogSlotValue::new(asset_id, asset_id_left, asset_id_right);
            let slot = Slot::new(my_coin_id, my_launcher_id, value.tree_hash().into())?;

            res.push((add_cat, uniq_prelauncher, slot));
        }

        Ok(res)
    }

    pub fn get_eve_cat_nft_p2_layer(
        ctx: &mut SpendContext,
        metadata: CatNftMetadata,
        owner_puzzle_hash: Bytes32,
        launcher_id: Bytes32,
    ) -> Result<ConditionsLayer<NodePtr>, DriverError> {
        let target_nft_metadata_ptr = metadata.to_clvm(&mut ctx.allocator)?;
        let any_metadata_updater_ptr =
            Program::new(Bytes::new(ANY_METADATA_UPDATER.into())).to_clvm(&mut ctx.allocator)?;

        Ok(ConditionsLayer::new(
            Conditions::new()
                .create_coin(owner_puzzle_hash, 1, vec![launcher_id.into()])
                .update_nft_metadata(any_metadata_updater_ptr, target_nft_metadata_ptr),
        ))
    }

    pub fn into_layers(self) -> Result<CatalogPrerollerLayers, DriverError> {
        let mut base_conditions =
            Conditions::new().create_coin(self.next_puzzle_hash, 1, vec![self.launcher_id.into()]);
        let mut nft_infos = Vec::with_capacity(self.to_launch.len());

        let fake_ctx = &mut SpendContext::new();
        for add_cat in self.to_launch {
            let asset_id = add_cat.asset_id;
            let Some(info) = add_cat.info else {
                return Err(DriverError::Custom(
                    "Missing CAT launch info (required to build slot)".to_string(),
                ));
            };

            // create slot
            let slot_value =
                CatalogSlotValue::new(asset_id, info.asset_id_left, info.asset_id_right);
            let slot_value_hash: Bytes32 = slot_value.tree_hash().into();

            base_conditions = base_conditions.create_coin(
                Slot::puzzle_hash(self.launcher_id, slot_value_hash).into(),
                0,
                vec![asset_id.into()],
            );

            let min_value = Bytes32::new(SLOT32_MIN_VALUE);
            if info.asset_id_left == min_value {
                // also launch min value slot
                base_conditions = base_conditions.create_coin(
                    Slot::puzzle_hash(
                        self.launcher_id,
                        CatalogSlotValue::new(min_value, min_value, asset_id)
                            .tree_hash()
                            .into(),
                    )
                    .into(),
                    0,
                    vec![min_value.into()],
                );
            }

            let max_value = Bytes32::new(SLOT32_MAX_VALUE);
            if info.asset_id_right == max_value {
                // also launch max value slot
                base_conditions = base_conditions.create_coin(
                    Slot::puzzle_hash(
                        self.launcher_id,
                        CatalogSlotValue::new(max_value, asset_id, max_value)
                            .tree_hash()
                            .into(),
                    )
                    .into(),
                    0,
                    vec![max_value.into()],
                );
            }

            // create uniqueness prelauncher
            base_conditions = base_conditions.create_coin(
                UniquenessPrelauncher::<()>::puzzle_hash(asset_id.tree_hash()).into(),
                0,
                vec![],
            );

            // NFT info
            let eve_nft_inner_layer = CatalogPrerollerInfo::get_eve_cat_nft_p2_layer(
                fake_ctx,
                info.metadata,
                info.owner_puzzle_hash,
                self.launcher_id,
            )?;
            let eve_nft_inner_puzzle = eve_nft_inner_layer.construct_puzzle(fake_ctx)?;
            let eve_nft_inner_puzzle_hash = fake_ctx.tree_hash(eve_nft_inner_puzzle);

            nft_infos.push(CatalogPrerollerNftInfo {
                eve_nft_inner_puzzle_hash: eve_nft_inner_puzzle_hash.into(),
                asset_id_hash: asset_id.tree_hash().into(),
            })
        }

        Ok(SingletonLayer::new(
            self.launcher_id,
            CatalogPrerollerLayer::new(
                nft_infos,
                base_conditions,
                self.royalty_puzzle_hash,
                self.royalty_ten_thousandths,
            ),
        ))
    }

    pub fn inner_puzzle_hash(self, ctx: &mut SpendContext) -> Result<TreeHash, DriverError> {
        let layers = self.into_layers()?;
        let inner_puzzle = layers.inner_puzzle.construct_puzzle(ctx)?;

        Ok(ctx.tree_hash(inner_puzzle))
    }
}

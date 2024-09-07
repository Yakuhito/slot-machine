use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes, Bytes32},
    puzzles::nft::NFT_ROYALTY_TRANSFER_PUZZLE_HASH,
};
use chia_wallet_sdk::{
    Condition, Conditions, DriverError, Launcher, Layer, Puzzle, SingletonLayer, SpendContext,
};
use clvm_traits::ToClvm;
use clvmr::{Allocator, NodePtr};

use crate::{CatNftMetadata, ConditionsLayer, SpendContextExt, ANY_METADATA_UPDATER_HASH};

use super::{CatalogSlotValue, Slot, UniquenessPrelauncher};

pub type CatalogPrerollerLayers = SingletonLayer<ConditionsLayer<NodePtr>>;

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
}

pub fn get_hint(memos: &[Bytes]) -> Option<Bytes32> {
    let hint = memos.first()?;

    let Ok(hint) = hint.try_into() else {
        return None;
    };

    Some(hint)
}

impl CatalogPrerollerInfo {
    pub fn new(launcher_id: Bytes32, to_launch: Vec<AddCat>, next_puzzle_hash: Bytes32) -> Self {
        Self {
            launcher_id,
            to_launch,
            next_puzzle_hash,
        }
    }

    pub fn parse(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(layers) = CatalogPrerollerLayers::parse_puzzle(allocator, puzzle)? else {
            return Ok(None);
        };

        Self::from_layers(layers)
    }

    pub fn from_layers(layers: CatalogPrerollerLayers) -> Result<Option<Self>, DriverError> {
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

    pub fn into_layers(
        self,
        allocator: &mut Allocator,
        my_coin_id: Bytes32,
    ) -> Result<CatalogPrerollerLayers, DriverError> {
        let mut conditions =
            Conditions::new().create_coin(self.next_puzzle_hash, 1, vec![self.launcher_id.into()]);

        let fake_ctx = &mut SpendContext::new();
        for (add_cat, uniq_prelauncher, slot) in CatalogPrerollerInfo::get_prelaunchers_and_slots(
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

            // uniqueness prelauncher was created - but we ned to assert that the correct NFT was acutally created
            let launcher = Launcher::new(uniq_prelauncher.coin.coin_id(), 1);

            let target_nft_metadata_ptr = info.metadata.to_clvm(&mut fake_ctx.allocator)?;
            let eve_nft_p2_layer = ConditionsLayer::new(
                Conditions::new()
                    .create_coin(info.owner_puzzle_hash, 1, vec![self.launcher_id.into()])
                    .update_nft_metadata(fake_ctx.any_metadata_updater()?, target_nft_metadata_ptr),
            );
            let eve_nft_p2_puzzle_ptr = eve_nft_p2_layer.construct_puzzle(fake_ctx)?;
            let eve_nft_p2_hash = fake_ctx.tree_hash(eve_nft_p2_puzzle_ptr);

            let (_, nft) = launcher.mint_eve_nft(
                fake_ctx,
                eve_nft_p2_hash.into(),
                (),
                ANY_METADATA_UPDATER_HASH.into(),
                NFT_ROYALTY_TRANSFER_PUZZLE_HASH.into(),
                100,
            )?;

            conditions = conditions
                .create_coin(
                    uniq_prelauncher.coin.puzzle_hash,
                    uniq_prelauncher.coin.amount,
                    vec![],
                )
                .create_coin(
                    slot.coin.puzzle_hash,
                    slot.coin.amount,
                    vec![asset_id.into()],
                )
                .assert_concurrent_spend(nft.coin.coin_id());
        }

        Ok(SingletonLayer::new(
            self.launcher_id,
            ConditionsLayer::new(conditions),
        ))
    }

    pub fn inner_puzzle_hash(
        self,
        ctx: &mut SpendContext,
        my_coin_id: Bytes32,
    ) -> Result<TreeHash, DriverError> {
        let layers = self.into_layers(&mut ctx.allocator, my_coin_id)?;
        let inner_puzzle = layers.inner_puzzle.construct_puzzle(ctx)?;

        Ok(ctx.tree_hash(inner_puzzle))
    }
}

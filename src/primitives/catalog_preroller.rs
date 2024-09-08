use chia::{
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, Proof},
};
use chia_wallet_sdk::{DriverError, Layer, Spend, SpendContext};
use clvmr::NodePtr;

use crate::{CatalogPrerollerSolution, ANY_METADATA_UPDATER_HASH};

use super::{CatalogPrerollerInfo, CatalogSlotValue, Slot};

/// Used to create slots & then transition to either a new
/// slot launcher or the main logic singleton innerpuzzle
#[derive(Debug, Clone)]
#[must_use]
pub struct CatalogPreroller {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogPrerollerInfo,
}

impl CatalogPreroller {
    pub fn new(coin: Coin, proof: Proof, info: CatalogPrerollerInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        royalty_puzzle_hash: Bytes32,
    ) -> Result<Vec<Slot<CatalogSlotValue>>, DriverError> {
        let res = CatalogPrerollerInfo::get_prelaunchers_and_slots(
            &mut ctx.allocator,
            self.info.to_launch.clone(),
            self.info.launcher_id,
            self.coin.coin_id(),
        )?;

        res.0
            .into_iter()
            .try_for_each(|(add_cat, uniqueness_prelauncher)| {
                let cat_nft_launcher = uniqueness_prelauncher.spend(ctx)?;

                let Some(info) = add_cat.info else {
                    return Err(DriverError::Custom(
                        "Missing CAT launch info (required to build puzzle)".to_string(),
                    ));
                };

                let eve_cat_nft_inner_puzzle = CatalogPrerollerInfo::get_eve_cat_nft_p2_layer(
                    ctx,
                    info.metadata,
                    info.owner_puzzle_hash,
                )?
                .construct_puzzle(ctx)?;
                let eve_cat_nft_p2_puzzle_hash = ctx.tree_hash(eve_cat_nft_inner_puzzle);

                let (_, eve_cat_nft) = cat_nft_launcher.mint_eve_nft(
                    ctx,
                    eve_cat_nft_p2_puzzle_hash.into(),
                    0,
                    ANY_METADATA_UPDATER_HASH.into(),
                    royalty_puzzle_hash,
                    self.info.royalty_ten_thousandths,
                )?;

                eve_cat_nft.spend(
                    ctx,
                    Spend {
                        puzzle: eve_cat_nft_inner_puzzle,
                        solution: NodePtr::NIL,
                    },
                )?;

                Ok(())
            })?;

        let slots = res.1;
        let layers = self.info.into_layers()?;

        let puzzle = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: CatalogPrerollerSolution {
                    my_coin_id: self.coin.coin_id(),
                },
            },
        )?;

        ctx.spend(self.coin, Spend::new(puzzle, solution))?;

        Ok(slots)
    }
}

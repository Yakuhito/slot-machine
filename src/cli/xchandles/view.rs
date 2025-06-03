use chia::clvm_utils::ToTreeHash;
use chia_wallet_sdk::{driver::SpendContext, utils::Address};

use crate::{
    get_coinset_client, get_prefix, hex_string_to_bytes32, parse_amount, quick_sync_xchandles,
    CliError, Db, DefaultCatMakerArgs, XchandlesExponentialPremiumRenewPuzzleArgs,
    XchandlesFactorPricingPuzzleArgs,
};

#[allow(clippy::too_many_arguments)]
pub async fn xchandles_view(
    launcher_id_str: String,
    testnet11: bool,
    payment_asset_id_str: Option<String>,
    payment_cat_base_price_str: Option<String>,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);

    print!("Syncing registry... ");
    let mut db = Db::new(false).await?;
    let registry = quick_sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?;
    println!("done.\n");

    println!("State:");
    println!(
        "  CAT maker puzzle hash: {}",
        registry.info.state.cat_maker_puzzle_hash
    );
    if let Some(payment_asset_id) = payment_asset_id_str {
        let payment_asset_id = hex_string_to_bytes32(&payment_asset_id)?;
        if registry.info.state.cat_maker_puzzle_hash
            == DefaultCatMakerArgs::curry_tree_hash(payment_asset_id.tree_hash().into()).into()
        {
            println!(
                "    Payment asset id: {} (VERIFIED)",
                hex::encode(payment_asset_id)
            );
        } else {
            return Err(CliError::Custom(
                "Payment asset id hint is wrong".to_string(),
            ));
        }
    }
    println!(
        "  Pricing puzzle hash: {}",
        hex::encode(registry.info.state.pricing_puzzle_hash)
    );
    println!(
        "  Expired handle pricing puzzle hash: {}",
        hex::encode(registry.info.state.expired_handle_pricing_puzzle_hash)
    );
    if let Some(payment_cat_base_price) = payment_cat_base_price_str {
        let payment_cat_base_price = parse_amount(&payment_cat_base_price, true)?;
        if registry.info.state.pricing_puzzle_hash
            == XchandlesFactorPricingPuzzleArgs::curry_tree_hash(payment_cat_base_price).into()
            && registry.info.state.expired_handle_pricing_puzzle_hash
                == XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(
                    payment_cat_base_price,
                    1000,
                )
                .into()
        {
            println!(
                "    Payment CAT base price: {} mojos (VERIFIED)",
                payment_cat_base_price
            );
        } else {
            return Err(CliError::Custom(
                "Payment CAT base price hint is wrong".to_string(),
            ));
        }
    }

    println!("Constants:");
    println!(
        "  Launcher ID: {}",
        hex::encode(registry.info.constants.launcher_id)
    );
    println!(
        "  Precommit payout address: {}",
        Address::new(
            registry.info.constants.precommit_payout_puzzle_hash,
            get_prefix(testnet11)
        )
        .encode()?
    );
    println!(
        "  Relative block height: {}",
        registry.info.constants.relative_block_height
    );
    println!(
        "  Price singleton launcher ID: {}",
        hex::encode(registry.info.constants.price_singleton_launcher_id)
    );

    Ok(())
}

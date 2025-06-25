use chia::protocol::CoinSpend;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::SpendContext,
};

use crate::{CatalogRegistry, CatalogRegistryConstants, CliError};

pub async fn quick_sync_catalog(
    client: &CoinsetClient,
    ctx: &mut SpendContext,
    constants: CatalogRegistryConstants,
) -> Result<CatalogRegistry, CliError> {
    let mut records = client
        .get_coin_records_by_hint(constants.launcher_id, None, None, Some(false))
        .await?
        .coin_records
        .ok_or(CliError::Custom(
            "No unspent CATalog records found".to_string(),
        ))?
        .into_iter();

    let mut coin_spend: Option<CoinSpend> = None;
    loop {
        let Some(next_coin_record) = records.next() else {
            break;
        };
        if next_coin_record.spent_block_index > 0 {
            continue;
        }

        let next_spend = client
            .get_puzzle_and_solution(
                next_coin_record.coin.parent_coin_info,
                Some(next_coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(
                next_coin_record.coin.parent_coin_info,
            ))?;

        let mut temp_ctx = SpendContext::new();
        let catalog_maybe =
            CatalogRegistry::from_parent_spend(&mut temp_ctx, &next_spend, constants)?;
        if catalog_maybe.is_some() {
            coin_spend = Some(next_spend);
            break;
        }
    }

    let Some(coin_spend) = coin_spend else {
        return Err(CliError::Custom("Could not find CATalog coin".to_string()));
    };
    let on_chain_catalog = CatalogRegistry::from_parent_spend(ctx, &coin_spend, constants)?.ok_or(
        CliError::Custom("Could not parse CATalog spend".to_string()),
    )?;

    let Some(mut mempool_items) = client
        .get_mempool_items_by_coin_name(on_chain_catalog.coin.coin_id())
        .await?
        .mempool_items
    else {
        return Ok(on_chain_catalog);
    };

    if mempool_items.is_empty() {
        return Ok(on_chain_catalog);
    }

    let mempool_item = mempool_items.remove(0);
    let mut catalog = on_chain_catalog;
    loop {
        let Some(catalog_spend) = mempool_item
            .spend_bundle
            .coin_spends
            .iter()
            .find(|c| c.coin.coin_id() == catalog.coin.coin_id())
        else {
            break;
        };

        let Some(new_catalog) = CatalogRegistry::from_spend(ctx, catalog_spend, constants)? else {
            break;
        };
        catalog = new_catalog;
    }

    Ok(catalog)
}

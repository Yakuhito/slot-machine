use chia_protocol::CoinSpend;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{CatalogRegistry, CatalogRegistryConstants, SpendContext},
};

use crate::CliError;

pub async fn quick_sync_catalog(
    client: &CoinsetClient,
    ctx: &mut SpendContext,
    constants: CatalogRegistryConstants,
) -> Result<CatalogRegistry, CliError> {
    let records = client
        .get_coin_records_by_hint(constants.launcher_id, None, None, Some(false), None)
        .await?
        .coin_records
        .ok_or(CliError::Custom(
            "No unspent CATalog records found".to_string(),
        ))?
        .into_iter();

    let mut coin_spend: Option<CoinSpend> = None;
    for coin_record in records {
        if coin_record.spent_block_index > 0 {
            continue;
        }

        let next_spend = client
            .get_puzzle_and_solution(
                coin_record.coin.parent_coin_info,
                Some(coin_record.confirmed_block_index),
            )
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(coin_record.coin.parent_coin_info))?;

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

    if let Some(mempool_items) = client
        .get_mempool_items_by_coin_name(coin_spend.coin.coin_id())
        .await?
        .mempool_items
    {
        if !mempool_items.is_empty() {
            if let Some(new_catalog) = CatalogRegistry::from_mempool_item(
                ctx,
                mempool_items[0].spend_bundle.clone(),
                constants,
            )? {
                return Ok(new_catalog);
            }
        }
    }

    let on_chain_catalog = CatalogRegistry::from_parent_spend(ctx, &coin_spend, constants)?.ok_or(
        CliError::Custom("Could not parse CATalog spend".to_string()),
    )?;

    Ok(on_chain_catalog)
}

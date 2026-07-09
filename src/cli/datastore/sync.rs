use chia_protocol::Bytes32;
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{DataStore, DataStoreMetadata, DelegatedPuzzle, SpendContext},
};

use crate::CliError;

pub async fn sync_datastore(
    client: &CoinsetClient,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    delegated_puzzles: &[DelegatedPuzzle],
) -> Result<DataStore<DataStoreMetadata>, CliError> {
    let mut records = client
        .get_coin_records_by_hint(launcher_id, None, None, Some(false), None)
        .await?
        .coin_records
        .ok_or(CliError::Custom(
            "No unspent coin records found".to_string(),
        ))?;

    while !records.is_empty() {
        let coin_record = records.remove(0);
        if coin_record.spent {
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

        if let Ok(Some(on_chain_datastore)) =
            DataStore::from_spend(ctx, &next_spend, delegated_puzzles).map_err(CliError::Driver)
        {
            if on_chain_datastore.info.launcher_id == launcher_id {
                if let Some(mempool_items) = client
                    .get_mempool_items_by_coin_name(on_chain_datastore.coin.coin_id())
                    .await?
                    .mempool_items
                {
                    if let Some(mempool_item) = mempool_items.first() {
                        for cs in &mempool_item.spend_bundle.coin_spends {
                            if let Ok(Some(candidate)) =
                                DataStore::from_spend(ctx, cs, delegated_puzzles)
                                    .map_err(CliError::Driver)
                            {
                                if candidate.info.launcher_id == launcher_id
                                    && candidate.coin.coin_id() == coin_record.coin.coin_id()
                                {
                                    return Ok(candidate);
                                }
                            }
                        }
                    }
                }

                return Ok(on_chain_datastore);
            }
        }
    }

    let launcher_coin_record = client
        .get_coin_record_by_name(launcher_id)
        .await?
        .coin_record
        .ok_or(CliError::CoinNotFound(launcher_id))?;
    let launcher_coin_spend = client
        .get_puzzle_and_solution(launcher_id, Some(launcher_coin_record.spent_block_index))
        .await?
        .coin_solution
        .ok_or(CliError::CoinNotSpent(launcher_id))?;

    DataStore::from_spend(ctx, &launcher_coin_spend, delegated_puzzles)
        .map_err(CliError::Driver)?
        .ok_or(CliError::Custom(
            "Could not parse datastore launcher spend".to_string(),
        ))
}

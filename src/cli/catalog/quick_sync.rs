use chia::protocol::CoinSpend;
use chia_wallet_sdk::{ChiaRpcClient, CoinsetClient, Puzzle, SpendContext};
use clvmr::serde::node_from_bytes;

use crate::{CatalogRegistry, CatalogRegistryConstants, CliError};

const BATCH_SIZE: usize = 10;

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
        let puzzle_ptr = node_from_bytes(&mut temp_ctx.allocator, &next_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(&mut temp_ctx.allocator, puzzle_ptr);
        let solution_ptr = node_from_bytes(&mut temp_ctx.allocator, &next_spend.solution)?;

        let catalog_maybe = if let Ok(resp) = CatalogRegistry::from_parent_spend(
            &mut temp_ctx.allocator,
            next_spend.coin,
            puzzle,
            solution_ptr,
            constants,
        ) {
            resp
        } else {
            None
        };
        if catalog_maybe.is_some() {
            coin_spend = Some(next_spend);
            break;
        }
    }

    if let Some(coin_spend) = coin_spend {
        let puzzle_ptr = node_from_bytes(&mut ctx.allocator, &coin_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(&mut ctx.allocator, puzzle_ptr);
        let solution_ptr = node_from_bytes(&mut ctx.allocator, &coin_spend.solution)?;

        CatalogRegistry::from_parent_spend(
            &mut ctx.allocator,
            coin_spend.coin,
            puzzle,
            solution_ptr,
            constants,
        )?
        .ok_or(CliError::Custom(
            "Tried to unwrap CATalog but couldn't".to_string(),
        ))
    } else {
        Err(CliError::Custom("Could not find CATalog coin".to_string()))
    }
}

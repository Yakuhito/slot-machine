use chia::protocol::{Bytes32, CoinSpend};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{Puzzle, SpendContext},
};
use clvmr::serde::node_from_bytes;

use crate::{CliError, Db, XchandlesRegistry};

pub async fn quick_sync_xchandles(
    client: &CoinsetClient,
    db: &mut Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
) -> Result<XchandlesRegistry, CliError> {
    let constants = if let Some(c) = db.get_xchandles_configuration(ctx, launcher_id).await? {
        c
    } else {
        let coin_record = client
            .get_coin_record_by_name(launcher_id)
            .await?
            .coin_record
            .ok_or(CliError::CoinNotFound(launcher_id))?;

        let coin_spend = client
            .get_puzzle_and_solution(launcher_id, Some(coin_record.spent_block_index))
            .await?
            .coin_solution
            .ok_or(CliError::CoinNotSpent(launcher_id))?;

        let launcher_solution = ctx.alloc(&coin_spend.solution)?;
        let Some((eve_registry, _initial_slots, _initial_asset_id, _initial_base_price)) =
            XchandlesRegistry::from_launcher_solution(ctx, coin_record.coin, launcher_solution)?
        else {
            return Err(CliError::Custom(
                "Could not parse XCHandles launcher spend".to_string(),
            ));
        };

        let c = eve_registry.info.constants;
        db.save_xchandles_configuration(ctx, c).await?;

        c
    };

    let mut records = client
        .get_coin_records_by_hint(launcher_id, None, None, Some(false))
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
        let puzzle_ptr = node_from_bytes(&mut temp_ctx, &next_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(&temp_ctx, puzzle_ptr);
        let solution_ptr = node_from_bytes(&mut temp_ctx, &next_spend.solution)?;

        let xchandles_maybe = XchandlesRegistry::from_parent_spend(
            &mut temp_ctx,
            next_spend.coin,
            puzzle,
            solution_ptr,
            constants,
        )
        .unwrap_or_default();
        if xchandles_maybe.is_some() {
            coin_spend = Some(next_spend);
            break;
        }
    }

    if let Some(coin_spend) = coin_spend {
        let puzzle_ptr = node_from_bytes(ctx, &coin_spend.puzzle_reveal)?;
        let puzzle = Puzzle::parse(ctx, puzzle_ptr);
        let solution_ptr = node_from_bytes(ctx, &coin_spend.solution)?;

        XchandlesRegistry::from_parent_spend(ctx, coin_spend.coin, puzzle, solution_ptr, constants)?
            .ok_or(CliError::Custom(
                "Tried to unwrap XCHandles but couldn't".to_string(),
            ))
    } else {
        Err(CliError::Custom(
            "Could not find XCHandles registry".to_string(),
        ))
    }
}

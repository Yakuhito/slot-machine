use chia::protocol::{Bytes32, CoinSpend};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{SpendContext, XchandlesRegistry},
};

use crate::{mempool_registry_maybe, CliError, Db};

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
        if let Ok(Some(_xchandles_maybe)) =
            XchandlesRegistry::from_parent_spend(&mut temp_ctx, &next_spend, constants)
        {
            coin_spend = Some(next_spend);
            break;
        }
    }

    let coin_spend = coin_spend.ok_or(CliError::Custom(
        "Could not find XCHandles registry".to_string(),
    ))?;

    let on_chain_registry = XchandlesRegistry::from_parent_spend(ctx, &coin_spend, constants)?
        .ok_or(CliError::Custom(
            "Could not parse latest XCHandles registry".to_string(),
        ))?;

    mempool_registry_maybe(ctx, on_chain_registry, client).await
}

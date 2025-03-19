use chia::protocol::Bytes32;
use chia_wallet_sdk::{
    ChiaRpcClient, CoinRecord, CoinsetClient, DriverError, Puzzle, SpendContext,
};

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CatalogSlotValue, CliError, Db,
    CATALOG_LAST_UNSPENT_COIN,
};

pub async fn sync_catalog(
    client: &CoinsetClient,
    db: &Db,
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    constants: CatalogRegistryConstants,
) -> Result<CatalogRegistry, CliError> {
    let last_unspent_coin_id_str = db.get_value_by_key(CATALOG_LAST_UNSPENT_COIN).await?;

    let mut last_spent_coin_id: Bytes32 =
        if let Some(last_unspent_coin_id_str) = last_unspent_coin_id_str {
            let last_unspent_coin_id = Bytes32::new(
                hex::decode(last_unspent_coin_id_str)?
                    .to_vec()
                    .try_into()
                    .unwrap(),
            );

            let coin_record_response = client.get_coin_record_by_name(last_unspent_coin_id).await?;
            if let Some(coin_record) = coin_record_response.coin_record {
                coin_record.coin.parent_coin_info
            } else {
                return Err(CliError::Driver(DriverError::Custom(
                    "Could not find latest unspent coin".to_string(),
                )));
            }
        } else {
            launcher_id
        };

    let mut slots = db
        .get_slots::<CatalogSlotValue>(&mut ctx.allocator, launcher_id, 0)
        .await?;

    let mut coin_record_response = client.get_coin_record_by_name(last_spent_coin_id).await?;
    let mut coin_record =
        coin_record_response
            .coin_record
            .ok_or(CliError::Driver(DriverError::Custom(
                "Could not find latest spent coin".to_string(),
            )))?;
    if !coin_record.spent {
        return Err(CliError::Driver(DriverError::Custom(
            "CATalog coin that should be spent is unspent".to_string(),
        )));
    }

    let mut puzzle_and_solution_resp = client
        .get_puzzle_and_solution(
            coin_record.coin.coin_id(),
            Some(coin_record.confirmed_block_index),
        )
        .await?;
    let mut coin_spend = puzzle_and_solution_resp
        .coin_solution
        .ok_or(CliError::Driver(DriverError::Custom(
            "Could not find puzzle and solution for a coin that should be spent".to_string(),
        )))?;

    let puzzle_ptr = ctx.alloc(&coin_spend.puzzle_reveal)?;
    let parent_puzzle = Puzzle::parse(&ctx.allocator, puzzle_ptr);
    let solution_ptr = ctx.alloc(&coin_spend.solution)?;
    let mut catalog = CatalogRegistry::from_parent_spend(
        &mut ctx.allocator,
        coin_record.coin,
        parent_puzzle,
        solution_ptr,
        constants,
    )?;

    db.save_key_value(
        CATALOG_LAST_UNSPENT_COIN,
        &hex::encode(catalog.coin.coin_id().to_vec()),
    )
    .await?;
    todo!("todo")
}

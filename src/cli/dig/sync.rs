use chia_wallet_sdk::{ChiaRpcClient, SpendContext};

use crate::{
    get_coinset_client, hex_string_to_bytes32, parse_amount, sync_distributor, CliError, Db,
};

pub async fn dig_sync(
    launcher_id_str: String,
    update_time: Option<u64>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();

    let update_time = if let Some(update_time) = update_time {
        update_time
    } else {
        println!("Fetching latest transaction block timestamp...");
        let blockchain_state = client
            .get_blockchain_state()
            .await?
            .blockchain_state
            .ok_or(CliError::Custom(
                "Could not fetch blockchain state".to_string(),
            ))?;

        if let Some(t) = blockchain_state.peak.timestamp {
            t
        } else {
            let mut height = blockchain_state.peak.height - 1;
            let mut block;
            loop {
                println!("Fetching block record #{}...", height);
                block = client
                    .get_block_record_by_height(height)
                    .await?
                    .block_record
                    .ok_or(CliError::Custom(format!(
                        "Could not fetch block record #{}",
                        height
                    )))?;

                if block.timestamp.is_some() {
                    break;
                }
                height -= 1;
            }

            block.timestamp.unwrap()
        }
    };
    println!("Using update time: {}", update_time);

    let mut distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    todo!();

    Ok(())
}

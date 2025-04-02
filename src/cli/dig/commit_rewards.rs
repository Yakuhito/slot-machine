use chia_wallet_sdk::SpendContext;

use crate::{
    get_coinset_client, hex_string_to_bytes32, parse_amount, sync_distributor, CliError, Db,
};

pub async fn dig_commit_rewards(
    launcher_id_str: String,
    reward_amount_str: String,
    clawback_address: String,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let reward_amount = parse_amount(&reward_amount_str, true)?;
    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();
    let distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    println!("Done!");
    Ok(())
}

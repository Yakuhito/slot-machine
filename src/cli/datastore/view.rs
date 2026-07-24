use chia_wallet_sdk::{
    driver::{DelegatedPuzzle, SpendContext},
    utils::Address,
};

use crate::{get_coinset_client, get_prefix, hex_string_to_bytes32, sync_datastore, CliError};

use super::oracle_delegated_puzzles;

pub async fn datastore_view(launcher_id_str: String, testnet11: bool) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    let mut ctx = SpendContext::new();
    let client = get_coinset_client(testnet11);

    print!("Syncing datastore... ");
    let datastore =
        sync_datastore(&client, &mut ctx, launcher_id, &oracle_delegated_puzzles()).await?;
    println!("done.\n");

    println!("Datastore:");
    println!("  Launcher ID: {}", hex::encode(datastore.info.launcher_id));
    println!(
        "  Latest coin id: {}",
        hex::encode(datastore.coin.coin_id())
    );
    println!(
        "  Coin puzzle hash: {}",
        hex::encode(datastore.coin.puzzle_hash)
    );
    println!("  Coin amount: {}", datastore.coin.amount);
    println!(
        "  Owner address: {}",
        Address::new(datastore.info.owner_puzzle_hash, get_prefix(testnet11)).encode()?
    );

    let metadata = &datastore.info.metadata;
    println!("Metadata:");
    println!("  root_hash: {}", hex::encode(metadata.root_hash));
    println!("  label: {}", metadata.label.as_deref().unwrap_or("(none)"));
    println!(
        "  description: {}",
        metadata.description.as_deref().unwrap_or("(none)")
    );

    println!("Delegated puzzles:");
    if datastore.info.delegated_puzzles.is_empty() {
        println!("  (none)");
    } else {
        for puzzle in &datastore.info.delegated_puzzles {
            match puzzle {
                DelegatedPuzzle::Admin(hash) => {
                    println!("  Admin: {}", hex::encode(hash));
                }
                DelegatedPuzzle::Writer(hash) => {
                    println!("  Writer inner puzzle hash: {}", hex::encode(hash));
                }
                DelegatedPuzzle::Oracle(fee_address, fee) => {
                    println!(
                        "  Oracle fee address puzzle hash: {}",
                        hex::encode(fee_address)
                    );
                    println!("  Oracle fee: {fee} mojos");
                }
            }
        }
    }

    Ok(())
}

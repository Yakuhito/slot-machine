use chia_wallet_sdk::{ChiaRpcClient, SpendContext};

use crate::{get_coinset_client, sync_catalog, CatalogRegistryConstants, CliError, Db};

pub async fn catalog_listen(testnet11: bool) -> Result<(), CliError> {
    let constants = CatalogRegistryConstants::get(testnet11);
    let client = get_coinset_client(testnet11);
    let mut db = Db::new().await?;

    println!("Syncing CATalog...");

    let mut catalog = {
        let mut ctx = SpendContext::new();
        sync_catalog(&client, &mut db, &mut ctx, constants).await?
    };

    println!("Now listening for changes :)");
    let mut retries = 0;
    loop {
        let coin_resp = client
            .get_coin_record_by_name(catalog.coin.coin_id())
            .await?;

        if let Some(coin_record) = coin_resp.coin_record {
            retries = 0;
            if coin_record.spent {
                println!(
                    "Latest CATalog coin was spent at height {} - syncing...",
                    coin_record.spent_block_index
                );
                {
                    let mut ctx = SpendContext::new();
                    catalog = sync_catalog(&client, &mut db, &mut ctx, constants).await?;
                }
            }
        } else {
            println!(
                "Weird - coin record not found. This was the {}th retry.",
                retries
            );
            retries += 1;

            if retries > 4 {
                break;
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }

    println!("Exiting...");
    Ok(())
}

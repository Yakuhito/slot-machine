use std::time::{SystemTime, UNIX_EPOCH};

use chia_wallet_sdk::{ChiaRpcClient, SpendContext};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use crate::{get_coinset_client, sync_catalog, CatalogRegistryConstants, CliError, Db};

#[derive(Debug, Deserialize)]
struct WebSocketMessage {
    #[serde(rename = "type")]
    message_type: String,
}

pub async fn catalog_listen(testnet11: bool) -> Result<(), CliError> {
    loop {
        match connect_websocket(testnet11).await {
            Ok(_resp) => (),
            Err(e) => {
                println!("WebSocket error: {}", e);
                println!("Reconnecting in 5 seconds...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn connect_websocket(testnet11: bool) -> Result<(), CliError> {
    let client = get_coinset_client(testnet11);
    let mut db = Db::new().await?;
    let mut ctx = SpendContext::new();
    let constants = CatalogRegistryConstants::get(testnet11);

    println!("Syncing CATalog (initial)...");
    let mut catalog = sync_catalog(&client, &mut db, &mut ctx, constants).await?;

    let ws_url = format!("{}/ws", client.base_url().replace("https://", "wss://"));
    println!("Connecting to WebSocket at {}", ws_url);

    let (ws_stream, _) = connect_async(ws_url)
        .await
        .map_err(|e| CliError::Custom(format!("Failed to connect: {}", e)))?;

    println!("WebSocket connected");

    let (mut _write, mut read) = ws_stream.split();
    let mut last_clear_time = SystemTime::now();

    while let Some(message) = read.next().await {
        match message {
            Ok(Message::Text(text)) => match serde_json::from_str::<WebSocketMessage>(&text) {
                Ok(msg) => {
                    if msg.message_type == "peak" {
                        let now = SystemTime::now();
                        println!(
                            "[{}] Received new peak",
                            now.duration_since(UNIX_EPOCH).unwrap().as_secs()
                        );

                        let coin_resp = client
                            .get_coin_record_by_name(catalog.coin.coin_id())
                            .await?;

                        if let Some(coin_record) = coin_resp.coin_record {
                            if coin_record.spent {
                                print!(
                                    "Latest CATalog coin was spent at height {}... ",
                                    coin_record.spent_block_index
                                );
                                let mut ctx = SpendContext::new();
                                catalog =
                                    sync_catalog(&client, &mut db, &mut ctx, constants).await?;
                                println!("synced :)")
                            }
                        } else {
                            return Err(CliError::Custom(
                                "Weird - coin record not found after peak update.".to_string(),
                            ));
                        }

                        if last_clear_time.elapsed().unwrap().as_secs() > 60 * 30 {
                            // 30 minutes in seconds
                            if let Some(current_blockchain_state) =
                                client.get_blockchain_state().await?.blockchain_state
                            {
                                print!("Clearing cache (every 30m)... ");
                                let cutoff = current_blockchain_state.peak.height - 128;
                                db.delete_slots_spent_before(cutoff).await?;
                                db.delete_singleton_coins_spent_before(cutoff).await?;
                                println!("done :)");
                                last_clear_time = now;
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Failed to parse message: {}, text: {}", e, text);
                }
            },
            Ok(Message::Close(_)) => {
                println!("WebSocket closed by server");
                break;
            }
            Err(e) => {
                println!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

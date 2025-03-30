use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::Query, http::StatusCode, response::Json, routing::get, Router};
use chia::{clvm_utils::ToTreeHash, protocol::Bytes32};
use chia_wallet_sdk::{ChiaRpcClient, SpendContext};
use clvmr::Allocator;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use crate::{get_coinset_client, sync_catalog, CatalogRegistryConstants, CliError, Db, Slot};

#[derive(Debug, Deserialize)]
struct WebSocketMessage {
    #[serde(rename = "type")]
    message_type: String,
}

#[derive(Debug, Deserialize)]
struct CatalogNeighborsQuery {
    asset_id: String,
}

#[derive(Debug, Serialize)]
struct CatalogNeighborResponse {
    asset_id: String,
    left_left_asset_id: String,
    right_right_asset_id: String,

    left_parent_parent_info: String,
    left_parent_inner_puzzle_hash: String,
    right_parent_parent_info: String,
    right_parent_inner_puzzle_hash: String,
}

struct AppState {
    db: Mutex<Db>,
    testnet11: bool,
}

pub async fn catalog_listen(testnet11: bool) -> Result<(), CliError> {
    let db = Db::new(true).await?;
    let constants = CatalogRegistryConstants::get(testnet11);
    let allocator = Allocator::new();

    let state = Arc::new(AppState {
        db: Mutex::new(db),
        testnet11,
    });

    // API
    let api_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = start_api_server(api_state).await {
            eprintln!("API server error: {}", e);
        }
    });

    // Updates
    loop {
        match connect_websocket(state.clone()).await {
            Ok(_resp) => (),
            Err(e) => {
                println!("WebSocket error: {}", e);
                println!("Reconnecting in 5 seconds...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn start_api_server(state: Arc<AppState>) -> Result<(), CliError> {
    // API routes
    let app = Router::new()
        .route("/", get(health_check))
        .route("/neighbors", get(get_neighbors))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("API server listening on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .map_err(|e| CliError::Custom(format!("API server error: {}", e)))
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}

async fn get_neighbors(
    Query(params): Query<CatalogNeighborsQuery>,
    state: axum::extract::State<Mutex<Db>>,
) -> Result<Json<CatalogNeighborResponse>, (StatusCode, String)> {
    let asset_id_bytes = hex::decode(&params.asset_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid asset_id: {}", e)))?;
    let asset_id = Bytes32::new(asset_id_bytes.try_into().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Asset ID must be 32 bytes".to_string(),
        )
    })?);

    let mut allocator = Allocator::new();

    let (left, right) = db
        .get_catalog_neighbors::<CatalogSlotValue>(
            &mut allocator,
            CatalogRegistryConstants::get(testnet11).launcher_id,
            asset_id,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get neighbors: {}", e),
            )
        })?;

    let response = CatalogNeighborResponse {
        asset_id: params.asset_id.clone(),
        left_left_asset_id: hex::encode(left.info.value.left_value.to_bytes()),
        right_right_asset_id: hex::encode(right.info.value.right_value.to_bytes()),

        left_parent_parent_info: hex::encode(left.proof.parent_parent_info.to_bytes()),
        left_parent_inner_puzzle_hash: hex::encode(left.proof.parent_inner_puzzle_hash.to_bytes()),
        right_parent_parent_info: hex::encode(right.proof.parent_parent_info.to_bytes()),
        right_parent_inner_puzzle_hash: hex::encode(
            right.proof.parent_inner_puzzle_hash.to_bytes(),
        ),
    };

    Ok(Json(response))
}

async fn connect_websocket(state: Arc<AppState>) -> Result<(), CliError> {
    println!("Syncing CATalog (initial)...");
    let client = get_coinset_client(state.testnet11);
    let mut catalog = {
        let mut db = state.db.lock().unwrap();
        let mut ctx = SpendContext::new();

        sync_catalog(&client, &mut db, &mut ctx, state.constants).await?;
    };

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

                                catalog = {
                                    let mut ctx = SpendContext::new();
                                    let mut db = state.db.lock().unwrap();
                                    sync_catalog(&client, &mut db, &mut ctx, state.constants)
                                        .await?;
                                };
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
                                {
                                    let mut db = state.db.lock().unwrap();
                                    db.delete_slots_spent_before(cutoff).await?;
                                    db.delete_singleton_coins_spent_before(cutoff).await?;
                                }
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

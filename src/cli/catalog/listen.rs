use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::debug_handler;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use axum::{http::StatusCode, routing::get, Json, Router};
use chia_wallet_sdk::{ChiaRpcClient, SpendContext};
use clvmr::Allocator;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tower_http::cors::CorsLayer;

use crate::{
    get_coinset_client, hex_string_to_bytes32, sync_catalog, CatalogRegistryConstants,
    CatalogSlotValue, CliError, Db,
};

#[derive(Debug, Deserialize)]
struct WebSocketMessage {
    #[serde(rename = "type")]
    message_type: String,
}

#[derive(Debug, Deserialize)]
struct CatalogNeighborsQuery {
    asset_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogNeighborResponse {
    pub asset_id: String,
    pub left_asset_id: String,
    pub right_asset_id: String,

    pub left_left_asset_id: String,
    pub right_right_asset_id: String,

    pub left_parent_parent_info: String,
    pub left_parent_inner_puzzle_hash: String,
    pub right_parent_parent_info: String,
    pub right_parent_inner_puzzle_hash: String,
}

#[derive(Clone)]
struct AppState {
    db: Arc<futures::lock::Mutex<Db>>,
    testnet11: bool,
}

pub async fn catalog_listen(testnet11: bool) -> Result<(), CliError> {
    let db = Db::new(true).await?;
    let db = Arc::new(futures::lock::Mutex::new(db));

    let state = AppState {
        db: Arc::clone(&db),
        testnet11,
    };

    // API
    let api_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = start_api_server(api_state).await {
            eprintln!("API server error: {}", e);
        }
    });

    // Updates
    loop {
        match connect_websocket(testnet11, Arc::clone(&db)).await {
            Ok(_resp) => (),
            Err(e) => {
                println!("WebSocket error: {}", e);
                println!("Reconnecting in 5 seconds...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn start_api_server(state: AppState) -> Result<(), CliError> {
    // API routes
    let app = Router::new()
        .route("/", get(health_check))
        .route("/neighbors", get(get_neighbors))
        .layer(
            CorsLayer::new()
                .allow_origin("*".parse::<HeaderValue>().unwrap())
                .allow_methods([Method::GET, Method::OPTIONS, Method::POST]),
        )
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("API server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}

impl IntoResponse for CliError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error: {}", self),
        )
            .into_response()
    }
}

#[debug_handler]
async fn get_neighbors(
    Query(params): Query<CatalogNeighborsQuery>,
    State(state): State<AppState>,
) -> Result<Json<CatalogNeighborResponse>, CliError> {
    let asset_id = hex_string_to_bytes32(&params.asset_id)?;

    let mut allocator = Allocator::new();

    let (left, right) = {
        let db = state.db.lock().await;

        db.get_catalog_neighbors::<CatalogSlotValue>(
            &mut allocator,
            CatalogRegistryConstants::get(state.testnet11).launcher_id,
            asset_id,
        )
        .await
    }?;

    let response = CatalogNeighborResponse {
        asset_id: params.asset_id.clone(),

        left_asset_id: hex::encode(left.info.value.asset_id.to_bytes()),
        right_asset_id: hex::encode(right.info.value.asset_id.to_bytes()),

        left_left_asset_id: hex::encode(left.info.value.neighbors.left_value.to_bytes()),
        right_right_asset_id: hex::encode(right.info.value.neighbors.right_value.to_bytes()),

        left_parent_parent_info: hex::encode(left.proof.parent_parent_info.to_bytes()),
        left_parent_inner_puzzle_hash: hex::encode(left.proof.parent_inner_puzzle_hash.to_bytes()),
        right_parent_parent_info: hex::encode(right.proof.parent_parent_info.to_bytes()),
        right_parent_inner_puzzle_hash: hex::encode(
            right.proof.parent_inner_puzzle_hash.to_bytes(),
        ),
    };

    Ok(Json(response))
}

async fn connect_websocket(
    testnet11: bool,
    db: Arc<futures::lock::Mutex<Db>>,
) -> Result<(), CliError> {
    println!("Syncing CATalog (initial)...");
    let client = get_coinset_client(testnet11);
    let constants = CatalogRegistryConstants::get(testnet11);

    let mut catalog = {
        let mut db = db.lock().await;
        let mut ctx = SpendContext::new();

        sync_catalog(&client, &mut db, &mut ctx, constants).await?
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
                                    let mut db = db.lock().await;
                                    sync_catalog(&client, &mut db, &mut ctx, constants).await?
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
                                    let mut db = db.lock().await;
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

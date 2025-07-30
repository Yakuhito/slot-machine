use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::debug_handler;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, Method};
use axum::{http::StatusCode, routing::get, Json, Router};
use chia::protocol::Bytes32;
use chia_wallet_sdk::coinset::ChiaRpcClient;
use chia_wallet_sdk::driver::{SpendContext, XchandlesRegistry};
use chia_wallet_sdk::types::puzzles::XchandlesSlotValue;
use clvmr::Allocator;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tower_http::cors::CorsLayer;

use crate::{get_coinset_client, hex_string_to_bytes32, sync_xchandles, CliError, Db};

#[derive(Debug, Deserialize)]
struct WebSocketMessage {
    #[serde(rename = "type")]
    message_type: String,
}

#[derive(Debug, Deserialize)]
struct XchandlesNeighborsQuery {
    launcher_id: String,
    handle_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct XchandlesNeighborsResponse {
    pub handle_hash: String,
    pub left_handle_hash: String,
    pub right_handle_hash: String,

    pub left_left_handle_hash: String,
    pub left_expiration: u64,
    pub left_owner_launcher_id: String,
    pub left_resolved_data: String,

    pub right_right_handle_hash: String,
    pub right_expiration: u64,
    pub right_owner_launcher_id: String,
    pub right_resolved_data: String,

    pub left_parent_parent_info: String,
    pub left_parent_inner_puzzle_hash: String,
    pub left_parent_amount: u64,
    pub right_parent_parent_info: String,
    pub right_parent_inner_puzzle_hash: String,
    pub right_parent_amount: u64,
}

#[derive(Clone)]
struct AppState {
    db: Arc<futures::lock::Mutex<Db>>,
}

pub async fn xchandles_listen(launcher_ids: String, testnet11: bool) -> Result<(), CliError> {
    let db = Db::new(true).await?;
    let db = Arc::new(futures::lock::Mutex::new(db));

    let state = AppState {
        db: Arc::clone(&db),
    };

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // API
    let api_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = start_api_server(api_state).await {
            eprintln!("API server error: {}", e);
        }
    });

    let launcher_ids = launcher_ids
        .split(',')
        .map(hex_string_to_bytes32)
        .collect::<Result<Vec<Bytes32>, CliError>>()?;

    // Updates
    loop {
        match connect_websocket(testnet11, Arc::clone(&db), launcher_ids.clone()).await {
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

#[debug_handler]
async fn get_neighbors(
    Query(params): Query<XchandlesNeighborsQuery>,
    State(state): State<AppState>,
) -> Result<Json<XchandlesNeighborsResponse>, CliError> {
    let handle_hash = hex_string_to_bytes32(&params.handle_hash)?;
    let launcher_id = hex_string_to_bytes32(&params.launcher_id)?;

    let mut allocator = Allocator::new();

    let (left, right) = {
        let db = state.db.lock().await;

        db.get_xchandles_neighbors::<XchandlesSlotValue>(&mut allocator, launcher_id, handle_hash)
            .await
    }?;

    let response = XchandlesNeighborsResponse {
        handle_hash: hex::encode(handle_hash.to_bytes()),

        left_handle_hash: hex::encode(left.info.value.handle_hash.to_bytes()),
        right_handle_hash: hex::encode(right.info.value.handle_hash.to_bytes()),

        left_left_handle_hash: hex::encode(left.info.value.neighbors.left_value.to_bytes()),
        left_expiration: left.info.value.expiration,
        left_owner_launcher_id: hex::encode(left.info.value.owner_launcher_id.to_bytes()),
        left_resolved_data: hex::encode(left.info.value.resolved_data),

        right_right_handle_hash: hex::encode(right.info.value.neighbors.right_value.to_bytes()),
        right_expiration: right.info.value.expiration,
        right_owner_launcher_id: hex::encode(right.info.value.owner_launcher_id.to_bytes()),
        right_resolved_data: hex::encode(right.info.value.resolved_data),

        left_parent_parent_info: hex::encode(left.proof.parent_parent_coin_info.to_bytes()),
        left_parent_inner_puzzle_hash: hex::encode(left.proof.parent_inner_puzzle_hash.to_bytes()),
        left_parent_amount: left.proof.parent_amount,
        right_parent_parent_info: hex::encode(right.proof.parent_parent_coin_info.to_bytes()),
        right_parent_inner_puzzle_hash: hex::encode(
            right.proof.parent_inner_puzzle_hash.to_bytes(),
        ),
        right_parent_amount: right.proof.parent_amount,
    };

    Ok(Json(response))
}

async fn connect_websocket(
    testnet11: bool,
    db: Arc<futures::lock::Mutex<Db>>,
    launcher_ids: Vec<Bytes32>,
) -> Result<(), CliError> {
    println!("Syncing XCHanldes registries (initial)...");
    let client = get_coinset_client(testnet11);

    let mut registries = Vec::<XchandlesRegistry>::new();
    for launcher_id in launcher_ids {
        let registry = {
            let mut db = db.lock().await;
            let mut ctx = SpendContext::new();

            sync_xchandles(&client, &mut db, &mut ctx, launcher_id).await?
        };

        registries.push(registry);
    }

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
                            .get_coin_records_by_names(
                                registries.iter().map(|r| r.coin.coin_id()).collect(),
                                None,
                                None,
                                Some(true),
                            )
                            .await?;

                        let coin_recorss = coin_resp.coin_records.ok_or(CliError::Custom(
                            "Weird - coin records not found after peak update.".to_string(),
                        ))?;
                        for (i, coin_record) in coin_recorss.iter().enumerate() {
                            if coin_record.spent {
                                print!(
                                    "Latest registry #{} coin was spent at height {}... ",
                                    i, coin_record.spent_block_index
                                );

                                let registry = {
                                    let mut ctx = SpendContext::new();
                                    let mut db = db.lock().await;

                                    sync_xchandles(
                                        &client,
                                        &mut db,
                                        &mut ctx,
                                        registries[i].info.constants.launcher_id,
                                    )
                                    .await?
                                };
                                registries[i] = registry;
                                println!("synced :)")
                            }
                        }

                        if last_clear_time.elapsed().unwrap().as_secs() > 60 * 30 {
                            // 30 minutes in seconds
                            if let Some(current_blockchain_state) =
                                client.get_blockchain_state().await?.blockchain_state
                            {
                                print!("Clearing cache (every 30m)... ");
                                let cutoff = current_blockchain_state.peak.height - 128;
                                {
                                    let db = db.lock().await;
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

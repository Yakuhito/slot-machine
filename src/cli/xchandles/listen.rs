use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::debug_handler;
use axum::extract::{Query, State};
use axum::http::Method;
use axum::{http::StatusCode, routing::get, Json, Router};
use chia_protocol::Bytes32;
use chia_wallet_sdk::coinset::ChiaRpcClient;
use chia_wallet_sdk::driver::{SpendContext, XchandlesRegistry};
use chia_wallet_sdk::types::puzzles::{XchandlesHandleSlotValue, XchandlesSlotNonce};
use clvm_utils::ToTreeHash;
use clvmr::Allocator;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tower_http::cors::{Any, CorsLayer};

use super::listener::{
    listener_router, DbHandleSlotStore, DbRegistrationStore, DbSingletonStore, FreshnessState,
    HandleSlotStore, ListenerApiState, RegistrationStore, SingletonIndexer, SingletonStore,
};
use crate::{
    get_coinset_client, hex_string_to_bytes32, sync_xchandles, CliError, CoinsetWebSocketMessage,
    Db,
};

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
    pub left_counter: u64,
    pub left_owner_launcher_id: String,
    pub left_resolved_launcher_id: String,

    pub right_right_handle_hash: String,
    pub right_expiration: u64,
    pub right_counter: u64,
    pub right_owner_launcher_id: String,
    pub right_resolved_launcher_id: String,

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

fn bind_addr() -> SocketAddr {
    std::env::var("BIND_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 8080)))
}

pub async fn xchandles_listen(launcher_ids: String, testnet11: bool) -> Result<(), CliError> {
    let db = Db::new(false).await?;
    let db = Arc::new(futures::lock::Mutex::new(db));

    let launcher_ids = launcher_ids
        .split(',')
        .map(hex_string_to_bytes32)
        .collect::<Result<Vec<Bytes32>, CliError>>()?;

    let singleton_store: Arc<dyn SingletonStore> = DbSingletonStore::new(Arc::clone(&db));
    let handle_slots: Arc<dyn HandleSlotStore> = DbHandleSlotStore::new(Arc::clone(&db));
    let registrations: Arc<dyn RegistrationStore> = DbRegistrationStore::new(Arc::clone(&db));
    let freshness = Arc::new(RwLock::new(FreshnessState::fresh_at(
        0,
        FreshnessState::now_unix(),
    )));
    let indexer = Arc::new(SingletonIndexer::new(
        Arc::clone(&singleton_store),
        Arc::clone(&handle_slots),
        Arc::clone(&registrations),
        Arc::clone(&freshness),
    ));

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let api_state = ListenerApiState {
        store: Arc::clone(&singleton_store),
        handle_slots: Arc::clone(&handle_slots),
        registrations: Arc::clone(&registrations),
        freshness: Arc::clone(&freshness),
        registry_launcher_ids: launcher_ids.clone(),
        now_unix_override: None,
    };
    let neighbors_state = AppState {
        db: Arc::clone(&db),
    };

    tokio::spawn(async move {
        if let Err(e) = start_api_server(api_state, neighbors_state).await {
            eprintln!("API server error: {}", e);
        }
    });

    loop {
        match connect_websocket(
            testnet11,
            Arc::clone(&db),
            launcher_ids.clone(),
            Arc::clone(&indexer),
        )
        .await
        {
            Ok(_resp) => (),
            Err(e) => {
                println!("WebSocket error: {}", e);
                println!("Reconnecting in 5 seconds...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn start_api_server(
    listener_state: ListenerApiState,
    neighbors_state: AppState,
) -> Result<(), CliError> {
    let neighbors = Router::new()
        .route("/", get(health_check))
        .route("/neighbors", get(get_neighbors))
        .with_state(neighbors_state);

    let app = listener_router(listener_state)
        .merge(neighbors)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
                .allow_headers(Any),
        );

    let addr = bind_addr();
    println!("API server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
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

        db.get_xchandles_neighbors::<XchandlesHandleSlotValue>(
            &mut allocator,
            launcher_id,
            handle_hash,
        )
        .await
    }?;

    let response = XchandlesNeighborsResponse {
        handle_hash: hex::encode(handle_hash.to_bytes()),

        left_handle_hash: hex::encode(left.info.value.handle_hash.to_bytes()),
        right_handle_hash: hex::encode(right.info.value.handle_hash.to_bytes()),

        left_left_handle_hash: hex::encode(left.info.value.neighbors.left_value.to_bytes()),
        left_expiration: left.info.value.expiration,
        left_counter: left.info.value.counter,
        left_owner_launcher_id: hex::encode(left.info.value.owner_launcher_id.to_bytes()),
        left_resolved_launcher_id: hex::encode(left.info.value.resolved_launcher_id.to_bytes()),

        right_right_handle_hash: hex::encode(right.info.value.neighbors.right_value.to_bytes()),
        right_expiration: right.info.value.expiration,
        right_counter: right.info.value.counter,
        right_owner_launcher_id: hex::encode(right.info.value.owner_launcher_id.to_bytes()),
        right_resolved_launcher_id: hex::encode(right.info.value.resolved_launcher_id.to_bytes()),

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
    indexer: Arc<SingletonIndexer>,
) -> Result<(), CliError> {
    println!("Syncing XCHandles registries (initial)...");
    let client = get_coinset_client(testnet11);

    let mut registries = Vec::<XchandlesRegistry>::new();
    for launcher_id in &launcher_ids {
        let registry = {
            let mut db = db.lock().await;
            let mut ctx = SpendContext::new();

            sync_xchandles(&client, &mut db, &mut ctx, *launcher_id).await?
        };

        registries.push(registry);
    }

    // Cold start: project currently indexed Handle slots so proofs work for already-synced
    // registries. Confirmation height is unknown for historical slots (0) — fishy for
    // exact height, but the slot value and parent coin remain verifiable.
    {
        let mut allocator = Allocator::new();
        let db_guard = db.lock().await;
        for launcher_id in &launcher_ids {
            if let Ok(rows) = db_guard.list_xchandles_indexed_slots(*launcher_id).await {
                for (handle_hash, value_hash) in rows {
                    let Ok(Some(slot)) = db_guard
                        .get_slot::<XchandlesHandleSlotValue>(
                            &mut allocator,
                            *launcher_id,
                            XchandlesSlotNonce::HANDLE.to_u64(),
                            value_hash,
                            0,
                        )
                        .await
                    else {
                        continue;
                    };
                    // Skip sentinel end markers (no real Owner/Resolved).
                    if slot.info.value.owner_launcher_id == Bytes32::default()
                        && slot.info.value.resolved_launcher_id == Bytes32::default()
                    {
                        continue;
                    }
                    let _ = handle_hash;
                    indexer
                        .project_handle_slot(
                            *launcher_id,
                            slot.info.value,
                            slot.coin.parent_coin_info,
                            0,
                        )
                        .await;
                }
            }
        }
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
            Ok(Message::Text(text)) => match serde_json::from_str::<CoinsetWebSocketMessage>(&text)
            {
                Ok(msg) => {
                    if msg.message_type() == "peak" {
                        let now = SystemTime::now();
                        let now_unix = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
                        println!("[{}] Received new peak", now_unix);

                        let upstream_peak = client
                            .get_blockchain_state()
                            .await?
                            .blockchain_state
                            .as_ref()
                            .map(|s| s.peak.height)
                            .unwrap_or(0);

                        let coin_resp = client
                            .get_coin_records_by_names(
                                registries.iter().map(|r| r.coin.coin_id()).collect(),
                                None,
                                None,
                                Some(true),
                                None,
                            )
                            .await?;

                        let coin_records = coin_resp.coin_records.ok_or(CliError::Custom(
                            "Weird - coin records not found after peak update.".to_string(),
                        ))?;
                        for (i, coin_record) in coin_records.iter().enumerate() {
                            if coin_record.spent {
                                print!(
                                    "Latest registry #{} coin was spent at height {}... ",
                                    i, coin_record.spent_block_index
                                );

                                let spent_height = coin_record.spent_block_index;
                                let header_hash = client
                                    .get_block_record_by_height(spent_height)
                                    .await?
                                    .block_record
                                    .map(|r| r.header_hash);

                                let block_spends = if let Some(hh) = header_hash {
                                    client
                                        .get_block_spends(hh)
                                        .await?
                                        .block_spends
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                };

                                let (registry, logs) = {
                                    let mut ctx = SpendContext::new();
                                    let mut db = db.lock().await;

                                    let parent_spend = client
                                        .get_puzzle_and_solution(
                                            coin_record.coin.coin_id(),
                                            Some(spent_height),
                                        )
                                        .await?
                                        .coin_solution
                                        .ok_or(CliError::CoinNotSpent(
                                            coin_record.coin.coin_id(),
                                        ))?;

                                    let parsed = XchandlesRegistry::from_spend(
                                        &mut ctx,
                                        &parent_spend,
                                        registries[i].info.constants,
                                        chia_bls::Signature::default(),
                                    )?
                                    .ok_or(CliError::Custom(
                                        "Could not parse registry spend".into(),
                                    ))?;
                                    let logs = parsed.pending_spend.logs.clone();

                                    let registry = sync_xchandles(
                                        &client,
                                        &mut db,
                                        &mut ctx,
                                        registries[i].info.constants.launcher_id,
                                    )
                                    .await?;
                                    (registry, logs)
                                };
                                registries[i] = registry;

                                let registry_launcher_id =
                                    registries[i].info.constants.launcher_id;

                                // Build parent_coin_id map for created slots from the synced DB.
                                let parent_by_value_hash = {
                                    let mut allocator = Allocator::new();
                                    let db_guard = db.lock().await;
                                    let mut map = std::collections::HashMap::new();
                                    let mut created = Vec::new();
                                    for log in &logs {
                                        log.extend_created_handle_slots(&mut created);
                                    }
                                    for value in &created {
                                        let value_hash: Bytes32 = value.tree_hash().into();
                                        if let Ok(Some(slot)) = db_guard
                                            .get_slot::<XchandlesHandleSlotValue>(
                                                &mut allocator,
                                                registry_launcher_id,
                                                XchandlesSlotNonce::HANDLE.to_u64(),
                                                value_hash,
                                                0,
                                            )
                                            .await
                                        {
                                            map.insert(value_hash, slot.coin.parent_coin_info);
                                        }
                                    }
                                    map
                                };

                                let mut allocator = Allocator::new();
                                if let Err(e) = indexer
                                    .on_registry_transition(
                                        &mut allocator,
                                        spent_height,
                                        &block_spends,
                                        &logs,
                                    )
                                    .await
                                {
                                    eprintln!("singleton discovery error: {e}");
                                }
                                indexer
                                    .project_handle_slots_from_logs(
                                        registry_launcher_id,
                                        spent_height,
                                        &logs,
                                        |value_hash| parent_by_value_hash.get(&value_hash).copied(),
                                    )
                                    .await;
                                indexer
                                    .project_registrations_from_logs(
                                        registry_launcher_id,
                                        spent_height,
                                        &logs,
                                    )
                                    .await;
                                if let Err(e) = indexer
                                    .on_block(&mut allocator, spent_height, &block_spends)
                                    .await
                                {
                                    eprintln!("singleton follow error: {e}");
                                }

                                println!("synced :)")
                            }
                        }

                        // Follow singleton spends in the new tip block as well.
                        if let Some(state) = client.get_blockchain_state().await?.blockchain_state {
                            let tip = state.peak.height;
                            if let Some(rec) = client
                                .get_block_record_by_height(tip)
                                .await?
                                .block_record
                            {
                                let tip_spends = client
                                    .get_block_spends(rec.header_hash)
                                    .await?
                                    .block_spends
                                    .unwrap_or_default();
                                let mut allocator = Allocator::new();
                                let _ = indexer.on_block(&mut allocator, tip, &tip_spends).await;
                            }
                            indexer.note_peak(tip, upstream_peak.max(tip), now_unix).await;
                        }

                        if last_clear_time.elapsed().unwrap().as_secs() > 60 * 30 {
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

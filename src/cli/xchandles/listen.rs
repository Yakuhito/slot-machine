use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::debug_handler;
use axum::extract::{Query, State};
use axum::http::Method;
use axum::{http::StatusCode, routing::get, Json, Router};
use chia_protocol::{Bytes32, CoinSpend};
use chia_wallet_sdk::coinset::{ChiaRpcClient, CoinsetClient};
use chia_wallet_sdk::driver::{SpendContext, XchandlesActionLog, XchandlesRegistry};
use chia_wallet_sdk::types::puzzles::{XchandlesHandleSlotValue, XchandlesSlotNonce};
use clvm_utils::ToTreeHash;
use clvmr::Allocator;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tower_http::cors::{Any, CorsLayer};

use super::listener::{
    listener_router, DbHandleSlotStore, DbPendingUpdateStore, DbRegistrationStore,
    DbSingletonStore, FollowRecordStatus, FreshnessState, HandleSlotStore, ListenerApiState,
    PendingUpdateStore, RegistrationStore, RegistryPricing, SingletonIndexer, SingletonStore,
};
use crate::{
    get_coinset_client, hex_string_to_bytes32, sync_xchandles,
    sync_xchandles_detailed_from_launcher, CliError, CoinsetWebSocketMessage, Db,
    XchandlesSpentTransition, BASE_PRICE_AT_FACTOR_ONE, PRICE_SCHEDULE, REGISTRATION_PERIOD,
};
use chia_wallet_sdk::driver::XchandlesExpirePricingPuzzle;
use chia_wallet_sdk::types::{puzzles::XchandlesFactorPricingPuzzleArgs, Mod};

fn pricing_from_registry_state(
    pricing_puzzle_hash: Bytes32,
    expired_handle_pricing_puzzle_hash: Bytes32,
) -> Option<RegistryPricing> {
    let candidates: Vec<u64> = PRICE_SCHEDULE
        .iter()
        .map(|(_, _, price)| *price)
        .chain(std::iter::once(BASE_PRICE_AT_FACTOR_ONE))
        .collect();
    for base_price in candidates {
        let factor = XchandlesFactorPricingPuzzleArgs {
            base_price,
            registration_period: REGISTRATION_PERIOD,
        }
        .curry_tree_hash();
        let expired =
            XchandlesExpirePricingPuzzle::curry_tree_hash(base_price, REGISTRATION_PERIOD);
        if factor == pricing_puzzle_hash.into()
            && expired == expired_handle_pricing_puzzle_hash.into()
        {
            return Some(RegistryPricing {
                base_price,
                registration_period: REGISTRATION_PERIOD,
            });
        }
    }
    None
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
    let pending_updates: Arc<dyn PendingUpdateStore> = DbPendingUpdateStore::new(Arc::clone(&db));
    let freshness = Arc::new(RwLock::new(FreshnessState::fresh_at(
        0,
        FreshnessState::now_unix(),
    )));
    let indexer = Arc::new(SingletonIndexer::new(
        Arc::clone(&singleton_store),
        Arc::clone(&handle_slots),
        Arc::clone(&registrations),
        Arc::clone(&pending_updates),
        Arc::clone(&freshness),
    ));

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let mut initial_pricing = std::collections::HashMap::new();
    for id in &launcher_ids {
        initial_pricing.insert(*id, RegistryPricing::default());
    }
    let registry_pricing = Arc::new(RwLock::new(initial_pricing));

    let api_state = ListenerApiState {
        store: Arc::clone(&singleton_store),
        handle_slots: Arc::clone(&handle_slots),
        registrations: Arc::clone(&registrations),
        pending_updates: Arc::clone(&pending_updates),
        freshness: Arc::clone(&freshness),
        registry_pricing: Arc::clone(&registry_pricing),
        registry_launcher_ids: launcher_ids.clone(),
        now_unix_override: None,
    };
    let neighbors_state = AppState {
        db: Arc::clone(&db),
    };

    // Mark the index as resyncing before the HTTP server binds so persisted slots
    // are not served as fresh until the first successful websocket peak.
    indexer.begin_resync().await;

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
            Arc::clone(&registry_pricing),
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

    let app = listener_router(listener_state).merge(neighbors).layer(
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

async fn block_spends_at_height(
    client: &CoinsetClient,
    height: u32,
) -> Result<Vec<CoinSpend>, CliError> {
    let header_hash = client
        .get_block_record_by_height(height)
        .await?
        .block_record
        .map(|r| r.header_hash);
    if let Some(hh) = header_hash {
        Ok(client
            .get_block_spends(hh)
            .await?
            .block_spends
            .unwrap_or_default())
    } else {
        Ok(Vec::new())
    }
}

async fn parent_coin_by_created_slot_hash(
    db: &Arc<futures::lock::Mutex<Db>>,
    launcher_id: Bytes32,
    logs: &[XchandlesActionLog],
) -> std::collections::HashMap<Bytes32, Bytes32> {
    let mut allocator = Allocator::new();
    let db_guard = db.lock().await;
    let mut map = std::collections::HashMap::new();
    let mut created = Vec::new();
    for log in logs {
        log.extend_created_handle_slots(&mut created);
    }
    for value in &created {
        let value_hash: Bytes32 = value.tree_hash().into();
        if let Ok(Some(slot)) = db_guard
            .get_slot::<XchandlesHandleSlotValue>(
                &mut allocator,
                launcher_id,
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
}

/// Same indexer path as a live registry spend: discover NFTs in the block,
/// project slots/registrations/pending, then follow any spent current coins.
async fn index_registry_transition(
    db: &Arc<futures::lock::Mutex<Db>>,
    indexer: &SingletonIndexer,
    launcher_id: Bytes32,
    height: u32,
    logs: &[XchandlesActionLog],
    block_spends: &[CoinSpend],
) {
    let parent_by_value_hash = parent_coin_by_created_slot_hash(db, launcher_id, logs).await;
    let mut allocator = Allocator::new();
    if let Err(e) = indexer
        .on_registry_transition(&mut allocator, height, block_spends, logs)
        .await
    {
        eprintln!("singleton discovery error: {e}");
    }
    indexer
        .project_handle_slots_from_logs(launcher_id, height, logs, |value_hash| {
            parent_by_value_hash.get(&value_hash).copied()
        })
        .await;
    indexer
        .project_registrations_from_logs(launcher_id, height, logs)
        .await;
    indexer
        .project_pending_updates_from_logs(launcher_id, height, logs)
        .await;
    if let Err(e) = indexer.on_block(&mut allocator, height, block_spends).await {
        eprintln!("singleton follow error: {e}");
    }
}

/// Walk every followed NFT already in the store (including ones persisted
/// before this restart) until a full pass finds no spent current coins.
async fn catch_up_followed_nfts(
    client: &CoinsetClient,
    indexer: &SingletonIndexer,
) -> Result<(), CliError> {
    loop {
        let ids = indexer.store.all_launcher_ids().await;
        let mut followed_a_spend = 0usize;
        for launcher_id in ids {
            let Some(rec) = indexer.store.get(launcher_id).await else {
                continue;
            };
            if rec.status != FollowRecordStatus::Active {
                continue;
            }
            let Some(current) = rec.current else {
                continue;
            };
            if current.melted {
                continue;
            }
            let Some(coin_record) = client
                .get_coin_record_by_name(current.coin_id)
                .await?
                .coin_record
            else {
                continue;
            };
            if !coin_record.spent {
                continue;
            }
            let Some(spend) = client
                .get_puzzle_and_solution(current.coin_id, Some(coin_record.spent_block_index))
                .await?
                .coin_solution
            else {
                return Err(CliError::CoinNotSpent(current.coin_id));
            };
            let mut allocator = Allocator::new();
            if let Err(e) = indexer
                .on_block(&mut allocator, coin_record.spent_block_index, &[spend])
                .await
            {
                eprintln!("singleton follow error: {e}");
            }
            followed_a_spend += 1;
        }
        if followed_a_spend == 0 {
            eprintln!("[xchandles-listen] all followed NFT coins are unspent");
            return Ok(());
        }
        eprintln!(
            "[xchandles-listen] followed {followed_a_spend} subsequent NFT spend(s); rechecking unspent coins"
        );
    }
}

async fn connect_websocket(
    testnet11: bool,
    db: Arc<futures::lock::Mutex<Db>>,
    launcher_ids: Vec<Bytes32>,
    indexer: Arc<SingletonIndexer>,
    registry_pricing: Arc<RwLock<std::collections::HashMap<Bytes32, RegistryPricing>>>,
) -> Result<(), CliError> {
    println!("Syncing XCHandles registries (initial)...");
    let client = get_coinset_client(testnet11);

    let mut registries = Vec::<XchandlesRegistry>::new();
    let mut spent_by_launcher: Vec<(Bytes32, Vec<XchandlesSpentTransition>)> = Vec::new();
    for launcher_id in &launcher_ids {
        eprintln!(
            "[xchandles-listen] initial sync {}",
            hex::encode(launcher_id)
        );
        let synced = {
            let mut db = db.lock().await;
            let mut ctx = SpendContext::new();

            sync_xchandles_detailed_from_launcher(&client, &mut db, &mut ctx, *launcher_id).await?
        };
        eprintln!(
            "[xchandles-listen] initial sync {} done, tip coin {}, {} spent transition(s)",
            hex::encode(launcher_id),
            hex::encode(synced.registry.coin.coin_id()),
            synced.spent_transitions.len()
        );
        spent_by_launcher.push((*launcher_id, synced.spent_transitions));
        registries.push(synced.registry);
    }

    // Seed confirmed pricing from the currently synced registry states.
    {
        let mut pricing = registry_pricing.write().await;
        for registry in &registries {
            let launcher_id = registry.info.constants.launcher_id;
            if let Some(p) = pricing_from_registry_state(
                registry.info.state.pricing_puzzle_hash,
                registry.info.state.expired_handle_pricing_puzzle_hash,
            ) {
                pricing.insert(launcher_id, p);
            }
        }
    }

    // Replay spent registry transitions with the same discover+project+follow
    // path as live peaks, so cold start can pick up NFTs created in those blocks.
    for (launcher_id, transitions) in &spent_by_launcher {
        for transition in transitions {
            let block_spends = block_spends_at_height(&client, transition.height).await?;
            index_registry_transition(
                &db,
                indexer.as_ref(),
                *launcher_id,
                transition.height,
                &transition.logs,
                &block_spends,
            )
            .await;
        }
    }

    // Fallback for indexed slots that were never in a replayed action log
    // (upgrade from a sync-only DB). Load slots first, then project —
    // DbHandleSlotStore uses the same `db` mutex, so projecting while this
    // lock is held deadlocks.
    let mut persisted_slots = Vec::new();
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
                    if slot.info.value.owner_launcher_id == Bytes32::default()
                        && slot.info.value.resolved_launcher_id == Bytes32::default()
                    {
                        continue;
                    }
                    persisted_slots.push((
                        *launcher_id,
                        handle_hash,
                        slot.info.value,
                        slot.coin.parent_coin_info,
                    ));
                }
            }
        }
    }
    let mut fallback = 0usize;
    for (launcher_id, handle_hash, value, parent_coin_info) in persisted_slots {
        let already = indexer
            .handle_slots
            .get(launcher_id, handle_hash)
            .await
            .and_then(|r| r.current);
        if already.is_some() {
            continue;
        }
        fallback += 1;
        indexer
            .project_handle_slot(launcher_id, value, parent_coin_info, 0)
            .await;
    }
    if fallback > 0 {
        eprintln!("[xchandles-listen] projected {fallback} persisted handle slot(s) without logs");
    }

    eprintln!(
        "[xchandles-listen] catching up {} followed NFT singleton(s)",
        indexer.store.all_launcher_ids().await.len()
    );
    catch_up_followed_nfts(&client, indexer.as_ref()).await?;

    let ws_url = format!("{}/ws", client.base_url().replace("https://", "wss://"));
    println!("Connecting to WebSocket at {}", ws_url);

    let (ws_stream, _) = connect_async(ws_url)
        .await
        .map_err(|e| CliError::Custom(format!("Failed to connect: {}", e)))?;

    println!("WebSocket connected");

    let (mut _write, mut read) = ws_stream.split();
    let mut last_clear_time = SystemTime::now();
    // In-memory window of recently processed peak (height, header_hash) pairs.
    // Used only to detect reorgs; not persisted.
    let mut recent_peaks: VecDeque<(u32, Bytes32)> = VecDeque::new();

    while let Some(message) = read.next().await {
        match message {
            Ok(Message::Text(text)) => match serde_json::from_str::<CoinsetWebSocketMessage>(&text)
            {
                Ok(msg) => {
                    if msg.message_type() == "peak" {
                        let now = SystemTime::now();
                        let now_unix = now.duration_since(UNIX_EPOCH).unwrap().as_secs();
                        println!("[{}] Received new peak", now_unix);

                        let blockchain_state =
                            client.get_blockchain_state().await?.blockchain_state;
                        let upstream_peak = blockchain_state
                            .as_ref()
                            .map(|s| s.peak.height)
                            .unwrap_or(0);
                        let tip_height = blockchain_state.as_ref().map(|s| s.peak.height);
                        let mut confirmed_timestamp = blockchain_state
                            .as_ref()
                            .and_then(|s| s.peak.timestamp)
                            .unwrap_or(0);
                        let mut tip_rec = None;

                        if let Some(tip) = tip_height {
                            if let Some(rec) =
                                client.get_block_record_by_height(tip).await?.block_record
                            {
                                if let Some(ts) = rec.timestamp {
                                    confirmed_timestamp = ts;
                                } else if confirmed_timestamp == 0 {
                                    // Walk back like get_last_onchain_timestamp when tip is non-tx.
                                    let mut height = tip.saturating_sub(1);
                                    while height > 0 && confirmed_timestamp == 0 {
                                        if let Some(br) = client
                                            .get_block_record_by_height(height)
                                            .await?
                                            .block_record
                                        {
                                            if let Some(ts) = br.timestamp {
                                                confirmed_timestamp = ts;
                                                break;
                                            }
                                        }
                                        height = height.saturating_sub(1);
                                    }
                                }

                                let needs_hash_at = match recent_peaks.back() {
                                    Some(&(last_height, last_hash)) => {
                                        tip != last_height.saturating_add(1)
                                            || rec.prev_hash != last_hash
                                    }
                                    None => false,
                                };
                                let mut hash_at_cache = std::collections::HashMap::new();
                                if needs_hash_at {
                                    for &(h, _) in &recent_peaks {
                                        if hash_at_cache.contains_key(&h) {
                                            continue;
                                        }
                                        if let Some(br) =
                                            client.get_block_record_by_height(h).await?.block_record
                                        {
                                            hash_at_cache.insert(h, br.header_hash);
                                        }
                                    }
                                }
                                if let Some(from_height) = reorg_rollback_from(
                                    recent_peaks.make_contiguous(),
                                    tip,
                                    rec.prev_hash,
                                    |h| hash_at_cache.get(&h).copied(),
                                ) {
                                    eprintln!(
                                        "chain reorg: rolling back from height {from_height}"
                                    );
                                    indexer.rollback(from_height).await;
                                    recent_peaks.retain(|(h, _)| *h < from_height);
                                    for (i, launcher_id) in launcher_ids.iter().enumerate() {
                                        let registry = {
                                            let mut ctx = SpendContext::new();
                                            let mut db = db.lock().await;
                                            sync_xchandles(&client, &mut db, &mut ctx, *launcher_id)
                                                .await?
                                        };
                                        if let Some(p) = pricing_from_registry_state(
                                            registry.info.state.pricing_puzzle_hash,
                                            registry.info.state.expired_handle_pricing_puzzle_hash,
                                        ) {
                                            registry_pricing.write().await.insert(*launcher_id, p);
                                        }
                                        registries[i] = registry;
                                    }
                                }

                                tip_rec = Some(rec);
                            }
                        }

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
                                let block_spends =
                                    block_spends_at_height(&client, spent_height).await?;

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
                                    .ok_or(
                                        CliError::Custom("Could not parse registry spend".into()),
                                    )?;
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

                                let registry_launcher_id = registries[i].info.constants.launcher_id;
                                if let Some(p) = pricing_from_registry_state(
                                    registries[i].info.state.pricing_puzzle_hash,
                                    registries[i].info.state.expired_handle_pricing_puzzle_hash,
                                ) {
                                    registry_pricing
                                        .write()
                                        .await
                                        .insert(registry_launcher_id, p);
                                }

                                index_registry_transition(
                                    &db,
                                    indexer.as_ref(),
                                    registry_launcher_id,
                                    spent_height,
                                    &logs,
                                    &block_spends,
                                )
                                .await;

                                println!("synced :)")
                            }
                        }

                        // Follow singleton spends in the new tip block as well.
                        if let (Some(tip), Some(rec)) = (tip_height, tip_rec) {
                            let tip_spends = client
                                .get_block_spends(rec.header_hash)
                                .await?
                                .block_spends
                                .unwrap_or_default();
                            let mut allocator = Allocator::new();
                            let _ = indexer.on_block(&mut allocator, tip, &tip_spends).await;
                            recent_peaks.retain(|(h, _)| *h < tip);
                            recent_peaks.push_back((tip, rec.header_hash));
                            while recent_peaks.len() > 32 {
                                recent_peaks.pop_front();
                            }
                        }
                        if let Some(tip) = tip_height {
                            indexer
                                .note_peak(
                                    tip,
                                    upstream_peak.max(tip),
                                    now_unix,
                                    confirmed_timestamp,
                                )
                                .await;
                        }

                        if last_clear_time.elapsed().unwrap().as_secs() > 60 * 30 {
                            if let Some(tip) = tip_height {
                                print!("Clearing cache (every 30m)... ");
                                let cutoff = tip.saturating_sub(128);
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

/// First height that is no longer canonical, for `indexer.rollback`.
///
/// - Empty window or linear extend (`new_height == last+1` and `new_prev_hash == last.hash`): `None`.
/// - Tip rewound (`new_height < last`): `Some(new_height + 1)` — everything above the new tip is orphaned.
/// - First stored height whose on-chain header no longer matches: `Some(that height)`.
/// - `hash_at` returning `None` stops the scan (RPC miss); that is not treated as a reorg.
/// - If every stored hash still matches, this is a skipped-peak gap, not a reorg.
fn reorg_rollback_from(
    stored: &[(u32, Bytes32)],
    new_height: u32,
    new_prev_hash: Bytes32,
    hash_at: impl Fn(u32) -> Option<Bytes32>,
) -> Option<u32> {
    let &(last_height, last_hash) = stored.last()?;

    if new_height < last_height {
        return Some(new_height.saturating_add(1));
    }
    if new_height == last_height.saturating_add(1) && new_prev_hash == last_hash {
        return None;
    }

    for &(height, hash) in stored {
        match hash_at(height) {
            Some(on_chain) if on_chain == hash => continue,
            Some(_) => return Some(height),
            None => break,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Bytes32 {
        Bytes32::new([byte; 32])
    }

    #[test]
    fn reorg_rollback_from_empty_stored() {
        assert_eq!(
            reorg_rollback_from(&[], 10, h(1), |_| panic!("hash_at unused")),
            None
        );
    }

    #[test]
    fn reorg_rollback_from_linear_extend() {
        let stored = [(10, h(1))];
        assert_eq!(
            reorg_rollback_from(&stored, 11, h(1), |_| panic!("hash_at unused")),
            None
        );
    }

    #[test]
    fn reorg_rollback_from_gap_last_still_canonical() {
        let stored = [(10, h(1))];
        let hash_at = |height: u32| {
            assert_eq!(height, 10);
            Some(h(1))
        };
        assert_eq!(reorg_rollback_from(&stored, 12, h(99), hash_at), None);
    }

    #[test]
    fn reorg_rollback_from_last_hash_changed() {
        let stored = [(10, h(1))];
        let hash_at = |height: u32| {
            if height == 10 {
                Some(h(2))
            } else {
                None
            }
        };
        assert_eq!(reorg_rollback_from(&stored, 10, h(0), hash_at), Some(10));
    }

    #[test]
    fn reorg_rollback_from_tip_rewound() {
        let stored = [(10, h(1)), (11, h(2))];
        assert_eq!(
            reorg_rollback_from(&stored, 10, h(0), |_| panic!("hash_at unused")),
            Some(11)
        );
    }

    #[test]
    fn reorg_rollback_from_first_mismatch_in_window() {
        let stored = [(10, h(1)), (11, h(2)), (12, h(3))];
        let hash_at = |height: u32| match height {
            10 => Some(h(1)),
            11 => Some(h(9)),
            12 => Some(h(8)),
            _ => None,
        };
        assert_eq!(reorg_rollback_from(&stored, 13, h(0), hash_at), Some(11));
    }

    #[test]
    fn reorg_rollback_from_unverified_hash_is_not_a_reorg() {
        let stored = [(10, h(1))];
        assert_eq!(reorg_rollback_from(&stored, 12, h(99), |_| None), None);
    }
}

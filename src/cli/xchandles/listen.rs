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
    committed_base_from_pricing_puzzle, effective_base_at, generations_for_network,
    listener_router, DbHandleSlotStore, DbPendingUpdateStore, DbRegistrationStore,
    DbSingletonStore, FollowRecordStatus, FreshnessState, HandleSlotStore, ListenerApiState,
    PendingUpdateStore, RegistrationStore, RegistryPricing, ScheduleGeneration, SingletonIndexer,
    SingletonStore, SlotParentLineage,
};
use crate::{
    get_coinset_client, get_last_onchain_timestamp, hex_string_to_bytes32, sync_xchandles,
    sync_xchandles_detailed, CliError, CoinsetWebSocketMessage, Db, XchandlesSpentTransition,
    REGISTRATION_PERIOD,
};

/// Latest schedule row whose timestamp is `<= now`. Before the first row, launch price is 1.
fn schedule_base_price_at(now: u64, testnet11: bool) -> u64 {
    effective_base_at(&generations_for_network(testnet11), now)
}

fn pricing_at(now: u64, testnet11: bool) -> RegistryPricing {
    RegistryPricing {
        base_price: schedule_base_price_at(now, testnet11),
        registration_period: REGISTRATION_PERIOD,
    }
}

async fn set_schedule_pricing(
    registry_pricing: &RwLock<std::collections::HashMap<Bytes32, RegistryPricing>>,
    committed_base_price: &RwLock<std::collections::HashMap<Bytes32, u64>>,
    launcher_ids: &[Bytes32],
    registries: &[chia_wallet_sdk::driver::XchandlesRegistry],
    schedule: &[ScheduleGeneration],
    now: u64,
    testnet11: bool,
) {
    let pricing = pricing_at(now, testnet11);
    let effective = pricing.base_price;
    {
        let mut map = registry_pricing.write().await;
        for id in launcher_ids {
            map.insert(*id, pricing);
        }
    }
    let mut committed = committed_base_price.write().await;
    for (i, id) in launcher_ids.iter().enumerate() {
        let from_puzzle = registries.get(i).and_then(|registry| {
            committed_base_from_pricing_puzzle(
                registry.info.state.pricing_puzzle_hash,
                registry_period_or_default(registry),
                schedule,
            )
        });
        committed.insert(*id, from_puzzle.unwrap_or(effective));
    }
}

fn registry_period_or_default(registry: &chia_wallet_sdk::driver::XchandlesRegistry) -> u64 {
    let _ = registry;
    REGISTRATION_PERIOD
}

#[derive(Debug, Deserialize)]
struct XchandlesNeighborsQuery {
    launcher_id: String,
    handle_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct XchandlesNeighborsResponse {
    pub left_handle_hash: String,
    pub right_handle_hash: String,

    pub left_left_handle_hash: String,
    pub left_expiration: u64,
    pub left_counter: u64,
    pub left_owner_launcher_id: String,
    pub left_resolved_launcher_id: String,
    #[serde(default)]
    pub left_parent_id: String,
    #[serde(alias = "left_parent_parent_info")]
    pub left_parent_parent_id: String,
    pub left_parent_inner_puzzle_hash: String,

    pub right_right_handle_hash: String,
    pub right_expiration: u64,
    pub right_counter: u64,
    pub right_owner_launcher_id: String,
    pub right_resolved_launcher_id: String,
    #[serde(default)]
    pub right_parent_id: String,
    #[serde(alias = "right_parent_parent_info")]
    pub right_parent_parent_id: String,
    pub right_parent_inner_puzzle_hash: String,
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
    let startup_pricing = pricing_at(FreshnessState::now_unix(), testnet11);
    for id in &launcher_ids {
        initial_pricing.insert(*id, startup_pricing);
    }
    let registry_pricing = Arc::new(RwLock::new(initial_pricing));
    let price_schedule = Arc::new(generations_for_network(testnet11));
    let mut initial_committed = std::collections::HashMap::new();
    for id in &launcher_ids {
        initial_committed.insert(*id, startup_pricing.base_price);
    }
    let committed_base_price = Arc::new(RwLock::new(initial_committed));

    let api_state = ListenerApiState {
        store: Arc::clone(&singleton_store),
        handle_slots: Arc::clone(&handle_slots),
        registrations: Arc::clone(&registrations),
        pending_updates: Arc::clone(&pending_updates),
        freshness: Arc::clone(&freshness),
        registry_pricing: Arc::clone(&registry_pricing),
        price_schedule: Arc::clone(&price_schedule),
        committed_base_price: Arc::clone(&committed_base_price),
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
            Arc::clone(&committed_base_price),
            Arc::clone(&price_schedule),
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
        left_handle_hash: hex::encode(left.info.value.handle_hash.to_bytes()),
        right_handle_hash: hex::encode(right.info.value.handle_hash.to_bytes()),

        left_left_handle_hash: hex::encode(left.info.value.neighbors.left_value.to_bytes()),
        left_expiration: left.info.value.expiration,
        left_counter: left.info.value.counter,
        left_owner_launcher_id: hex::encode(left.info.value.owner_launcher_id.to_bytes()),
        left_resolved_launcher_id: hex::encode(left.info.value.resolved_launcher_id.to_bytes()),
        left_parent_id: hex::encode(left.coin.parent_coin_info.to_bytes()),
        left_parent_parent_id: hex::encode(left.proof.parent_parent_coin_info.to_bytes()),
        left_parent_inner_puzzle_hash: hex::encode(left.proof.parent_inner_puzzle_hash.to_bytes()),

        right_right_handle_hash: hex::encode(right.info.value.neighbors.right_value.to_bytes()),
        right_expiration: right.info.value.expiration,
        right_counter: right.info.value.counter,
        right_owner_launcher_id: hex::encode(right.info.value.owner_launcher_id.to_bytes()),
        right_resolved_launcher_id: hex::encode(right.info.value.resolved_launcher_id.to_bytes()),
        right_parent_id: hex::encode(right.coin.parent_coin_info.to_bytes()),
        right_parent_parent_id: hex::encode(right.proof.parent_parent_coin_info.to_bytes()),
        right_parent_inner_puzzle_hash: hex::encode(
            right.proof.parent_inner_puzzle_hash.to_bytes(),
        ),
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

async fn parent_lineage_by_created_slot_hash(
    db: &Arc<futures::lock::Mutex<Db>>,
    launcher_id: Bytes32,
    logs: &[XchandlesActionLog],
) -> std::collections::HashMap<Bytes32, SlotParentLineage> {
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
            map.insert(
                value_hash,
                SlotParentLineage {
                    parent_coin_id: slot.coin.parent_coin_info,
                    parent_parent_id: slot.proof.parent_parent_coin_info,
                    parent_inner_puzzle_hash: slot.proof.parent_inner_puzzle_hash,
                },
            );
        }
    }
    map
}

#[derive(Debug, Clone)]
struct RegistryIndexedTransition {
    height: u32,
    logs: Vec<XchandlesActionLog>,
    block_spends: Vec<CoinSpend>,
    parent_by_value_hash: std::collections::HashMap<Bytes32, SlotParentLineage>,
}

#[derive(Debug, Clone)]
struct RegistrySyncBatch {
    registry: XchandlesRegistry,
    transitions: Vec<RegistryIndexedTransition>,
}

#[async_trait::async_trait]
trait RegistryChainSource: Send + Sync {
    async fn sync_registry(&self, launcher_id: Bytes32) -> Result<RegistrySyncBatch, CliError>;
}

#[async_trait::async_trait]
trait BlockSpendSource: Send + Sync {
    async fn block_at_height(
        &self,
        height: u32,
    ) -> Result<Option<(Bytes32, Vec<CoinSpend>)>, CliError>;
}

struct CoinsetBlockSource<'a> {
    client: &'a CoinsetClient,
}

#[async_trait::async_trait]
impl BlockSpendSource for CoinsetBlockSource<'_> {
    async fn block_at_height(
        &self,
        height: u32,
    ) -> Result<Option<(Bytes32, Vec<CoinSpend>)>, CliError> {
        let Some(rec) = self
            .client
            .get_block_record_by_height(height)
            .await?
            .block_record
        else {
            return Ok(None);
        };
        let spends = self
            .client
            .get_block_spends(rec.header_hash)
            .await?
            .block_spends
            .unwrap_or_default();
        Ok(Some((rec.header_hash, spends)))
    }
}

/// Follow spent current coins in every canonical block after `after_height` through `tip`.
///
/// Returns `(height, header_hash)` for each height that had a block record, in order.
/// Missing records are skipped so the next peak can retry them.
async fn follow_blocks_after(
    indexer: &SingletonIndexer,
    source: &dyn BlockSpendSource,
    after_height: u32,
    tip: u32,
) -> Result<Vec<(u32, Bytes32)>, CliError> {
    let mut followed = Vec::new();
    let start = after_height.saturating_add(1);
    if start > tip {
        return Ok(followed);
    }
    for height in start..=tip {
        let Some((header_hash, spends)) = source.block_at_height(height).await? else {
            continue;
        };
        let mut allocator = Allocator::new();
        if let Err(e) = indexer.on_block(&mut allocator, height, &spends).await {
            eprintln!("singleton follow error: {e}");
        }
        followed.push((height, header_hash));
    }
    Ok(followed)
}

struct CoinsetRegistryChainSource<'a> {
    client: &'a CoinsetClient,
    db: &'a Arc<futures::lock::Mutex<Db>>,
}

#[async_trait::async_trait]
impl RegistryChainSource for CoinsetRegistryChainSource<'_> {
    async fn sync_registry(&self, launcher_id: Bytes32) -> Result<RegistrySyncBatch, CliError> {
        let synced = {
            let mut ctx = SpendContext::new();
            let mut db = self.db.lock().await;
            sync_xchandles_detailed(self.client, &mut db, &mut ctx, launcher_id).await?
        };

        let mut transitions = Vec::with_capacity(synced.spent_transitions.len());
        for transition in synced.spent_transitions {
            let block_spends = block_spends_at_height(self.client, transition.height).await?;
            let parent_by_value_hash =
                parent_lineage_by_created_slot_hash(self.db, launcher_id, &transition.logs).await;
            transitions.push(RegistryIndexedTransition {
                height: transition.height,
                logs: transition.logs,
                block_spends,
                parent_by_value_hash,
            });
        }

        Ok(RegistrySyncBatch {
            registry: synced.registry,
            transitions,
        })
    }
}

async fn apply_registry_transition(
    indexer: &SingletonIndexer,
    launcher_id: Bytes32,
    height: u32,
    logs: &[XchandlesActionLog],
    block_spends: &[CoinSpend],
    parent_by_value_hash: &std::collections::HashMap<Bytes32, SlotParentLineage>,
) {
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

async fn process_spent_registry_records(
    source: &dyn RegistryChainSource,
    indexer: &SingletonIndexer,
    registries: &mut [XchandlesRegistry],
    coin_records: Vec<chia_wallet_sdk::coinset::CoinRecord>,
) -> Result<(), CliError> {
    let mut records_by_coin_id = std::collections::HashMap::new();
    for record in coin_records {
        let coin_id = record.coin.coin_id();
        if records_by_coin_id.insert(coin_id, record).is_some() {
            return Err(CliError::Custom(format!(
                "duplicate coin record for registry coin {}",
                hex::encode(coin_id)
            )));
        }
    }
    for registry in registries.iter() {
        let coin_id = registry.coin.coin_id();
        if !records_by_coin_id.contains_key(&coin_id) {
            return Err(CliError::Custom(format!(
                "missing coin record for registry coin {}",
                hex::encode(coin_id)
            )));
        }
    }
    if records_by_coin_id.len() != registries.len() {
        return Err(CliError::Custom(
            "coin record response contained an unexpected registry coin".to_string(),
        ));
    }

    for (i, registry) in registries.iter_mut().enumerate() {
        let coin_record = records_by_coin_id
            .remove(&registry.coin.coin_id())
            .expect("registry coin records were validated before processing");
        if !coin_record.spent {
            continue;
        }
        let launcher_id = registry.info.constants.launcher_id;
        println!(
            "Latest registry #{} coin was spent at height {}... ",
            i, coin_record.spent_block_index
        );
        let synced = source.sync_registry(launcher_id).await?;
        for transition in &synced.transitions {
            apply_registry_transition(
                indexer,
                launcher_id,
                transition.height,
                &transition.logs,
                &transition.block_spends,
                &transition.parent_by_value_hash,
            )
            .await;
        }
        *registry = synced.registry;
        println!("synced :)");
    }
    Ok(())
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
    let parent_by_value_hash = parent_lineage_by_created_slot_hash(db, launcher_id, logs).await;
    apply_registry_transition(
        indexer,
        launcher_id,
        height,
        logs,
        block_spends,
        &parent_by_value_hash,
    )
    .await;
}

/// Full-node `get_coin_records_by_names` request cap.
const COINSET_NAMES_BATCH: usize = 500;

/// Walk every followed NFT already in the store (including ones persisted
/// before this restart) until a full pass finds no spent current coins.
async fn catch_up_followed_nfts(
    client: &CoinsetClient,
    indexer: &SingletonIndexer,
) -> Result<(), CliError> {
    loop {
        let ids = indexer.store.all_launcher_ids().await;
        let mut coin_ids = Vec::new();
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
            coin_ids.push(current.coin_id);
        }

        let mut spent = Vec::new();
        for batch in coin_ids.chunks(COINSET_NAMES_BATCH) {
            let Some(records) = client
                .get_coin_records_by_names(batch.to_vec(), None, None, Some(true), None)
                .await?
                .coin_records
            else {
                continue;
            };
            spent.extend(records.into_iter().filter(|record| record.spent));
        }

        if spent.is_empty() {
            eprintln!("[xchandles-listen] all followed NFT coins are unspent");
            return Ok(());
        }

        for coin_record in &spent {
            let coin_id = coin_record.coin.coin_id();
            let Some(spend) = client
                .get_puzzle_and_solution(coin_id, Some(coin_record.spent_block_index))
                .await?
                .coin_solution
            else {
                return Err(CliError::CoinNotSpent(coin_id));
            };
            let mut allocator = Allocator::new();
            if let Err(e) = indexer
                .on_block(&mut allocator, coin_record.spent_block_index, &[spend])
                .await
            {
                eprintln!("singleton follow error: {e}");
            }
        }
        eprintln!(
            "[xchandles-listen] followed {} subsequent NFT spend(s); rechecking unspent coins",
            spent.len()
        );
    }
}

async fn connect_websocket(
    testnet11: bool,
    db: Arc<futures::lock::Mutex<Db>>,
    launcher_ids: Vec<Bytes32>,
    indexer: Arc<SingletonIndexer>,
    registry_pricing: Arc<RwLock<std::collections::HashMap<Bytes32, RegistryPricing>>>,
    committed_base_price: Arc<RwLock<std::collections::HashMap<Bytes32, u64>>>,
    price_schedule: Arc<Vec<ScheduleGeneration>>,
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

            sync_xchandles_detailed(&client, &mut db, &mut ctx, *launcher_id).await?
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

    let onchain_ts = get_last_onchain_timestamp(&client).await?;
    set_schedule_pricing(
        &registry_pricing,
        &committed_base_price,
        &launcher_ids,
        &registries,
        &price_schedule,
        onchain_ts,
        testnet11,
    )
    .await;

    // Replay spent registry transitions (full history on first sync, only new
    // spends when resuming a saved tip) with the same discover+project+follow
    // path as live peaks.
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
    // (upgrade from a sync-only DB). Load slots first, then project -
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
                        SlotParentLineage {
                            parent_coin_id: slot.coin.parent_coin_info,
                            parent_parent_id: slot.proof.parent_parent_coin_info,
                            parent_inner_puzzle_hash: slot.proof.parent_inner_puzzle_hash,
                        },
                    ));
                }
            }
        }
    }
    let mut fallback = 0usize;
    for (launcher_id, handle_hash, value, parent) in persisted_slots {
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
            .project_handle_slot(launcher_id, value, parent, 0)
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

    // Seed the live follow cursor at the current tip so reconnects walk only
    // new blocks (catch-up above already walked spent coins via coin records).
    // recent_peaks is an in-memory window of (height, header_hash) used only
    // to detect reorgs; it is not persisted.
    let mut last_followed_height = client
        .get_blockchain_state()
        .await?
        .blockchain_state
        .map(|s| s.peak.height)
        .ok_or_else(|| CliError::Custom("no blockchain state after NFT catch-up".to_string()))?;
    let mut recent_peaks: VecDeque<(u32, Bytes32)> = VecDeque::new();
    if let Some(rec) = client
        .get_block_record_by_height(last_followed_height)
        .await?
        .block_record
    {
        recent_peaks.push_back((last_followed_height, rec.header_hash));
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
                                    last_followed_height = recent_peaks
                                        .back()
                                        .map(|(h, _)| *h)
                                        .unwrap_or(from_height.saturating_sub(1));
                                    for (i, launcher_id) in launcher_ids.iter().enumerate() {
                                        let registry = {
                                            let mut ctx = SpendContext::new();
                                            let mut db = db.lock().await;
                                            sync_xchandles(&client, &mut db, &mut ctx, *launcher_id)
                                                .await?
                                        };
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
                        let source = CoinsetRegistryChainSource {
                            client: &client,
                            db: &db,
                        };
                        process_spent_registry_records(
                            &source,
                            indexer.as_ref(),
                            &mut registries,
                            coin_records,
                        )
                        .await?;

                        // Follow singleton spends in every block from the last
                        // followed height through the current tip.
                        if let Some(tip) = tip_height {
                            let source = CoinsetBlockSource { client: &client };
                            match follow_blocks_after(
                                indexer.as_ref(),
                                &source,
                                last_followed_height,
                                tip,
                            )
                            .await
                            {
                                Ok(followed) => {
                                    for (height, header_hash) in followed {
                                        recent_peaks.retain(|(h, _)| *h < height);
                                        recent_peaks.push_back((height, header_hash));
                                        last_followed_height = height;
                                    }
                                    while recent_peaks.len() > 32 {
                                        recent_peaks.pop_front();
                                    }
                                }
                                Err(e) => {
                                    eprintln!("singleton follow error: {e}");
                                }
                            }
                            if let Some(rec) = &tip_rec {
                                if recent_peaks.back().map(|(h, _)| *h) != Some(tip) {
                                    recent_peaks.retain(|(h, _)| *h < tip);
                                    recent_peaks.push_back((tip, rec.header_hash));
                                    while recent_peaks.len() > 32 {
                                        recent_peaks.pop_front();
                                    }
                                }
                            }
                        }
                        if let Some(tip) = tip_height {
                            set_schedule_pricing(
                                &registry_pricing,
                                &committed_base_price,
                                &launcher_ids,
                                &registries,
                                &price_schedule,
                                confirmed_timestamp,
                                testnet11,
                            )
                            .await;
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
/// - Tip rewound (`new_height < last`): `Some(new_height + 1)` - everything above the new tip is orphaned.
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

    use std::collections::HashMap;

    use chia_protocol::Coin;
    use chia_puzzle_types::singleton::SingletonArgs;
    use chia_puzzle_types::{EveProof, Proof};
    use chia_wallet_sdk::coinset::CoinRecord;
    use chia_wallet_sdk::driver::{
        Launcher, SingletonInfo, StandardLayer, XchandlesPrecommitValue,
        XchandlesRegisterActionLog, XchandlesRegistryInfo, XchandlesRegistryState,
    };
    use chia_wallet_sdk::test::{BlsPair, Simulator};
    use chia_wallet_sdk::types::puzzles::{XchandlesHandleSlotValue, XchandlesPricingSolution};
    use chia_wallet_sdk::types::Conditions;

    #[derive(Clone)]
    struct FakeRegistryChainSource {
        syncs: HashMap<Bytes32, RegistrySyncBatch>,
    }

    #[derive(Clone, Default)]
    struct FakeBlockSource {
        blocks: HashMap<u32, (Bytes32, Vec<CoinSpend>)>,
    }

    #[async_trait::async_trait]
    impl BlockSpendSource for FakeBlockSource {
        async fn block_at_height(
            &self,
            height: u32,
        ) -> Result<Option<(Bytes32, Vec<CoinSpend>)>, CliError> {
            Ok(self.blocks.get(&height).cloned())
        }
    }

    #[async_trait::async_trait]
    impl RegistryChainSource for FakeRegistryChainSource {
        async fn sync_registry(&self, launcher_id: Bytes32) -> Result<RegistrySyncBatch, CliError> {
            self.syncs.get(&launcher_id).cloned().ok_or_else(|| {
                CliError::Custom(format!(
                    "unexpected registry sync: {}",
                    hex::encode(launcher_id)
                ))
            })
        }
    }

    fn h(byte: u8) -> Bytes32 {
        Bytes32::new([byte; 32])
    }

    fn fake_registry(launcher_id: Bytes32, coin_tag: u8) -> XchandlesRegistry {
        XchandlesRegistry::new(
            Coin::new(h(coin_tag), h(coin_tag.wrapping_add(1)), 1),
            Proof::Eve(EveProof {
                parent_parent_coin_info: h(coin_tag.wrapping_add(2)),
                parent_amount: 1,
            }),
            XchandlesRegistryInfo::new(
                XchandlesRegistryState::from(h(0x91), 5_000, REGISTRATION_PERIOD),
                chia_wallet_sdk::driver::XchandlesConstants::new(launcher_id, h(0x92), 32, h(0x93)),
            ),
        )
    }

    fn register_log(handle: &str, tag: u8) -> XchandlesActionLog {
        let slot = |name: &str, counter: u64| {
            XchandlesHandleSlotValue::new(
                counter,
                name.tree_hash().into(),
                Bytes32::default(),
                Bytes32::new([0xff; 32]),
                4_102_444_800,
                h(0x11),
                h(0x11),
            )
        };
        let precommit = XchandlesPrecommitValue::new(
            h(0x01),
            (),
            h(0x02),
            XchandlesPricingSolution {
                buy_time: 1_700_000_000,
                current_expiration: 0,
                handle: handle.to_string(),
                num_periods: 1,
            },
            handle.to_string(),
            h(tag),
            h(0x11),
            h(0x11),
        );
        XchandlesActionLog::Register(XchandlesRegisterActionLog {
            spent_left_slot: slot("left", 0),
            spent_right_slot: slot("right", 0),
            created_left_slot: slot("left", 1),
            created_handle_slot: slot(handle, 0),
            created_right_slot: slot("right", 1),
            precommit_value: precommit,
            total_price: u64::from(tag),
            registered_time: REGISTRATION_PERIOD,
            owner_full_puzzle_hash: h(0x31),
            resolved_full_puzzle_hash: None,
            owner_inner_puzzle_hash: h(0x32),
            resolved_inner_puzzle_hash: h(0x32),
        })
    }

    async fn spawn_registration_api(
        registry_launcher_ids: Vec<Bytes32>,
        indexer: &SingletonIndexer,
    ) -> String {
        let state = ListenerApiState {
            store: Arc::clone(&indexer.store),
            handle_slots: Arc::clone(&indexer.handle_slots),
            registrations: Arc::clone(&indexer.registrations),
            pending_updates: Arc::clone(&indexer.pending_updates),
            freshness: Arc::clone(&indexer.freshness),
            registry_pricing: Arc::new(RwLock::new(HashMap::new())),
            price_schedule: Arc::new(Vec::new()),
            committed_base_price: Arc::new(RwLock::new(HashMap::new())),
            registry_launcher_ids,
            now_unix_override: None,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, listener_router(state)).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn skipped_peaks_publish_every_intermediate_registry_transition() {
        let launcher_id = h(0xa0);
        let current = fake_registry(launcher_id, 0x10);
        let latest = fake_registry(launcher_id, 0x20);
        let observed = CoinRecord {
            coin: current.coin,
            coinbase: false,
            confirmed_block_index: 8,
            spent: true,
            spent_block_index: 10,
            timestamp: 1_700_000_000,
        };
        let source = FakeRegistryChainSource {
            syncs: HashMap::from([(
                launcher_id,
                RegistrySyncBatch {
                    registry: latest.clone(),
                    transitions: vec![
                        RegistryIndexedTransition {
                            height: 10,
                            logs: vec![register_log("alice", 0xa1)],
                            block_spends: Vec::new(),
                            parent_by_value_hash: HashMap::new(),
                        },
                        RegistryIndexedTransition {
                            height: 11,
                            logs: vec![register_log("bravo", 0xb1)],
                            block_spends: Vec::new(),
                            parent_by_value_hash: HashMap::new(),
                        },
                    ],
                },
            )]),
        };
        let indexer = SingletonIndexer::new(
            crate::MemorySingletonStore::shared() as Arc<dyn SingletonStore>,
            crate::MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
            crate::MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
            crate::MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
            Arc::new(RwLock::new(FreshnessState::fresh_at(
                11,
                FreshnessState::now_unix(),
            ))),
        );
        let mut registries = vec![current];

        process_spent_registry_records(&source, &indexer, &mut registries, vec![observed])
            .await
            .unwrap();

        let base = spawn_registration_api(vec![launcher_id], &indexer).await;
        let client = reqwest::Client::new();
        for (handle, height) in [("alice", 10), ("bravo", 11)] {
            let response = client
                .get(format!("{base}/registrations/{handle}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["confirmation_height"], height);
        }
        assert_eq!(registries[0].coin, latest.coin);
    }

    #[tokio::test]
    async fn skipped_peaks_follow_singleton_spend_between_last_indexed_and_tip() {
        let mut sim = Simulator::new();
        let mut ctx = SpendContext::new();
        let bls = BlsPair::default();
        let p2 = StandardLayer::new(bls.pk);
        let launcher_coin = sim.new_coin(chia_puzzles::SINGLETON_LAUNCHER_HASH.into(), 1);
        let launcher = Launcher::new(launcher_coin.parent_coin_info, 1);
        let (_, did) = launcher.create_simple_did(&mut ctx, &p2).unwrap();
        sim.spend_coins(ctx.take(), std::slice::from_ref(&bls.sk))
            .unwrap();

        let owner = did.info.launcher_id;
        let inner: Bytes32 = did.info.inner_puzzle_hash().into();
        let full: Bytes32 = SingletonArgs::curry_tree_hash(owner, inner.into()).into();

        let mut ctx = SpendContext::new();
        let current = did.update(&mut ctx, &p2, Conditions::new()).unwrap();
        let initiator = current.coin.parent_coin_info;
        let spends = ctx.take();
        sim.spend_coins(spends, std::slice::from_ref(&bls.sk))
            .unwrap();

        let mut ctx = SpendContext::new();
        let next = current.update(&mut ctx, &p2, Conditions::new()).unwrap();
        let spends = ctx.take();
        let follow_spend = spends
            .iter()
            .find(|s| s.coin.coin_id() == current.coin.coin_id())
            .cloned()
            .unwrap();
        sim.spend_coins(spends, std::slice::from_ref(&bls.sk))
            .unwrap();

        let registry = h(0xa0);
        let handle = "alice";
        let handle_hash: Bytes32 = handle.tree_hash().into();
        let store = crate::MemorySingletonStore::shared();
        let handle_slots = crate::MemoryHandleSlotStore::shared();
        let pending_updates = crate::MemoryPendingUpdateStore::shared();
        let indexer = SingletonIndexer::new(
            store.clone() as Arc<dyn SingletonStore>,
            handle_slots.clone() as Arc<dyn HandleSlotStore>,
            crate::MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
            pending_updates.clone() as Arc<dyn PendingUpdateStore>,
            Arc::new(RwLock::new(FreshnessState::fresh_at(
                12,
                FreshnessState::now_unix(),
            ))),
        );

        store
            .upsert(crate::FollowedSingleton {
                launcher_id: owner,
                expected_full_puzzle_hash: full,
                expected_inner_puzzle_hash: inner,
                discovery_height: 10,
                status: FollowRecordStatus::Active,
                current: Some(crate::StoredSingletonState::from_coin(
                    owner,
                    current.coin,
                    inner,
                    10,
                    false,
                    None,
                    None,
                )),
                history: Vec::new(),
                reference_count: 1,
                dereference_height: None,
            })
            .await;
        handle_slots
            .upsert(crate::HandleSlotRecord {
                registry_launcher_id: registry,
                handle_hash,
                current: Some(crate::StoredHandleSlot {
                    registry_launcher_id: registry,
                    handle_hash,
                    counter: 1,
                    neighbors_left: Bytes32::default(),
                    neighbors_right: Bytes32::new([0xff; 32]),
                    expiration: 4_102_444_800,
                    owner_launcher_id: owner,
                    resolved_launcher_id: owner,
                    parent_coin_id: h(0x31),
                    parent_parent_id: h(0xee),
                    parent_inner_puzzle_hash: h(0xef),
                    confirmation_height: 90,
                }),
                history: Vec::new(),
            })
            .await;
        pending_updates
            .upsert(crate::PendingUpdateRecord {
                registry_launcher_id: registry,
                handle_hash,
                current: Some(crate::StoredPendingUpdate {
                    registry_launcher_id: registry,
                    handle_hash,
                    new_owner_launcher_id: h(0x33),
                    new_resolved_launcher_id: h(0x44),
                    update_confirmation_height: 100,
                    minimum_execution_height: 150,
                    update_initiator_coin_id: initiator,
                }),
                history: Vec::new(),
            })
            .await;

        let base = spawn_registration_api(vec![registry], &indexer).await;
        let client = reqwest::Client::new();
        let before = client
            .get(format!("{base}/handle/{handle}/pending-transfer"))
            .send()
            .await
            .unwrap();
        assert_eq!(before.status(), StatusCode::OK);

        let source = FakeBlockSource {
            blocks: HashMap::from([
                (11, (h(0x11), vec![follow_spend])),
                (12, (h(0x12), Vec::new())),
            ]),
        };
        follow_blocks_after(&indexer, &source, 10, 12)
            .await
            .unwrap();

        let rec = store.get(owner).await.unwrap();
        assert_eq!(
            rec.current.as_ref().unwrap().coin_id,
            next.coin.coin_id(),
            "intermediate singleton spend must be followed before the empty tip"
        );
        let after = client
            .get(format!("{base}/handle/{handle}/pending-transfer"))
            .send()
            .await
            .unwrap();
        assert_eq!(after.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn reordered_coin_records_update_the_registry_with_the_matching_coin_id() {
        let launcher_a = h(0xa0);
        let launcher_b = h(0xb0);
        let current_a = fake_registry(launcher_a, 0x10);
        let current_b = fake_registry(launcher_b, 0x30);
        let latest_a = fake_registry(launcher_a, 0x20);
        let latest_b = fake_registry(launcher_b, 0x40);
        let spent_a = CoinRecord {
            coin: current_a.coin,
            coinbase: false,
            confirmed_block_index: 8,
            spent: true,
            spent_block_index: 10,
            timestamp: 1_700_000_000,
        };
        let unspent_b = CoinRecord {
            coin: current_b.coin,
            coinbase: false,
            confirmed_block_index: 8,
            spent: false,
            spent_block_index: 0,
            timestamp: 1_700_000_000,
        };
        let source = FakeRegistryChainSource {
            syncs: HashMap::from([
                (
                    launcher_a,
                    RegistrySyncBatch {
                        registry: latest_a.clone(),
                        transitions: vec![RegistryIndexedTransition {
                            height: 10,
                            logs: vec![register_log("alice", 0xa1)],
                            block_spends: Vec::new(),
                            parent_by_value_hash: HashMap::new(),
                        }],
                    },
                ),
                (
                    launcher_b,
                    RegistrySyncBatch {
                        registry: latest_b,
                        transitions: vec![RegistryIndexedTransition {
                            height: 10,
                            logs: vec![register_log("bravo", 0xb1)],
                            block_spends: Vec::new(),
                            parent_by_value_hash: HashMap::new(),
                        }],
                    },
                ),
            ]),
        };
        let indexer = SingletonIndexer::new(
            crate::MemorySingletonStore::shared() as Arc<dyn SingletonStore>,
            crate::MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
            crate::MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
            crate::MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
            Arc::new(RwLock::new(FreshnessState::fresh_at(
                10,
                FreshnessState::now_unix(),
            ))),
        );
        let mut registries = vec![current_a, current_b];

        // Coinset does not promise request order; return B before A.
        process_spent_registry_records(
            &source,
            &indexer,
            &mut registries,
            vec![unspent_b, spent_a],
        )
        .await
        .unwrap();

        let base = spawn_registration_api(vec![launcher_a, launcher_b], &indexer).await;
        let alice = reqwest::get(format!(
            "{base}/registrations/alice?launcher_id={launcher_a}"
        ))
        .await
        .unwrap();
        assert_eq!(alice.status(), StatusCode::OK);
        let bravo = reqwest::get(format!(
            "{base}/registrations/bravo?launcher_id={launcher_b}"
        ))
        .await
        .unwrap();
        assert_eq!(bravo.status(), StatusCode::NOT_FOUND);
        assert_eq!(registries[0].coin, latest_a.coin);
    }

    #[tokio::test]
    async fn partial_coin_record_response_is_rejected_before_any_registry_update() {
        let launcher_a = h(0xa0);
        let launcher_b = h(0xb0);
        let current_a = fake_registry(launcher_a, 0x10);
        let current_b = fake_registry(launcher_b, 0x30);
        let latest_a = fake_registry(launcher_a, 0x20);
        let spent_a = CoinRecord {
            coin: current_a.coin,
            coinbase: false,
            confirmed_block_index: 8,
            spent: true,
            spent_block_index: 10,
            timestamp: 1_700_000_000,
        };
        let source = FakeRegistryChainSource {
            syncs: HashMap::from([(
                launcher_a,
                RegistrySyncBatch {
                    registry: latest_a,
                    transitions: vec![RegistryIndexedTransition {
                        height: 10,
                        logs: vec![register_log("alice", 0xa1)],
                        block_spends: Vec::new(),
                        parent_by_value_hash: HashMap::new(),
                    }],
                },
            )]),
        };
        let indexer = SingletonIndexer::new(
            crate::MemorySingletonStore::shared() as Arc<dyn SingletonStore>,
            crate::MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
            crate::MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
            crate::MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
            Arc::new(RwLock::new(FreshnessState::fresh_at(
                10,
                FreshnessState::now_unix(),
            ))),
        );
        let original_a_coin = current_a.coin;
        let mut registries = vec![current_a, current_b];

        let error =
            process_spent_registry_records(&source, &indexer, &mut registries, vec![spent_a])
                .await
                .unwrap_err();

        assert!(error.to_string().contains("missing coin record"));
        assert_eq!(registries[0].coin, original_a_coin);
        let base = spawn_registration_api(vec![launcher_a, launcher_b], &indexer).await;
        let alice = reqwest::get(format!(
            "{base}/registrations/alice?launcher_id={launcher_a}"
        ))
        .await
        .unwrap();
        assert_eq!(alice.status(), StatusCode::NOT_FOUND);
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

    #[test]
    fn coinset_names_batches_cap_at_500() {
        assert_eq!(COINSET_NAMES_BATCH, 500);
        let ids = vec![Bytes32::default(); 501];
        let batches: Vec<&[Bytes32]> = ids.chunks(COINSET_NAMES_BATCH).collect();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 500);
        assert_eq!(batches[1].len(), 1);
    }

    #[test]
    fn testnet11_schedule_price_follows_time() {
        assert_eq!(schedule_base_price_at(0, true), 1);
        assert_eq!(schedule_base_price_at(1_786_885_199, true), 1);
        assert_eq!(schedule_base_price_at(1_786_885_200, true), 9);
        assert_eq!(schedule_base_price_at(1_786_935_600, true), 6);
        assert_eq!(schedule_base_price_at(1_786_953_600, true), 5);
        assert_eq!(schedule_base_price_at(1_787_022_000, true), 1);
        assert_eq!(schedule_base_price_at(u64::MAX, true), 1);
    }

    #[test]
    fn mainnet_schedule_price_follows_time() {
        assert_eq!(schedule_base_price_at(0, false), 1);
        assert_eq!(schedule_base_price_at(1_787_216_399, false), 1);
        assert_eq!(schedule_base_price_at(1_787_216_400, false), 5_000_000);
        assert_eq!(schedule_base_price_at(1_788_426_000, false), 5_000);
    }

    #[test]
    fn testnet11_typed_schedule_matches_csv() {
        let path = format!(
            "{}/xchandles_price_schedule_testnet11.csv",
            env!("CARGO_MANIFEST_DIR")
        );
        let records = crate::load_xchandles_state_schedule_csv(path).unwrap();
        let from_csv: Vec<(u64, u64)> = records
            .iter()
            .map(|r| (r.timestamp, r.registration_price))
            .collect();
        assert_eq!(
            from_csv.as_slice(),
            crate::TESTNET11_PRICE_SCHEDULE.as_slice()
        );
    }
}

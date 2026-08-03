use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use chia_protocol::Bytes32;
use clvm_utils::ToTreeHash;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use super::auction_pricing::{
    SOON_WINDOW_SECONDS, auction_premium, base_registration_fee, projected_pricing_timestamp,
    reaches_base_at,
};
use super::error::ApiError;
use super::freshness::FreshnessState;
use super::handle_store::{HandleSlotStore, StoredHandleSlot};
use super::pending_store::PendingUpdateStore;
use super::registration_store::{RegistrationActionKind, RegistrationStore, StoredRegistration};
use super::store::{FollowRecordStatus, SingletonStore, StoredSingletonState};
use super::types::{
    ExpiringActiveItem, ExpiringActiveResponse, ExpiringQuery, ExpiringSoonItem,
    ExpiringSoonResponse, ExpiringView, HandleProofResponse, HandleQuery, HandleSlotJson,
    PendingTransferQuery, PendingTransferResponse, RecentRegistrationItem,
    RecentRegistrationsQuery, RecentRegistrationsResponse, RegistrationQuery, RegistrationResponse,
    SingletonQuery, SingletonResponse, SlotNeighborsJson, hex32, is_canonical_handle,
    parse_launcher_id,
};
use crate::{BASE_PRICE_AT_FACTOR_ONE, REGISTRATION_PERIOD};

/// Confirmed pricing inputs for one followed registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryPricing {
    pub base_price: u64,
    pub registration_period: u64,
}

impl Default for RegistryPricing {
    fn default() -> Self {
        Self {
            // Settled post-schedule mainnet base (factor 1).
            base_price: BASE_PRICE_AT_FACTOR_ONE,
            registration_period: REGISTRATION_PERIOD,
        }
    }
}

#[derive(Clone)]
pub struct ListenerApiState {
    pub store: Arc<dyn SingletonStore>,
    pub handle_slots: Arc<dyn HandleSlotStore>,
    pub registrations: Arc<dyn RegistrationStore>,
    pub pending_updates: Arc<dyn PendingUpdateStore>,
    pub freshness: Arc<RwLock<FreshnessState>>,
    /// Per-registry confirmed base price / registration period.
    pub registry_pricing: Arc<RwLock<HashMap<Bytes32, RegistryPricing>>>,
    /// Configured registries in follow order; omission of `launcher_id` selects the first.
    pub registry_launcher_ids: Vec<Bytes32>,
    /// Optional clock override for tests (unix seconds).
    pub now_unix_override: Option<u64>,
}

impl ListenerApiState {
    pub fn new(
        store: Arc<dyn SingletonStore>,
        handle_slots: Arc<dyn HandleSlotStore>,
        registrations: Arc<dyn RegistrationStore>,
        pending_updates: Arc<dyn PendingUpdateStore>,
        freshness: FreshnessState,
        registry_launcher_ids: Vec<Bytes32>,
    ) -> Self {
        let mut pricing = HashMap::new();
        for id in &registry_launcher_ids {
            pricing.insert(*id, RegistryPricing::default());
        }
        Self {
            store,
            handle_slots,
            registrations,
            pending_updates,
            freshness: Arc::new(RwLock::new(freshness)),
            registry_pricing: Arc::new(RwLock::new(pricing)),
            registry_launcher_ids,
            now_unix_override: None,
        }
    }

    fn now_unix(&self) -> u64 {
        self.now_unix_override.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        })
    }
}

/// Public listener router: CORS is credential-free for any origin on GET/HEAD.
pub fn listener_router(state: ListenerApiState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route(
            "/singletons/{launcher_id}",
            get(get_singleton).head(head_singleton),
        )
        .route(
            "/handle/{handle}/pending-transfer",
            get(get_pending_transfer).head(head_pending_transfer),
        )
        .route("/handle/{handle}", get(get_handle).head(head_handle))
        // Static path before `{handle}` so the handle "recent" never shadows this feed.
        .route(
            "/registrations/recent",
            get(get_registrations_recent).head(head_registrations_recent),
        )
        .route(
            "/registrations/{handle}",
            get(get_registration).head(head_registration),
        )
        .route("/expiring", get(get_expiring).head(head_expiring))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
                .allow_headers(Any),
        )
        .with_state(state)
}

async fn require_fresh(state: &ListenerApiState) -> Result<(u32, u64), ApiError> {
    let freshness = state.freshness.read().await;
    let now = state.now_unix();
    if !freshness.is_fresh(now) {
        return Err(ApiError::index_stale(
            freshness.indexed_peak_height,
            freshness.upstream_peak_height,
        ));
    }
    Ok((
        freshness.indexed_peak_height,
        freshness.confirmed_timestamp,
    ))
}

/// Opaque cursor: `v1.{expiration}.{handle}` — stable for the canonical projection order.
fn encode_cursor(expiration: u64, handle: &str) -> String {
    format!("v1.{expiration}.{handle}")
}

fn decode_cursor(cursor: &str) -> Option<(u64, String)> {
    let rest = cursor.strip_prefix("v1.")?;
    let (expiration_raw, handle) = rest.split_once('.')?;
    let expiration = expiration_raw.parse().ok()?;
    if !is_canonical_handle(handle) {
        return None;
    }
    Some((expiration, handle.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExpiringKey {
    expiration: u64,
    handle: String,
}

fn after_cursor(key: &ExpiringKey, cursor: Option<&(u64, String)>) -> bool {
    match cursor {
        None => true,
        Some((exp, handle)) => {
            key.expiration > *exp || (key.expiration == *exp && key.handle.as_str() > handle.as_str())
        }
    }
}

fn state_to_response(
    state: &StoredSingletonState,
    indexed_peak_height: u32,
    include_metadata: bool,
) -> SingletonResponse {
    SingletonResponse {
        launcher_id: hex32(state.launcher_id),
        parent_coin_id: hex32(state.parent_coin_id),
        amount: state.amount,
        inner_puzzle_hash: hex32(state.inner_puzzle_hash),
        confirmation_height: state.confirmation_height,
        melted: state.melted,
        melt_height: state.melt_height,
        nft: state.nft.as_ref().map(|n| n.to_public(include_metadata)),
        indexed_peak_height,
    }
}

fn slot_to_json(slot: &StoredHandleSlot) -> HandleSlotJson {
    HandleSlotJson {
        counter: slot.counter,
        handle_hash: hex32(slot.handle_hash),
        neighbors: SlotNeighborsJson {
            left_value: hex32(slot.neighbors_left),
            right_value: hex32(slot.neighbors_right),
        },
        expiration: slot.expiration,
        owner_launcher_id: hex32(slot.owner_launcher_id),
        resolved_launcher_id: hex32(slot.resolved_launcher_id),
    }
}

fn action_kind_str(kind: RegistrationActionKind) -> &'static str {
    match kind {
        RegistrationActionKind::Register => "register",
        RegistrationActionKind::Expire => "expire",
    }
}

fn registration_to_response(
    reg: &StoredRegistration,
    indexed_peak_height: u32,
) -> RegistrationResponse {
    RegistrationResponse {
        handle: reg.handle.clone(),
        registration_secret: hex32(reg.registration_secret),
        action_kind: action_kind_str(reg.action_kind).to_string(),
        protocol_fee: reg.protocol_fee,
        confirmation_height: reg.confirmation_height,
        indexed_peak_height,
    }
}

async fn lookup_singleton(
    state: &ListenerApiState,
    launcher_id: Bytes32,
    include_metadata: bool,
) -> Result<SingletonResponse, ApiError> {
    let (indexed_peak_height, _confirmed_timestamp) = require_fresh(state).await?;
    let record = state
        .store
        .get(launcher_id)
        .await
        .ok_or_else(ApiError::singleton_not_followed)?;

    // After dereference finality, record should already be removed; belt-and-suspenders.
    if record.reference_count == 0 {
        if let Some(deref_h) = record.dereference_height {
            if indexed_peak_height >= deref_h.saturating_add(32) {
                return Err(ApiError::singleton_not_followed());
            }
        }
    }

    match record.status {
        FollowRecordStatus::Incomplete => Err(ApiError::singleton_incomplete()),
        FollowRecordStatus::Mismatch => Err(ApiError::singleton_mismatch()),
        FollowRecordStatus::Active => {
            let current = record
                .current
                .as_ref()
                .ok_or_else(ApiError::singleton_incomplete)?;
            Ok(state_to_response(
                current,
                indexed_peak_height,
                include_metadata,
            ))
        }
    }
}

fn select_registry(
    state: &ListenerApiState,
    launcher_id_param: Option<&str>,
) -> Result<Bytes32, ApiError> {
    match launcher_id_param {
        None => state
            .registry_launcher_ids
            .first()
            .copied()
            .ok_or_else(ApiError::registry_not_followed),
        Some(raw) => {
            let id = parse_launcher_id(raw).map_err(|_| ApiError::invalid_launcher_id())?;
            if state.registry_launcher_ids.iter().any(|r| *r == id) {
                Ok(id)
            } else {
                Err(ApiError::registry_not_followed())
            }
        }
    }
}

async fn lookup_resolved_for_proof(
    state: &ListenerApiState,
    launcher_id: Bytes32,
    indexed_peak_height: u32,
    include_metadata: bool,
) -> Result<SingletonResponse, ApiError> {
    let record = state
        .store
        .get(launcher_id)
        .await
        .ok_or_else(ApiError::resolution_incomplete)?;

    match record.status {
        FollowRecordStatus::Incomplete => Err(ApiError::resolution_incomplete()),
        FollowRecordStatus::Mismatch => Err(ApiError::resolution_mismatch()),
        FollowRecordStatus::Active => {
            let current = record
                .current
                .as_ref()
                .ok_or_else(ApiError::resolution_incomplete)?;
            Ok(state_to_response(
                current,
                indexed_peak_height,
                include_metadata,
            ))
        }
    }
}

async fn lookup_handle_proof(
    state: &ListenerApiState,
    handle: &str,
    query: &HandleQuery,
) -> Result<HandleProofResponse, ApiError> {
    if !is_canonical_handle(handle) {
        return Err(ApiError::invalid_handle());
    }

    let registry = select_registry(state, query.launcher_id.as_deref())?;
    let (indexed_peak_height, _confirmed_timestamp) = require_fresh(state).await?;

    let handle_hash: Bytes32 = handle.tree_hash().into();
    let record = state
        .handle_slots
        .get(registry, handle_hash)
        .await
        .ok_or_else(ApiError::handle_not_found)?;
    let slot = record
        .current
        .as_ref()
        .ok_or_else(ApiError::handle_not_found)?;

    let now = state.now_unix();
    // Fail-closed: suppress once on-chain expiration has passed unless bypass is set.
    // Immediate suppression is within the "no later than 30 minutes after" bound.
    if now >= slot.expiration && !query.bypass_expiration_safety_check {
        return Err(ApiError::handle_expired(slot.expiration));
    }

    let resolved = lookup_resolved_for_proof(
        state,
        slot.resolved_launcher_id,
        indexed_peak_height,
        query.include_metadata,
    )
    .await?;

    // Owner Singleton incompleteness must not invalidate a complete Resolved proof.
    // (No Owner lookup is performed here.)

    Ok(HandleProofResponse {
        registry_launcher_id: hex32(registry),
        handle: handle.to_string(),
        slot: slot_to_json(slot),
        slot_parent_coin_id: hex32(slot.parent_coin_id),
        slot_confirmation_height: slot.confirmation_height,
        resolved_singleton: resolved,
        indexed_peak_height,
    })
}

async fn lookup_registration(
    state: &ListenerApiState,
    handle: &str,
    query: &RegistrationQuery,
) -> Result<RegistrationResponse, ApiError> {
    if !is_canonical_handle(handle) {
        return Err(ApiError::invalid_handle());
    }

    let registry = select_registry(state, query.launcher_id.as_deref())?;
    let (indexed_peak_height, _confirmed_timestamp) = require_fresh(state).await?;

    let handle_hash: Bytes32 = handle.tree_hash().into();
    let record = state
        .registrations
        .get(registry, handle_hash)
        .await
        .ok_or_else(ApiError::handle_not_found)?;
    let reg = record
        .current
        .as_ref()
        .ok_or_else(ApiError::handle_not_found)?;

    // Readable after Handle Expiration — no expiration safety gate here.
    Ok(registration_to_response(reg, indexed_peak_height))
}

async fn lookup_registrations_recent(
    state: &ListenerApiState,
    query: &RecentRegistrationsQuery,
) -> Result<RecentRegistrationsResponse, ApiError> {
    let registry = select_registry(state, query.launcher_id.as_deref())?;
    let (indexed_peak_height, _confirmed_timestamp) = require_fresh(state).await?;

    let limit = query.limit.unwrap_or(50).min(50) as usize;
    let stats = state.registrations.get_stats(registry).await;
    let items = stats
        .events
        .iter()
        .rev()
        .take(limit)
        .map(|ev| RecentRegistrationItem {
            handle: ev.handle.clone(),
            action_kind: action_kind_str(ev.action_kind).to_string(),
            confirmation_height: ev.confirmation_height,
        })
        .collect();

    Ok(RecentRegistrationsResponse {
        items,
        total_registered: stats.total_registered,
        indexed_peak_height,
    })
}

/// Returns `Ok(Some(_))` when performable, `Ok(None)` for 204, or an API error.
async fn lookup_pending_transfer(
    state: &ListenerApiState,
    handle: &str,
    query: &PendingTransferQuery,
) -> Result<Option<PendingTransferResponse>, ApiError> {
    if !is_canonical_handle(handle) {
        return Err(ApiError::invalid_handle());
    }

    let registry = select_registry(state, query.launcher_id.as_deref())?;
    let (_indexed_peak_height, _confirmed_timestamp) = require_fresh(state).await?;

    let handle_hash: Bytes32 = handle.tree_hash().into();
    let handle_record = state
        .handle_slots
        .get(registry, handle_hash)
        .await
        .ok_or_else(ApiError::handle_not_found)?;
    let slot = handle_record
        .current
        .as_ref()
        .ok_or_else(ApiError::handle_not_found)?;

    // Expired Handles are not performable — distinct from unified proof's 410.
    if state.now_unix() >= slot.expiration {
        return Ok(None);
    }

    let pending_record = state.pending_updates.get(registry, handle_hash).await;
    let Some(pending) = pending_record.as_ref().and_then(|r| r.current.as_ref()) else {
        return Ok(None);
    };

    // Executor must be the live Owner Singleton coin whose parent is the initiator.
    let Some(owner_rec) = state.store.get(slot.owner_launcher_id).await else {
        return Ok(None);
    };
    if owner_rec.status != FollowRecordStatus::Active {
        return Ok(None);
    }
    let Some(owner) = owner_rec.current.as_ref() else {
        return Ok(None);
    };
    if owner.melted {
        return Ok(None);
    }
    // Separately spent lineage: current coin is no longer the post-initiate executor.
    if owner.parent_coin_id != pending.update_initiator_coin_id {
        return Ok(None);
    }

    Ok(Some(PendingTransferResponse {
        handle_hash: hex32(pending.handle_hash),
        new_owner_launcher_id: hex32(pending.new_owner_launcher_id),
        new_resolved_launcher_id: hex32(pending.new_resolved_launcher_id),
        update_confirmation_height: pending.update_confirmation_height,
        minimum_execution_height: pending.minimum_execution_height,
        update_initiator_coin_id: hex32(pending.update_initiator_coin_id),
        current_executor_coin_id: hex32(owner.coin_id),
    }))
}

async fn collect_named_slots(
    state: &ListenerApiState,
    registry: Bytes32,
) -> Vec<(String, StoredHandleSlot)> {
    let mut out = Vec::new();
    for (reg, handle_hash) in state.handle_slots.all_keys().await {
        if reg != registry {
            continue;
        }
        let Some(record) = state.handle_slots.get(reg, handle_hash).await else {
            continue;
        };
        let Some(slot) = record.current else {
            continue;
        };
        // Cold-backfill slots without a registration fact lack a Handle string — skip.
        let Some(reg_rec) = state.registrations.get(reg, handle_hash).await else {
            continue;
        };
        let Some(reg_cur) = reg_rec.current else {
            continue;
        };
        out.push((reg_cur.handle, slot));
    }
    out
}

async fn lookup_expiring_active(
    state: &ListenerApiState,
    query: &ExpiringQuery,
) -> Result<ExpiringActiveResponse, ApiError> {
    let registry = select_registry(state, query.launcher_id.as_deref())?;
    let (indexed_peak_height, confirmed_timestamp) = require_fresh(state).await?;
    let now = state.now_unix();
    let projected = projected_pricing_timestamp(confirmed_timestamp);
    let pricing = state
        .registry_pricing
        .read()
        .await
        .get(&registry)
        .copied()
        .unwrap_or_default();

    let cursor = query.cursor.as_deref().and_then(decode_cursor);
    // Malformed cursors restart from the beginning rather than inventing a new error code.

    let limit = query.limit.unwrap_or(50).min(50) as usize;
    let mut rows: Vec<(ExpiringKey, ExpiringActiveItem)> = Vec::new();
    for (handle, slot) in collect_named_slots(state, registry).await {
        // Fail-closed at/after expiry (Ticket 11 alignment).
        if now < slot.expiration {
            continue;
        }
        let premium = auction_premium(slot.expiration, projected);
        if premium == 0 {
            continue;
        }
        let key = ExpiringKey {
            expiration: slot.expiration,
            handle: handle.clone(),
        };
        if !after_cursor(&key, cursor.as_ref()) {
            continue;
        }
        let base = base_registration_fee(pricing.base_price, &handle);
        rows.push((
            key,
            ExpiringActiveItem {
                handle,
                expiration: slot.expiration,
                projected_pricing_timestamp: projected,
                current_premium: premium,
                total_registration_fee: base.saturating_add(premium),
                base_registration_fee: base,
                reaches_base_at: reaches_base_at(slot.expiration),
            },
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let next_cursor = if rows.len() > limit {
        let last = &rows[limit - 1].0;
        Some(encode_cursor(last.expiration, &last.handle))
    } else {
        None
    };
    let items = rows.into_iter().take(limit).map(|(_, item)| item).collect();

    Ok(ExpiringActiveResponse {
        items,
        next_cursor,
        indexed_peak_height,
        confirmed_timestamp,
    })
}

async fn lookup_expiring_soon(
    state: &ListenerApiState,
    query: &ExpiringQuery,
) -> Result<ExpiringSoonResponse, ApiError> {
    let registry = select_registry(state, query.launcher_id.as_deref())?;
    let (indexed_peak_height, confirmed_timestamp) = require_fresh(state).await?;
    let now = state.now_unix();
    let pricing = state
        .registry_pricing
        .read()
        .await
        .get(&registry)
        .copied()
        .unwrap_or_default();

    let cursor = query.cursor.as_deref().and_then(decode_cursor);

    let limit = query.limit.unwrap_or(50).min(50) as usize;
    let soon_deadline = now.saturating_add(SOON_WINDOW_SECONDS);
    let mut rows: Vec<(ExpiringKey, ExpiringSoonItem)> = Vec::new();
    for (handle, slot) in collect_named_slots(state, registry).await {
        // Active (not yet expired) and within the inclusive 30-day window.
        if slot.expiration <= now || slot.expiration > soon_deadline {
            continue;
        }
        let key = ExpiringKey {
            expiration: slot.expiration,
            handle: handle.clone(),
        };
        if !after_cursor(&key, cursor.as_ref()) {
            continue;
        }
        rows.push((
            key,
            ExpiringSoonItem {
                handle: handle.clone(),
                expiration: slot.expiration,
                base_registration_fee: base_registration_fee(pricing.base_price, &handle),
            },
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let next_cursor = if rows.len() > limit {
        let last = &rows[limit - 1].0;
        Some(encode_cursor(last.expiration, &last.handle))
    } else {
        None
    };
    let items = rows.into_iter().take(limit).map(|(_, item)| item).collect();

    Ok(ExpiringSoonResponse {
        items,
        next_cursor,
        indexed_peak_height,
        confirmed_timestamp,
    })
}

async fn get_expiring(
    State(state): State<ListenerApiState>,
    Query(query): Query<ExpiringQuery>,
) -> Result<impl IntoResponse, ApiError> {
    match ExpiringView::parse(query.view.as_deref()) {
        Some(ExpiringView::Active) => {
            let body = lookup_expiring_active(&state, &query).await?;
            Ok(Json(body).into_response())
        }
        Some(ExpiringView::Soon) => {
            let body = lookup_expiring_soon(&state, &query).await?;
            Ok(Json(body).into_response())
        }
        None => Err(ApiError::invalid_view()),
    }
}

async fn head_expiring(
    State(state): State<ListenerApiState>,
    Query(query): Query<ExpiringQuery>,
) -> Result<StatusCode, ApiError> {
    match ExpiringView::parse(query.view.as_deref()) {
        Some(ExpiringView::Active) => {
            let _ = lookup_expiring_active(&state, &query).await?;
            Ok(StatusCode::OK)
        }
        Some(ExpiringView::Soon) => {
            let _ = lookup_expiring_soon(&state, &query).await?;
            Ok(StatusCode::OK)
        }
        None => Err(ApiError::invalid_view()),
    }
}

async fn get_singleton(
    State(state): State<ListenerApiState>,
    Path(launcher_id_raw): Path<String>,
    Query(query): Query<SingletonQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let launcher_id =
        parse_launcher_id(&launcher_id_raw).map_err(|_| ApiError::invalid_launcher_id())?;
    let body = lookup_singleton(&state, launcher_id, query.include_metadata).await?;
    Ok(Json(body))
}

async fn head_singleton(
    State(state): State<ListenerApiState>,
    Path(launcher_id_raw): Path<String>,
    Query(query): Query<SingletonQuery>,
) -> Result<StatusCode, ApiError> {
    let launcher_id =
        parse_launcher_id(&launcher_id_raw).map_err(|_| ApiError::invalid_launcher_id())?;
    let _ = lookup_singleton(&state, launcher_id, query.include_metadata).await?;
    Ok(StatusCode::OK)
}

async fn get_handle(
    State(state): State<ListenerApiState>,
    Path(handle): Path<String>,
    Query(query): Query<HandleQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let body = lookup_handle_proof(&state, &handle, &query).await?;
    Ok(Json(body))
}

async fn head_handle(
    State(state): State<ListenerApiState>,
    Path(handle): Path<String>,
    Query(query): Query<HandleQuery>,
) -> Result<StatusCode, ApiError> {
    let _ = lookup_handle_proof(&state, &handle, &query).await?;
    Ok(StatusCode::OK)
}

async fn get_registration(
    State(state): State<ListenerApiState>,
    Path(handle): Path<String>,
    Query(query): Query<RegistrationQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let body = lookup_registration(&state, &handle, &query).await?;
    Ok(Json(body))
}

async fn head_registration(
    State(state): State<ListenerApiState>,
    Path(handle): Path<String>,
    Query(query): Query<RegistrationQuery>,
) -> Result<StatusCode, ApiError> {
    let _ = lookup_registration(&state, &handle, &query).await?;
    Ok(StatusCode::OK)
}

async fn get_registrations_recent(
    State(state): State<ListenerApiState>,
    Query(query): Query<RecentRegistrationsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let body = lookup_registrations_recent(&state, &query).await?;
    Ok(Json(body))
}

async fn head_registrations_recent(
    State(state): State<ListenerApiState>,
    Query(query): Query<RecentRegistrationsQuery>,
) -> Result<StatusCode, ApiError> {
    let _ = lookup_registrations_recent(&state, &query).await?;
    Ok(StatusCode::OK)
}

async fn get_pending_transfer(
    State(state): State<ListenerApiState>,
    Path(handle): Path<String>,
    Query(query): Query<PendingTransferQuery>,
) -> Result<impl IntoResponse, ApiError> {
    match lookup_pending_transfer(&state, &handle, &query).await? {
        Some(body) => Ok((StatusCode::OK, Json(body)).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

async fn head_pending_transfer(
    State(state): State<ListenerApiState>,
    Path(handle): Path<String>,
    Query(query): Query<PendingTransferQuery>,
) -> Result<StatusCode, ApiError> {
    match lookup_pending_transfer(&state, &handle, &query).await? {
        Some(_) => Ok(StatusCode::OK),
        None => Ok(StatusCode::NO_CONTENT),
    }
}

/// Bind + serve helper used by production listen and the real-HTTP test fixture.
pub async fn serve_listener(
    state: ListenerApiState,
    bind: std::net::SocketAddr,
) -> Result<(), std::io::Error> {
    let app = listener_router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await
}

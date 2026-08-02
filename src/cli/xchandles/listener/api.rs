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

use super::error::ApiError;
use super::freshness::FreshnessState;
use super::handle_store::{HandleSlotStore, StoredHandleSlot};
use super::store::{FollowRecordStatus, SingletonStore, StoredSingletonState};
use super::types::{
    hex32, is_canonical_handle, parse_launcher_id, HandleProofResponse, HandleQuery, HandleSlotJson,
    SingletonQuery, SingletonResponse, SlotNeighborsJson,
};

#[derive(Clone)]
pub struct ListenerApiState {
    pub store: Arc<dyn SingletonStore>,
    pub handle_slots: Arc<dyn HandleSlotStore>,
    pub freshness: Arc<RwLock<FreshnessState>>,
    /// Configured registries in follow order; omission of `launcher_id` selects the first.
    pub registry_launcher_ids: Vec<Bytes32>,
    /// Optional clock override for tests (unix seconds).
    pub now_unix_override: Option<u64>,
}

impl ListenerApiState {
    pub fn new(
        store: Arc<dyn SingletonStore>,
        handle_slots: Arc<dyn HandleSlotStore>,
        freshness: FreshnessState,
        registry_launcher_ids: Vec<Bytes32>,
    ) -> Self {
        Self {
            store,
            handle_slots,
            freshness: Arc::new(RwLock::new(freshness)),
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
        .route("/handle/{handle}", get(get_handle).head(head_handle))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
                .allow_headers(Any),
        )
        .with_state(state)
}

async fn require_fresh(state: &ListenerApiState) -> Result<u32, ApiError> {
    let freshness = state.freshness.read().await;
    let now = state.now_unix();
    if !freshness.is_fresh(now) {
        return Err(ApiError::index_stale(
            freshness.indexed_peak_height,
            freshness.upstream_peak_height,
        ));
    }
    Ok(freshness.indexed_peak_height)
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

async fn lookup_singleton(
    state: &ListenerApiState,
    launcher_id: Bytes32,
    include_metadata: bool,
) -> Result<SingletonResponse, ApiError> {
    let indexed_peak_height = require_fresh(state).await?;
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
    let indexed_peak_height = require_fresh(state).await?;

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

/// Bind + serve helper used by production listen and the real-HTTP test fixture.
pub async fn serve_listener(
    state: ListenerApiState,
    bind: std::net::SocketAddr,
) -> Result<(), std::io::Error> {
    let app = listener_router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await
}

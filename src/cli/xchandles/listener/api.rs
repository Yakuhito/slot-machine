use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use chia_protocol::Bytes32;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use super::error::ApiError;
use super::freshness::FreshnessState;
use super::store::{FollowRecordStatus, SingletonStore, StoredSingletonState};
use super::types::{hex32, parse_launcher_id, SingletonQuery, SingletonResponse};

#[derive(Clone)]
pub struct ListenerApiState {
    pub store: Arc<dyn SingletonStore>,
    pub freshness: Arc<RwLock<FreshnessState>>,
}

impl ListenerApiState {
    pub fn new(store: Arc<dyn SingletonStore>, freshness: FreshnessState) -> Self {
        Self {
            store,
            freshness: Arc::new(RwLock::new(freshness)),
        }
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
    let now = FreshnessState::now_unix();
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
        nft: state
            .nft
            .as_ref()
            .map(|n| n.to_public(include_metadata)),
        indexed_peak_height,
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
            Ok(state_to_response(current, indexed_peak_height, include_metadata))
        }
    }
}

async fn get_singleton(
    State(state): State<ListenerApiState>,
    Path(launcher_id_raw): Path<String>,
    Query(query): Query<SingletonQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let launcher_id = parse_launcher_id(&launcher_id_raw).map_err(|_| ApiError::invalid_launcher_id())?;
    let body = lookup_singleton(&state, launcher_id, query.include_metadata).await?;
    Ok(Json(body))
}

async fn head_singleton(
    State(state): State<ListenerApiState>,
    Path(launcher_id_raw): Path<String>,
    Query(query): Query<SingletonQuery>,
) -> Result<StatusCode, ApiError> {
    let launcher_id = parse_launcher_id(&launcher_id_raw).map_err(|_| ApiError::invalid_launcher_id())?;
    let _ = lookup_singleton(&state, launcher_id, query.include_metadata).await?;
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

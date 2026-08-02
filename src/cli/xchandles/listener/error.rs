use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use super::types::ApiErrorBody;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
    pub request_id: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
            request_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = request_id.into();
        self
    }

    pub fn invalid_launcher_id() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_launcher_id",
            "Launcher ID must be 32-byte lowercase hex without a required 0x prefix",
        )
    }

    pub fn singleton_not_followed() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "singleton_not_followed",
            "Launcher is not followed by any canonical Handle slot",
        )
    }

    pub fn singleton_incomplete() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "singleton_incomplete",
            "Referenced singleton has not been reconstructed yet",
        )
    }

    pub fn singleton_mismatch() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "singleton_mismatch",
            "Block-local singleton discovery found an integrity mismatch",
        )
    }

    pub fn index_stale(indexed_peak_height: u32, upstream_peak_height: u32) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "index_stale",
            "Listener index is stale, rolling back, or resyncing",
        )
        .with_details(json!({
            "indexed_peak_height": indexed_peak_height,
            "upstream_peak_height": upstream_peak_height,
        }))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            code: self.code.to_string(),
            message: self.message,
            request_id: self.request_id,
            details: self.details,
        };
        (self.status, Json(body)).into_response()
    }
}

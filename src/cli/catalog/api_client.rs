use reqwest::{Client, Error as ReqwestError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct NeighborResponse {
    pub parent_parent_info: String,
    pub parent_inner_puzzle_hash: String,
    pub asset_id: String,
    pub left_asset_id: String,
    pub right_asset_id: String,
}

#[derive(Debug)]
pub enum ApiClientError {
    RequestError(ReqwestError),
    InvalidResponse(String),
}

impl From<ReqwestError> for ApiClientError {
    fn from(err: ReqwestError) -> Self {
        ApiClientError::RequestError(err)
    }
}

pub struct CatalogApiClient {
    client: Client,
    base_url: String,
}

impl CatalogApiClient {
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: base_url.to_string(),
        }
    }

    pub async fn health_check(&self) -> Result<bool, ApiClientError> {
        let response = self
            .client
            .get(&format!("{}/", self.base_url))
            .send()
            .await?;

        if response.status().is_success() {
            Ok(true)
        } else {
            Err(ApiClientError::InvalidResponse(format!(
                "Health check failed with status: {}",
                response.status()
            )))
        }
    }

    pub async fn get_neighbors(&self, asset_id: &str) -> Result<NeighborResponse, ApiClientError> {
        let url = format!("{}/neighbors?asset_id={}", self.base_url, asset_id);
        let response = self.client.get(&url).send().await?;

        if response.status().is_success() {
            let neighbors = response.json::<NeighborResponse>().await?;
            Ok(neighbors)
        } else {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("Error text unavailable"));
            Err(ApiClientError::InvalidResponse(format!(
                "Failed to get neighbors: {} - {}",
                response.status(),
                error_text
            )))
        }
    }
}

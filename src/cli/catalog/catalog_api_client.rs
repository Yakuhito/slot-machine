use chia::protocol::Bytes32;
use reqwest::Client;
use std::time::Duration;

use crate::{hex_string_to_bytes32, CliError};
use chia_wallet_sdk::driver::{Slot, SlotProof};
use chia_wallet_sdk::types::puzzles::{CatalogSlotValue, SlotInfo};

use super::CatalogNeighborResponse;

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

    pub fn mainnet() -> Self {
        Self::new("https://api.catalog.cat")
    }

    pub fn testnet() -> Self {
        // Self::new("https://testnet11-api.catalog.cat")
        Self::new("http://localhost:3000")
    }

    pub fn get(testnet11: bool) -> Self {
        if testnet11 {
            Self::testnet()
        } else {
            Self::mainnet()
        }
    }

    pub async fn health_check(&self) -> Result<(), CliError> {
        let response = self
            .client
            .get(format!("{}/", self.base_url))
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(CliError::Custom(format!(
                "Health check failed with status: {}",
                response.status()
            )))
        }
    }

    pub async fn get_neighbors(
        &self,
        launcher_id: Bytes32,
        asset_id: Bytes32,
    ) -> Result<(Slot<CatalogSlotValue>, Slot<CatalogSlotValue>), CliError> {
        let url = format!(
            "{}/neighbors?asset_id={}",
            self.base_url,
            hex::encode(asset_id)
        );
        let response = self.client.get(&url).send().await?;

        let neighbors_resp = response.json::<CatalogNeighborResponse>().await?;

        let left_asset_id = hex_string_to_bytes32(&neighbors_resp.left_asset_id)?;
        let right_asset_id = hex_string_to_bytes32(&neighbors_resp.right_asset_id)?;

        let left_left_asset_id = hex_string_to_bytes32(&neighbors_resp.left_left_asset_id)?;
        let left_value = CatalogSlotValue::new(left_asset_id, left_left_asset_id, right_asset_id);

        let right_right_asset_id = hex_string_to_bytes32(&neighbors_resp.right_right_asset_id)?;
        let right_value =
            CatalogSlotValue::new(right_asset_id, left_asset_id, right_right_asset_id);

        let left_parent_parent_info =
            hex_string_to_bytes32(&neighbors_resp.left_parent_parent_info)?;
        let left_parent_inner_puzzle_hash =
            hex_string_to_bytes32(&neighbors_resp.left_parent_inner_puzzle_hash)?;
        let left_proof = SlotProof {
            parent_parent_info: left_parent_parent_info,
            parent_inner_puzzle_hash: left_parent_inner_puzzle_hash,
        };

        let right_parent_parent_info =
            hex_string_to_bytes32(&neighbors_resp.right_parent_parent_info)?;
        let right_parent_inner_puzzle_hash =
            hex_string_to_bytes32(&neighbors_resp.right_parent_inner_puzzle_hash)?;
        let right_proof = SlotProof {
            parent_parent_info: right_parent_parent_info,
            parent_inner_puzzle_hash: right_parent_inner_puzzle_hash,
        };

        let left_info = SlotInfo::<CatalogSlotValue>::from_value(launcher_id, 0, left_value);
        let left = Slot::<CatalogSlotValue>::new(left_proof, left_info);

        let right_info = SlotInfo::<CatalogSlotValue>::from_value(launcher_id, 0, right_value);
        let right = Slot::<CatalogSlotValue>::new(right_proof, right_info);

        Ok((left, right))
    }
}

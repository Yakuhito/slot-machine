use chia::protocol::Bytes32;
use reqwest::Client;
use std::time::Duration;

use crate::{hex_string_to_bytes32, CliError, Slot, SlotInfo, SlotProof, XchandlesSlotValue};

use super::XchandlesNeighborsResponse;

pub struct XchandlesApiClient {
    client: Client,
    base_url: String,
}

impl XchandlesApiClient {
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
        Self::new("https://api.xchandles.com")
    }

    pub fn testnet() -> Self {
        // Self::new("https://testnet11-api.xchandles.com")
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
        handle_hash: Bytes32,
    ) -> Result<(Slot<XchandlesSlotValue>, Slot<XchandlesSlotValue>), CliError> {
        let url = format!(
            "{}/neighbors?launcher_id={}&handle_hash={}",
            self.base_url,
            hex::encode(launcher_id),
            hex::encode(handle_hash)
        );
        let response = self.client.get(&url).send().await?;

        let neighbors_resp = response.json::<XchandlesNeighborsResponse>().await?;

        let left_handle_hash = hex_string_to_bytes32(&neighbors_resp.left_handle_hash)?;
        let right_handle_hash = hex_string_to_bytes32(&neighbors_resp.right_handle_hash)?;

        let left_left_handle_hash = hex_string_to_bytes32(&neighbors_resp.left_left_handle_hash)?;
        let left_expiration = neighbors_resp.left_expiration;
        let left_owner_launcher_id = hex_string_to_bytes32(&neighbors_resp.left_owner_launcher_id)?;
        let left_resolved_launcher_id =
            hex_string_to_bytes32(&neighbors_resp.left_resolved_launcher_id)?;
        let left_value = XchandlesSlotValue::new(
            left_handle_hash,
            left_left_handle_hash,
            right_handle_hash,
            left_expiration,
            left_owner_launcher_id,
            left_resolved_launcher_id,
        );

        let right_right_handle_hash =
            hex_string_to_bytes32(&neighbors_resp.right_right_handle_hash)?;
        let right_expiration = neighbors_resp.right_expiration;
        let right_owner_launcher_id =
            hex_string_to_bytes32(&neighbors_resp.right_owner_launcher_id)?;
        let right_resolved_launcher_id =
            hex_string_to_bytes32(&neighbors_resp.right_resolved_launcher_id)?;
        let right_value = XchandlesSlotValue::new(
            right_handle_hash,
            left_handle_hash,
            right_right_handle_hash,
            right_expiration,
            right_owner_launcher_id,
            right_resolved_launcher_id,
        );

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

        let left_info = SlotInfo::<XchandlesSlotValue>::from_value(launcher_id, 0, left_value);
        let left = Slot::<XchandlesSlotValue>::new(left_proof, left_info);

        let right_info = SlotInfo::<XchandlesSlotValue>::from_value(launcher_id, 0, right_value);
        let right = Slot::<XchandlesSlotValue>::new(right_proof, right_info);

        Ok((left, right))
    }
}

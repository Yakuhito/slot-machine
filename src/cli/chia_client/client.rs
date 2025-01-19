use chia::protocol::Bytes32;
use reqwest::Client as ReqwestClient;
use serde_json::Value;
use std::error::Error;

use super::{AdditionsAndRemovalsResponse, BlockchainStateResponse, MockChiaClient};

#[derive(Debug)]
pub enum Client {
    Reqwest(ReqwestClient),
    Mock(MockChiaClient),
}

#[derive(Debug)]
pub struct ChiaRpcClient {
    pub base_url: String,
    pub client: Client,
}

impl ChiaRpcClient {
    pub fn new(base_url: &str) -> Self {
        ChiaRpcClient {
            base_url: base_url.to_string(),
            client: Client::Reqwest(ReqwestClient::new()),
        }
    }

    pub fn new_mock() -> Self {
        Self {
            base_url: "http://api.example.com".to_string(),
            client: Client::Mock(MockChiaClient::new()),
        }
    }

    pub fn coinset_testnet11() -> Self {
        Self::new("https://testnet11.api.coinset.org")
    }

    pub fn coinset_mainnet() -> Self {
        Self::new("https://api.coinset.org")
    }

    pub fn coinset(testnet11: bool) -> Self {
        if testnet11 {
            Self::coinset_testnet11()
        } else {
            Self::coinset_mainnet()
        }
    }

    pub async fn make_post_request<ResponseType>(
        &self,
        endpoint: &str,
        json: Value,
    ) -> Result<ResponseType, Box<dyn Error>>
    where
        ResponseType: serde::de::DeserializeOwned,
    {
        let url = format!("{}/{}", self.base_url, endpoint);
        match &self.client {
            Client::Reqwest(client) => {
                let res = client.post(&url).json(&json).send().await?;
                Ok(res.json::<ResponseType>().await?)
            }
            Client::Mock(client) => {
                let res = client.post(&url, json).await?;
                Ok(serde_json::from_str::<ResponseType>(&res)?)
            }
        }
    }

    pub async fn get_blockchain_state(&self) -> Result<BlockchainStateResponse, Box<dyn Error>> {
        self.make_post_request("get_blockchain_state", serde_json::json!({}))
            .await
    }

    pub async fn get_additions_and_removals(
        &self,
        header_hash: Bytes32,
    ) -> Result<AdditionsAndRemovalsResponse, Box<dyn Error>> {
        self.make_post_request(
            "get_additions_and_removals",
            serde_json::json!({
                "header_hash": format!("0x{}", hex::encode(header_hash.to_bytes())),
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_blockchain_state_success() {
        let mut client = ChiaRpcClient::new_mock();

        // Get mock client reference
        if let Client::Mock(mock_client) = &mut client.client {
            mock_client.mock_response(
                "http://api.example.com/get_blockchain_state",
                r#"{
                    "blockchain_state": {
                        "average_block_time": 18,
                        "block_max_cost": 11000000000,
                        "difficulty": 13504,
                        "genesis_challenge_initialized": true,
                        "mempool_cost": 393999880,
                        "mempool_fees": 146444089,
                        "mempool_max_total_cost": 110000000000,
                        "mempool_min_fees": {
                            "cost_5000000": 0
                        },
                        "mempool_size": 5,
                        "node_id": "5c8c1640aae6b0ab0f16d5ec01be46aa10ad68f8aa85446fa65f1aee9d6b0b2d",
                        "peak": {
                            "header_hash": "b98b88d8da393524929708bae7dcb4916b287605dee42fad6e7a72e78007f6ec",
                            "height": 6515259,
                            "prev_hash": "bf24a15571a4755867acd674113c9c2267981991399559c5d32f431a76ae9e16",
                            "weight": 35984730512,
                            "total_iters": 56498920731463,
                            "signage_point_index": 63,
                            "farmer_puzzle_hash": "9fbde16e03f55c85ecf94cb226083fcfe2737d4e629a981e5db3ea0eb9907af4",
                            "required_iters": 4567879,
                            "deficit": 16,
                            "overflow": true,
                            "prev_transaction_block_height": 6515255,
                            "timestamp": null,
                            "prev_transaction_block_hash": null,
                            "fees": null
                        },
                        "space": 22091151228153044992,
                        "sub_slot_iters": 578813952,
                        "sync": {
                            "sync_mode": false,
                            "sync_progress_height": 0,
                            "sync_tip_height": 0,
                            "synced": true
                        }
                    },
                    "success": true
                }"#,
            );
        }

        let response = client.get_blockchain_state().await.unwrap();
        assert!(response.success);
        assert!(response.error.is_none());

        let state = response.blockchain_state.unwrap();
        assert_eq!(state.average_block_time, 18);
        assert_eq!(state.difficulty, 13504);
        assert_eq!(state.mempool_size, 5);

        let peak = state.peak;
        assert_eq!(peak.height, 6515259);
        assert_eq!(peak.deficit, 16);
        assert!(peak.overflow);
    }

    #[tokio::test]
    async fn test_get_blockchain_state_error() {
        let mut client = ChiaRpcClient::new_mock();

        // Get mock client reference
        if let Client::Mock(mock_client) = &mut client.client {
            mock_client.mock_response(
                "http://api.example.com/get_blockchain_state",
                r#"{
                    "success": false,
                    "error": "Failed to connect to full node"
                }"#,
            );
        }

        let response = client.get_blockchain_state().await.unwrap();
        assert!(!response.success);
        assert_eq!(
            response.error,
            Some("Failed to connect to full node".to_string())
        );
        assert!(response.blockchain_state.is_none());
    }
}

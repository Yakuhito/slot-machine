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
    use chia::protocol::Coin;

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

    #[tokio::test]
    async fn test_get_additions_and_removals_success() {
        let mut client = ChiaRpcClient::new_mock();

        if let Client::Mock(mock_client) = &mut client.client {
            mock_client.mock_response(
                "http://api.example.com/get_additions_and_removals",
                r#"{
                    "additions": [{
                        "coin": {
                            "amount": 10019626640,
                            "parent_coin_info": "c325057d788bee13367cb8e2d71ff3e209b5e94b31b296322ba1a143053fef5b",
                            "puzzle_hash": "11cd056d9ec93f4612919b445e1ad9afeb7ef7739708c2d16cec4fd2d3cd5e63"
                        },
                        "coinbase": false,
                        "confirmed_block_index": 5910291,
                        "spent": false,
                        "spent_block_index": 0,
                        "timestamp": 1725991066
                    }],
                    "removals": [{
                        "coin": {
                            "amount": 1,
                            "parent_coin_info": "4dda4b8b6017c633794c2b719c3591870b4bc7682930094c11a311112c772ce6",
                            "puzzle_hash": "18cfd81a9a58d598197730b2f2a21ff3b72951577be1dcc6004080ad17069e84"
                        },
                        "coinbase": false,
                        "confirmed_block_index": 5612341,
                        "spent": true,
                        "spent_block_index": 5910291,
                        "timestamp": 1720407964
                    }],
                    "success": true
                }"#,
            );
        }

        let header_hash = Bytes32::from([0x88; 32]);
        let response = client
            .get_additions_and_removals(header_hash)
            .await
            .unwrap();

        assert!(response.success);
        assert!(response.error.is_none());

        // Check additions
        let additions = response.additions.unwrap();
        assert_eq!(additions.len(), 1);
        let addition = &additions[0];
        assert_eq!(
            addition.coin,
            Coin::new(
                Bytes32::new(hex_literal::hex!(
                    "c325057d788bee13367cb8e2d71ff3e209b5e94b31b296322ba1a143053fef5b"
                )),
                Bytes32::new(hex_literal::hex!(
                    "11cd056d9ec93f4612919b445e1ad9afeb7ef7739708c2d16cec4fd2d3cd5e63"
                )),
                10019626640
            )
        );
        assert!(!addition.coinbase);
        assert_eq!(addition.confirmed_block_index, 5910291);
        assert!(!addition.spent);

        // Check removals
        let removals = response.removals.unwrap();
        assert_eq!(removals.len(), 1);
        let removal = &removals[0];
        assert_eq!(removal.coin.amount, 1);
        assert!(removal.spent);
        assert_eq!(removal.spent_block_index, 5910291);
    }

    #[tokio::test]
    async fn test_get_additions_and_removals_error() {
        let mut client = ChiaRpcClient::new_mock();

        if let Client::Mock(mock_client) = &mut client.client {
            mock_client.mock_response(
                "http://api.example.com/get_additions_and_removals",
                r#"{
                    "success": false,
                    "error": "Record not found: [blah blah]"
                }"#,
            );
        }

        let header_hash = Bytes32::from([0x88; 32]);
        let response = client
            .get_additions_and_removals(header_hash)
            .await
            .unwrap();

        assert!(!response.success);
        assert_eq!(
            response.error,
            Some("Record not found: [blah blah]".to_string())
        );
        assert!(response.additions.is_none());
        assert!(response.removals.is_none());
    }
}

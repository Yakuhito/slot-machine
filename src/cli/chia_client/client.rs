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

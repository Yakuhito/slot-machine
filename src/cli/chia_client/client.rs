use reqwest::Client;
use std::error::Error;

use super::BlockchainStateResponse;

#[derive(Debug)]
pub struct ChiaRpcClient {
    base_url: String,
    client: Client,
}

impl ChiaRpcClient {
    pub fn new(base_url: &str) -> Self {
        ChiaRpcClient {
            base_url: base_url.to_string(),
            client: Client::new(),
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

    pub async fn get_blockchain_state(&self) -> Result<BlockchainStateResponse, Box<dyn Error>> {
        let url = format!("{}/get_blockchain_state", self.base_url);
        let res = self
            .client
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await?;

        let response = res.json::<BlockchainStateResponse>().await?;
        Ok(response)
    }
}

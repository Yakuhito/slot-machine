use chia::protocol::CoinSpend;
use dirs::data_dir;
use reqwest::Identity;
use sage_api::{
    Amount, Assets, CatAmount, CoinJson, CoinSpendJson, GetDerivations, GetDerivationsResponse,
    MakeOffer, MakeOfferResponse, SendCat, SendCatResponse, SendXch, SendXchResponse,
    SignCoinSpends, SignCoinSpendsResponse,
};
use thiserror::Error;

use super::CliError;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Failed to load certificate")]
    CertificateError,
    #[error("Request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Failed to build client")]
    ClientBuildError,
}

pub struct SageClient {
    client: reqwest::Client,
    base_url: String,
}

impl SageClient {
    pub fn new() -> Result<Self, CliError> {
        let data_dir = data_dir().ok_or(CliError::DataDirNotFound)?;

        let cert_file = data_dir.join("com.rigidnetwork.sage/ssl/wallet.crt");
        let key_file = data_dir.join("com.rigidnetwork.sage/ssl/wallet.key");

        let cert = std::fs::read(cert_file).map_err(|_| ClientError::CertificateError)?;
        let key = std::fs::read(key_file).map_err(|_| ClientError::CertificateError)?;

        let identity =
            Identity::from_pem(&[cert, key].concat()).map_err(|_| ClientError::CertificateError)?;

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .identity(identity)
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|_| ClientError::ClientBuildError)?;

        Ok(Self {
            client,
            base_url: "https://localhost:9257".to_string(),
        })
    }

    pub async fn send_cat(
        &self,
        asset_id: String,
        address: String,
        amount: u64,
        fee: u64,
        include_hint: bool,
        memos: Option<Vec<String>>,
        auto_submit: bool,
    ) -> Result<SendCatResponse, ClientError> {
        let url = format!("{}/send_cat", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&SendCat {
                asset_id,
                address,
                amount: Amount::u64(amount),
                fee: Amount::u64(fee),
                include_hint,
                memos,
                auto_submit,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "Status: {}, Body: {:?}",
                response.status(),
                response.text().await?
            )));
        }

        let response_body = response.json::<SendCatResponse>().await?;
        Ok(response_body)
    }

    pub async fn get_derivations(
        &self,
        hardened: bool,
        offset: u32,
        limit: u32,
    ) -> Result<GetDerivationsResponse, ClientError> {
        let url = format!("{}/get_derivations", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&GetDerivations {
                hardened,
                offset,
                limit,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "Status: {}, Body: {:?}",
                response.status(),
                response.text().await?
            )));
        }

        let response_body = response.json::<GetDerivationsResponse>().await?;
        Ok(response_body)
    }

    pub async fn send_xch(
        &self,
        address: String,
        amount: u64,
        fee: u64,
        memos: Option<Vec<String>>,
        auto_submit: bool,
    ) -> Result<SendXchResponse, ClientError> {
        let url = format!("{}/send_xch", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&SendXch {
                address,
                amount: Amount::u64(amount),
                fee: Amount::u64(fee),
                memos,
                auto_submit,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "Status: {}, Body: {:?}",
                response.status(),
                response.text().await?
            )));
        }

        let response_body = response.json::<SendCatResponse>().await?;
        Ok(response_body)
    }

    pub async fn sign_coin_spends(
        &self,
        coin_spends: Vec<CoinSpend>,
        auto_submit: bool,
        partial: bool,
    ) -> Result<SignCoinSpendsResponse, ClientError> {
        let url = format!("{}/sign_coin_spends", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&SignCoinSpends {
                coin_spends: coin_spends
                    .into_iter()
                    .map(|cs| CoinSpendJson {
                        coin: CoinJson {
                            parent_coin_info: format!(
                                "0x{}",
                                hex::encode(cs.coin.parent_coin_info)
                            ),
                            puzzle_hash: format!("0x{}", hex::encode(cs.coin.puzzle_hash)),
                            amount: Amount::u64(cs.coin.amount),
                        },
                        puzzle_reveal: format!("0x{:}", hex::encode(cs.puzzle_reveal.to_vec())),
                        solution: format!("0x{:}", hex::encode(cs.solution.to_vec())),
                    })
                    .collect(),
                auto_submit,
                partial,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "Status: {}, Body: {:?}",
                response.status(),
                response.text().await?
            )));
        }

        let response_body = response.json::<SignCoinSpendsResponse>().await?;
        Ok(response_body)
    }

    pub async fn make_offer(
        &self,
        requested_assets: Assets,
        offered_assets: Assets,
        fee: u64,
        receive_address: Option<String>,
        expires_at_second: Option<u64>,
        auto_import: bool,
    ) -> Result<MakeOfferResponse, ClientError> {
        let url = format!("{}/make_offer", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&MakeOffer {
                requested_assets,
                offered_assets,
                fee: Amount::u64(fee),
                receive_address,
                expires_at_second,
                auto_import,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ClientError::InvalidResponse(format!(
                "Status: {}, Body: {:?}",
                response.status(),
                response.text().await?
            )));
        }

        let response_body = response.json::<MakeOfferResponse>().await?;
        Ok(response_body)
    }
}

pub fn assets_xch_only(amount: u64) -> Assets {
    Assets {
        xch: Amount::u64(amount),
        cats: vec![],
        nfts: vec![],
    }
}

pub fn no_assets() -> Assets {
    assets_xch_only(0)
}

pub fn assets_xch_and_cat(xch_amount: u64, asset_id: String, cat_amount: u64) -> Assets {
    Assets {
        xch: Amount::u64(xch_amount),
        cats: vec![CatAmount {
            asset_id,
            amount: Amount::u64(cat_amount),
        }],
        nfts: vec![],
    }
}

pub fn assets_xch_and_nft(xch_amount: u64, nft_id: String) -> Assets {
    Assets {
        xch: Amount::u64(xch_amount),
        cats: vec![],
        nfts: vec![nft_id],
    }
}

use super::{ClientError, SageClient};
use chia::{
    bls::{self, PublicKey, SecretKey, Signature},
    consensus::consensus_constants::ConsensusConstants,
    protocol::{Bytes, Bytes32, Coin, CoinSpend, Program},
};
use chia_wallet_sdk::{
    coinset::{ChiaRpcClient, CoinsetClient},
    driver::{DriverError, Spend, SpendContext},
    types::{MAINNET_CONSTANTS, TESTNET11_CONSTANTS},
    utils::AddressError,
};
use hex::FromHex;
use std::{
    io::{self, Write},
    num::ParseIntError,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("csv: {0}")]
    Csv(#[from] csv::Error),

    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("driver: {0}")]
    Driver(#[from] DriverError),

    #[error("couldn't parse int: {0}")]
    ParseInt(#[from] ParseIntError),

    #[error("bech32: {0}")]
    Bech32(#[from] bech32::Error),

    #[error("address: {0}")]
    Address(#[from] AddressError),

    #[error("that's not a clear 'yes'")]
    YesNoPromptRejected,

    #[error("couldn't parse hex: {0}")]
    ParseHex(#[from] hex::FromHexError),

    #[error("invalid public key (or other BLS object): {0}")]
    InvalidPublicKey(#[from] bls::Error),

    #[error("invalid amount: must contain '.'")]
    InvalidAmount,

    #[error("home directory not found")]
    HomeDirectoryNotFound,

    #[error("client error: {0}")]
    Client(#[from] ClientError),

    #[error("custom error: {0}")]
    Custom(String),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Data directory could not be found")]
    DataDirNotFound,

    #[error("could not parse db column")]
    DbColumnParse(),

    #[error("could not find db column")]
    DbColumnNotFound(),

    #[error("coin not found: {0}")]
    CoinNotFound(Bytes32),

    #[error("puzzle hash records not found: {0}")]
    PuzzleHashRecordsNotFound(Bytes32),

    #[error("coin not spent: {0}")]
    CoinNotSpent(Bytes32),

    #[error("constants not set (launcher id or price singletong launcher id)")]
    ConstantsNotSet,

    #[error("{0} slot not found")]
    SlotNotFound(&'static str),
}

pub fn yes_no_prompt(prompt: &str) -> Result<(), CliError> {
    let mut input = String::new();
    print!("{} (yes/no): ", prompt);
    io::stdout().flush().map_err(CliError::Io)?;

    io::stdin().read_line(&mut input).map_err(CliError::Io)?;
    let input = input.trim().to_lowercase();

    if input != "yes" {
        return Err(CliError::YesNoPromptRejected);
    }

    Ok(())
}

pub fn prompt_for_value(prompt: &str) -> Result<String, CliError> {
    let mut input = String::new();
    print!("{} ", prompt);
    io::stdout().flush().map_err(CliError::Io)?;

    io::stdin().read_line(&mut input).map_err(CliError::Io)?;
    let input = input.trim().to_lowercase();

    Ok(input)
}

pub fn parse_amount(amount: &str, is_cat: bool) -> Result<u64, CliError> {
    if !amount.contains(".") {
        eprintln!("Amount must contain '.' to make sure you aren't providing mojos :)");
        return Err(CliError::InvalidAmount);
    }

    let Some((whole, fractional)) = amount.split_once('.') else {
        return Err(CliError::InvalidAmount);
    };

    let whole = whole.parse::<u64>().map_err(|_| CliError::InvalidAmount)?;
    let fractional = if is_cat {
        format!("{:0<3}", fractional)
    } else {
        format!("{:0<12}", fractional)
    }
    .parse::<u64>()
    .map_err(|_| CliError::InvalidAmount)?;

    if is_cat {
        // For CATs: 1 CAT = 1000 mojos
        Ok(whole * 1000 + fractional)
    } else {
        // For XCH: 1 XCH = 1_000_000_000_000 mojos
        Ok(whole * 1_000_000_000_000 + fractional)
    }
}

pub fn get_prefix(testnet11: bool) -> String {
    if testnet11 {
        "txch".to_string()
    } else {
        "xch".to_string()
    }
}

pub fn get_constants(testnet11: bool) -> &'static ConsensusConstants {
    if testnet11 {
        &TESTNET11_CONSTANTS
    } else {
        &MAINNET_CONSTANTS
    }
}

pub fn get_coinset_client(testnet11: bool) -> CoinsetClient {
    if testnet11 {
        CoinsetClient::testnet11()
    } else {
        CoinsetClient::mainnet()
    }
}

pub fn hex_string_to_bytes32(hex: &str) -> Result<Bytes32, CliError> {
    let bytes = <[u8; 32]>::from_hex(hex.replace("0x", "")).map_err(CliError::ParseHex)?;
    Ok(Bytes32::from(bytes))
}

pub fn hex_string_to_pubkey(hex: &str) -> Result<PublicKey, CliError> {
    let bytes = <[u8; 48]>::from_hex(hex.replace("0x", "")).map_err(CliError::ParseHex)?;
    PublicKey::from_bytes(&bytes).map_err(CliError::InvalidPublicKey)
}

pub fn hex_string_to_secret_key(hex: &str) -> Result<SecretKey, CliError> {
    let bytes = <[u8; 32]>::from_hex(hex.replace("0x", "")).map_err(CliError::ParseHex)?;
    SecretKey::from_bytes(&bytes).map_err(CliError::InvalidPublicKey)
}

pub fn hex_string_to_signature(hex: &str) -> Result<Signature, CliError> {
    let bytes = <[u8; 96]>::from_hex(hex.replace("0x", "")).map_err(CliError::ParseHex)?;
    Signature::from_bytes(&bytes).map_err(CliError::InvalidPublicKey)
}

pub fn hex_string_to_bytes(hex: &str) -> Result<Bytes, CliError> {
    let bytes = hex::decode(hex.replace("0x", "")).map_err(CliError::ParseHex)?;
    Ok(Bytes::from(bytes))
}

#[allow(clippy::nonminimal_bool)]
pub async fn wait_for_coin(
    client: &CoinsetClient,
    coin_id: Bytes32,
    also_wait_for_spent: bool,
) -> Result<(), CliError> {
    println!("Waiting for coin...");
    loop {
        let record = client.get_coin_record_by_name(coin_id).await?;
        if let Some(record) = record.coin_record {
            if !also_wait_for_spent || (also_wait_for_spent && record.spent) {
                break;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    Ok(())
}

pub async fn get_coin_public_key(
    client: &SageClient,
    address: &String,
    address_limit: u32,
) -> Result<PublicKey, CliError> {
    let addresses_per_request: u32 = 500;
    let mut offset = 0;
    while offset < address_limit {
        let resp = client
            .get_derivations(false, offset, addresses_per_request)
            .await?;
        for derivation in resp.derivations {
            if derivation.address.eq(address) {
                return hex_string_to_pubkey(&derivation.public_key);
            }
        }
        offset += addresses_per_request;
    }

    Err(CliError::Custom(
        "Could not find public key associated with provided puzzle hash".to_string(),
    ))
}

pub async fn get_last_onchain_timestamp(client: &CoinsetClient) -> Result<u64, CliError> {
    println!("Fetching latest transaction block timestamp...");
    let blockchain_state = client
        .get_blockchain_state()
        .await?
        .blockchain_state
        .ok_or(CliError::Custom(
            "Could not fetch blockchain state".to_string(),
        ))?;

    if let Some(t) = blockchain_state.peak.timestamp {
        Ok(t)
    } else {
        let mut height = blockchain_state.peak.height - 1;
        let mut block;
        loop {
            println!("Fetching block record #{}...", height);
            block = client
                .get_block_record_by_height(height)
                .await?
                .block_record
                .ok_or(CliError::Custom(format!(
                    "Could not fetch block record #{}",
                    height
                )))?;

            if block.timestamp.is_some() {
                break;
            }
            height -= 1;
        }

        Ok(block.timestamp.unwrap())
    }
}

pub fn spend_to_coin_spend(
    ctx: &mut SpendContext,
    coin: Coin,
    spend: Spend,
) -> Result<CoinSpend, CliError> {
    Ok(CoinSpend {
        coin,
        puzzle_reveal: Program::new(ctx.serialize(&spend.puzzle)?.to_vec().into()),
        solution: Program::new(ctx.serialize(&spend.solution)?.to_vec().into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_amount() -> anyhow::Result<()> {
        assert_eq!(parse_amount("1.01", true)?, 1010);
        assert_eq!(parse_amount("1.01", false)?, 1_010_000_000_000);

        Ok(())
    }
}

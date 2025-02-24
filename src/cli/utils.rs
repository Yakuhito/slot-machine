use chia::{
    bls::{self, SecretKey},
    protocol::{Bytes32, Coin, CoinSpend, Program},
    puzzles::standard::StandardArgs,
    traits::Streamable,
};
use chia_wallet_sdk::{encode_address, DriverError, SpendContext};
use hex::FromHex;
use sage_api::{Amount, CoinSpendJson, SendXch};
use std::{
    io::{self, Write},
    num::ParseIntError,
};
use thiserror::Error;

use crate::new_sk;

use super::{ClientError, SageClient};

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

    #[error("that's not a clear 'yes'")]
    YesNoPromptRejected,

    #[error("couldn't parse hex: {0}")]
    ParseHex(#[from] hex::FromHexError),

    #[error("invalid public key")]
    InvalidPublicKey(#[from] bls::Error),

    #[error("invalid amount: must contain '.'")]
    InvalidAmount,

    #[error("home directory not found")]
    HomeDirectoryNotFound,

    #[error("client error: {0}")]
    ClientError(#[from] ClientError),

    #[error("custom error: {0}")]
    Custom(String),
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

pub fn parse_amount(amount: String, is_cat: bool) -> Result<u64, CliError> {
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

pub fn hex_string_to_bytes32(hex: &str) -> Result<Bytes32, CliError> {
    let bytes = <[u8; 32]>::from_hex(hex.replace("0x", "")).map_err(CliError::ParseHex)?;
    Ok(Bytes32::from(bytes))
}

pub fn json_to_coin_spend(json: CoinSpendJson) -> Result<CoinSpend, CliError> {
    let coin = Coin::new(
        hex_string_to_bytes32(&json.coin.parent_coin_info)?,
        hex_string_to_bytes32(&json.coin.puzzle_hash)?,
        json.coin.amount.to_u64().ok_or(CliError::Custom(
            "response coin amount is too large".to_string(),
        ))?,
    );

    let puzzle_reveal = hex_string_to_bytes32(&json.puzzle_reveal)?;
    let solution = hex_string_to_bytes32(&json.solution)?;

    Ok(CoinSpend::new(
        coin,
        Program::from_bytes(&puzzle_reveal).map_err(|_| {
            CliError::Custom("could not load puzzle reveal string to program".to_string())
        })?,
        Program::from_bytes(&solution).map_err(|_| {
            CliError::Custom("could not load solution string to program".to_string())
        })?,
    ))
}

pub async fn get_xch_coin(
    client: &SageClient,
    ctx: &mut SpendContext,
    amount: u64,
    fee: u64,
    testnet11: bool,
) -> Result<(SecretKey, Coin), CliError> {
    println!("Getting source XCH coin...");

    let sk = new_sk()?;

    let target_puzzle_hash = StandardArgs::curry_tree_hash(sk.public_key());

    let target_address = encode_address(target_puzzle_hash.into(), &get_prefix(testnet11))
        .map_err(CliError::Bech32)?;
    let response = client
        .send_xch(SendXch {
            address: target_address.clone(),
            amount: Amount::Number(2),
            fee: Amount::Number(fee),
            memos: vec![],
            auto_submit: false,
        })
        .await?;

    response
        .coin_spends
        .into_iter()
        .map(|json| {
            ctx.insert(json_to_coin_spend(json)?);
            Ok::<(), CliError>(())
        })
        .collect::<Result<Vec<()>, _>>()?;

    let mut new_coin: Option<Coin> = None;

    for input in response.summary.inputs {
        for output in input.outputs {
            if output.address == target_address && output.amount == Amount::Number(amount) {
                new_coin = Some(Coin::new(
                    hex_string_to_bytes32(&input.coin_id)?,
                    target_puzzle_hash.into(),
                    amount,
                ));
                break;
            }
        }

        if new_coin.is_some() {
            break;
        }
    }

    let Some(new_coin) = new_coin else {
        return Err(CliError::Custom(
            "could not identify new coin in Sage RPC response".to_string(),
        ));
    };
    Ok((sk, new_coin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_amount() -> anyhow::Result<()> {
        assert_eq!(parse_amount("1.01".to_string(), true)?, 1010);
        assert_eq!(parse_amount("1.01".to_string(), false)?, 1_010_000_000_000);

        Ok(())
    }
}

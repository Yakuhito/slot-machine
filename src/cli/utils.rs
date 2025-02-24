use chia::bls;
use chia_wallet_sdk::DriverError;
use std::{
    io::{self, Write},
    num::ParseIntError,
};
use thiserror::Error;

use super::ClientError;

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

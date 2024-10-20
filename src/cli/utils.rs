use chia_wallet_sdk::{DriverError, OfferError};
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

    #[error("offer: {0}")]
    Offer(#[from] OfferError),

    #[error("driver: {0}")]
    Driver(#[from] DriverError),

    #[error("couldn't parse int: {0}")]
    ParseInt(#[from] ParseIntError),

    #[error("bech32: {0}")]
    Bech32(#[from] bech32::Error),

    #[error("that's not a clear 'yes'")]
    YesNoPromptRejected,
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

pub mod hex_string_to_bytes32 {
    use chia::protocol::Bytes32;
    use hex::FromHex;
    use serde::{self, Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Bytes32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(deserializer)?;
        let bytes = <[u8; 32]>::from_hex(s.replace("0x", "")).map_err(serde::de::Error::custom)?;
        Ok(Bytes32::new(bytes))
    }
}

pub mod hex_string_to_bytes {
    use chia::protocol::Bytes;
    use hex::FromHex;
    use serde::{self, Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(deserializer)?;
        let bytes = Vec::<u8>::from_hex(s.replace("0x", "")).map_err(serde::de::Error::custom)?;
        Ok(Bytes::new(bytes))
    }
}

pub mod hex_string_to_bytes_maybe {
    use chia::protocol::Bytes;
    use hex::FromHex;
    use serde::{self, Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Bytes>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if let Ok(s) = String::deserialize(deserializer) {
            if s.is_empty() {
                return Ok(None);
            }

            let bytes =
                Vec::<u8>::from_hex(s.replace("0x", "")).map_err(serde::de::Error::custom)?;
            return Ok(Some(Bytes::new(bytes)));
        }

        Ok(None)
    }
}

pub mod hex_string_to_bytes32_maybe {
    use chia::protocol::Bytes32;
    use hex::FromHex;
    use serde::{self, Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Bytes32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if let Ok(s) = String::deserialize(deserializer) {
            if s.len() != 64 && s.len() != 66 {
                return Ok(None);
            }

            let bytes =
                <[u8; 32]>::from_hex(s.replace("0x", "")).map_err(serde::de::Error::custom)?;
            return Ok(Some(Bytes32::new(bytes)));
        }

        Ok(None)
    }
}

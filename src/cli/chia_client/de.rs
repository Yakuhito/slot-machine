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

pub mod deserializable_coin {
    use chia::protocol::Coin;
    use serde::{Deserialize, Deserializer};

    use crate::DeserializableCoin;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Coin, D::Error>
    where
        D: Deserializer<'de>,
    {
        let coin = DeserializableCoin::deserialize(deserializer)?;
        Ok(Coin::new(
            coin.parent_coin_info,
            coin.puzzle_hash,
            coin.amount,
        ))
    }
}

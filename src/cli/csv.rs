use chia::protocol::Bytes32;
use csv::ReaderBuilder;
use hex::FromHex;
use serde::Deserialize;
use std::error::Error;
use std::fs::File;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct CatalogPremineRecord {
    #[serde(with = "hex_string")]
    asset_id: Bytes32,
    #[serde(deserialize_with = "decode_bech32m")]
    owner: Bytes32,
    code: String,
    name: String,
    precision: u8,
    #[serde(deserialize_with = "deserialize_string_array")]
    image_uris: Vec<String>,
    #[serde(with = "hex_string")]
    image_hash: Bytes32,
}

mod hex_string {
    use super::*;
    use serde::{self, Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Bytes32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(deserializer)?;
        let bytes = <[u8; 32]>::from_hex(s).map_err(serde::de::Error::custom)?;
        Ok(Bytes32::new(bytes))
    }
}

fn decode_bech32m<'de, D>(deserializer: D) -> Result<Bytes32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    let (hrp, data, _) = bech32::decode(s).map_err(serde::de::Error::custom)?;

    if hrp != "xch" && hrp != "txch" {
        return Err(serde::de::Error::custom("Invalid Bech32m prefix"));
    }

    let bytes = Vec::from_base32(&data).map_err(serde::de::Error::custom)?;
    let mut result = [0u8; 32];
    if bytes.len() != 32 {
        return Err(serde::de::Error::custom(
            "Decoded Bech32m does not match expected length of 32 bytes",
        ));
    }
    result.copy_from_slice(&bytes[..32]);
    Ok(Bytes32(result))
}

fn deserialize_string_array<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    let s = s.trim_matches(&['[', ']'][..]); // trim square brackets
    let strs: Vec<String> = s
        .split(',')
        .map(|s| s.trim().trim_matches(&['\'', '"'][..]).to_string())
        .collect();
    Ok(strs)
}

pub fn load_catalog<P: AsRef<Path>>(path: P) -> Result<Vec<CatalogPremineRecord>, Box<dyn Error>> {
    let file = File::open(path)?;
    let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut records = Vec::new();
    for result in rdr.deserialize() {
        let record: CatalogPremineRecord = result?;
        records.push(record);
    }

    Ok(records)
}

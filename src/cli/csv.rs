use chia::bls::PublicKey;
use chia::protocol::Bytes32;
use chia_wallet_sdk::decode_address;
use csv::ReaderBuilder;
use hex::FromHex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use super::utils::CliError;

#[derive(Debug, Deserialize)]
pub struct CatalogPremineRecord {
    #[serde(deserialize_with = "hex_string_to_bytes32")]
    pub asset_id: Bytes32,
    #[serde(deserialize_with = "decode_bech32m")]
    pub owner: Bytes32,
    pub code: String,
    pub name: String,
    pub precision: u8,
    #[serde(deserialize_with = "deserialize_string_array")]
    pub image_uris: Vec<String>,
    #[serde(deserialize_with = "hex_string_to_bytes32")]
    pub image_hash: Bytes32,
}

fn decode_bech32m<'de, D>(deserializer: D) -> Result<Bytes32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    let (res, hrp) = decode_address(s).map_err(serde::de::Error::custom)?;

    if hrp != "xch" && hrp != "txch" {
        return Err(serde::de::Error::custom("Invalid bech32m prefix"));
    }

    Ok(Bytes32::new(res))
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

fn hex_string_to_bytes32<'de, D>(deserializer: D) -> Result<Bytes32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    let bytes = <[u8; 32]>::from_hex(s.replace("0x", "")).map_err(serde::de::Error::custom)?;
    Ok(Bytes32::new(bytes))
}

pub fn load_catalog_premine_csv<P: AsRef<Path>>(
    path: P,
) -> Result<Vec<CatalogPremineRecord>, CliError> {
    let file = File::open(path)?;
    let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut records = Vec::new();
    for result in rdr.deserialize() {
        let record: CatalogPremineRecord = result.map_err(CliError::Csv)?;
        records.push(record);
    }

    Ok(records)
}

#[derive(Debug, Deserialize)]
pub struct AliasRecord {
    #[serde(deserialize_with = "hex_string_to_pubkey")]
    pub pubkey: PublicKey,
    pub alias: String,
}

fn hex_string_to_pubkey<'de, D>(deserializer: D) -> Result<PublicKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    let pubkey = PublicKey::from_bytes(
        &<[u8; 48]>::from_hex(s.replace("0x", "")).map_err(serde::de::Error::custom)?,
    )
    .map_err(serde::de::Error::custom)?;
    Ok(pubkey)
}

const ALIASES_CSV_PATH: &str = "aliases.csv";

pub fn get_alias_map() -> Result<HashMap<PublicKey, String>, CliError> {
    let file = File::open(ALIASES_CSV_PATH)?;
    let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut alias_map = HashMap::new();
    for result in rdr.deserialize() {
        let record: AliasRecord = result.map_err(CliError::Csv)?;
        alias_map.insert(record.pubkey, record.alias);
    }

    Ok(alias_map)
}

#[derive(Debug, Deserialize)]
pub struct CatalogStateScheduleRecord {
    pub block_height: u64,
    #[serde(deserialize_with = "hex_string_to_bytes32")]
    pub asset_id: Bytes32,
    pub registration_price: u64,
}

pub fn load_catalog_state_schedule_csv<P: AsRef<Path>>(
    path: P,
) -> Result<Vec<CatalogStateScheduleRecord>, CliError> {
    let file = File::open(path)?;
    let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut records = Vec::new();
    for result in rdr.deserialize() {
        let record: CatalogStateScheduleRecord = result.map_err(CliError::Csv)?;
        records.push(record);
    }

    Ok(records)
}

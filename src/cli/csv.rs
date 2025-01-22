// use chia::protocol::Bytes32;
// use chia_wallet_sdk::decode_address;
// use csv::ReaderBuilder;
// use serde::Deserialize;
// use std::fs::File;
// use std::path::Path;

// use super::chia_client::de::hex_string_to_bytes32;
// use super::utils::CliError;

// #[derive(Debug, Deserialize)]
// pub struct CatalogPremineRecord {
//     #[serde(with = "hex_string_to_bytes32")]
//     pub asset_id: Bytes32,
//     #[serde(deserialize_with = "decode_bech32m")]
//     pub owner: Bytes32,
//     pub code: String,
//     pub name: String,
//     pub precision: u8,
//     #[serde(deserialize_with = "deserialize_string_array")]
//     pub image_uris: Vec<String>,
//     #[serde(with = "hex_string_to_bytes32")]
//     pub image_hash: Bytes32,
// }

// fn decode_bech32m<'de, D>(deserializer: D) -> Result<Bytes32, D::Error>
// where
//     D: serde::Deserializer<'de>,
// {
//     let s: &str = Deserialize::deserialize(deserializer)?;
//     let (res, hrp) = decode_address(s).map_err(serde::de::Error::custom)?;

//     if hrp != "xch" && hrp != "txch" {
//         return Err(serde::de::Error::custom("Invalid bech32m prefix"));
//     }

//     Ok(Bytes32::new(res))
// }

// fn deserialize_string_array<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
// where
//     D: serde::Deserializer<'de>,
// {
//     let s: &str = Deserialize::deserialize(deserializer)?;
//     let s = s.trim_matches(&['[', ']'][..]); // trim square brackets
//     let strs: Vec<String> = s
//         .split(',')
//         .map(|s| s.trim().trim_matches(&['\'', '"'][..]).to_string())
//         .collect();
//     Ok(strs)
// }

// pub fn load_catalog_premine_csv<P: AsRef<Path>>(
//     path: P,
// ) -> Result<Vec<CatalogPremineRecord>, CliError> {
//     let file = File::open(path)?;
//     let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(file);

//     let mut records = Vec::new();
//     for result in rdr.deserialize() {
//         let record: CatalogPremineRecord = result.map_err(CliError::Csv)?;
//         records.push(record);
//     }

//     Ok(records)
// }

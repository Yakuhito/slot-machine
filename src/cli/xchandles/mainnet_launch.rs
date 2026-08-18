//! Local typed mainnet launch configuration for XCHandles.
//!
//! This is ordinary application/launch configuration owned by the launch tool.
//! It is not a canonical deployment manifest, discovery endpoint, or wallet-SDK
//! deployment surface.

use chia_bls::PublicKey;
use chia_protocol::Bytes32;
use chia_wallet_sdk::utils::Address;
use hex_literal::hex;

use crate::{hex_string_to_pubkey, CliError, XchandlesStateScheduleRecord};

/// Mainnet wUSDC.b asset ID.
pub const WUSDC_B_ASSET_ID: Bytes32 = Bytes32::new(hex!(
    "fa4a180ac326e67ea289b869e3448256f6af05721f7cf934cb9901baa6b7a99d"
));

/// One registration period in seconds.
pub const REGISTRATION_PERIOD: u64 = 31_557_600;

/// Launch Instant: 2026-08-20 09:00:00 UTC.
pub const LAUNCH_INSTANT: u64 = 1_787_216_400;

/// XCHandles-minted royalty bech32m address.
pub const ROYALTY_ADDRESS: &str = "xch1xmdgcuuqzxla65w52uuh2s7g7ug29pvc7m97esqej556xkwwhjqssy7fzl";

/// XCHandles-minted royalty puzzle hash.
pub const ROYALTY_PUZZLE_HASH: Bytes32 = Bytes32::new(hex!(
    "36da8c738011bfdd51d457397543c8f710a28598f6cbecc0199529a359cebc81"
));

/// Royalty basis points for XCHandles-minted Handle NFTs.
pub const ROYALTY_BASIS_POINTS: u16 = 500;

/// Production media origin for generator-v1 URLs.
pub const MEDIA_ORIGIN: &str = "https://nfts.xchandles.com";

/// Initial license URI (CC0 1.0 Universal legal text).
pub const LICENSE_URI: &str = "https://creativecommons.org/publicdomain/zero/1.0/legalcode.txt";

/// Contribution Premine Expiration: 2027-08-20 09:00:00 UTC.
pub const CONTRIBUTION_PREMINE_EXPIRATION: u64 = 1_818_752_400;

/// Deterministic launch batch size baked into the Premine Launch Bundle.
pub const LAUNCH_HANDLES_PER_BATCH: usize = 25;

/// Post-schedule Price Singleton threshold (m of n).
pub const PRICE_SINGLETON_M: usize = 6;

/// Ordered current validator public keys for the post-schedule 6-of-10 controller.
/// Former validator 10 is intentionally excluded.
pub const PRICE_SINGLETON_PUBLIC_KEYS: [&str; 10] = [
    "b63871fbc72a7ff07d8f2419c8d3bbe1ac557d3cbf367761d08a1a1209dd285358124845151381da912751e33bd7ffa8",
    "8d5ca1a64a587c2fe7603a6933d335ad01a50f0187085981651d617ffaffce9d57ad25680813a030665fabef12075811",
    "aa5ea815c1c0e70882b532bb7462a2d8ba68817a4ccfb728214382217b671fd534e19b869ad404ecd5c7852520c6f0c0",
    "b2ee00a2e657ee8bb818999f56ec781c4b10a04919eb061904e44bc96f022e47a75454b1d63796ea6eff8fcf2932d8ca",
    "9322d4a1f8d078b81ea674947ba2420f4175f38483d7ac60dc3ff4de3d27cf33bb1c06cb7638467536ef766533a7ad79",
    "a7b9970795c085979ca94b54e5c1b8e4ee96104dac690c01175b138b327c85e2537e53dc97189ecca57b629d3283bdca",
    "813dcc8c4870df68416f14253a559aa0e088db84a46b9fe3e442eb6dbc2a1cb5381e2e862927a2743a1903889aadad1b",
    "b5bd04adb90273d97a458b5e42d4930ab35643203131c22d53ac312026da74fda64c71216fc6db263063262266c45727",
    "84b00e171a571b5904e48cdf456bf2861d37f01961c8b641a66974c70f49393fc55ba4b05543ca2894e8f6c7daad0719",
    "836324ba44d7e1f2290a1ccf4c3c2d064c5047619981198ec4059165d85c06a1fd214da2b1b58603254f9b48637a9db1",
];

/// Exact mainnet launch pricing schedule: `(timestamp, factor, base_price_atomic)`.
pub const PRICE_SCHEDULE: [(u64, u64, u64); 15] = [
    (1_787_216_400, 1000, 5_000_000),
    (1_787_302_800, 750, 3_750_000),
    (1_787_389_200, 500, 2_500_000),
    (1_787_475_600, 250, 1_250_000),
    (1_787_562_000, 100, 500_000),
    (1_787_648_400, 75, 375_000),
    (1_787_734_800, 50, 250_000),
    (1_787_821_200, 33, 165_000),
    (1_787_907_600, 25, 125_000),
    (1_787_994_000, 15, 75_000),
    (1_788_080_400, 10, 50_000),
    (1_788_166_800, 5, 25_000),
    (1_788_253_200, 3, 15_000),
    (1_788_339_600, 2, 10_000),
    (1_788_426_000, 1, 5_000),
];

/// Base atomic price at factor 1 (final schedule row).
pub const BASE_PRICE_AT_FACTOR_ONE: u64 = 5_000;

pub fn royalty_puzzle_hash_from_address() -> Result<Bytes32, CliError> {
    let address = Address::decode(ROYALTY_ADDRESS).map_err(|e| CliError::Custom(e.to_string()))?;
    Ok(address.puzzle_hash)
}

pub fn price_singleton_public_keys() -> Result<Vec<PublicKey>, CliError> {
    PRICE_SINGLETON_PUBLIC_KEYS
        .iter()
        .map(|hex_key| hex_string_to_pubkey(hex_key))
        .collect()
}

pub fn mainnet_price_schedule_records() -> Vec<XchandlesStateScheduleRecord> {
    PRICE_SCHEDULE
        .iter()
        .map(
            |(timestamp, _factor, registration_price)| XchandlesStateScheduleRecord {
                timestamp: *timestamp,
                asset_id: WUSDC_B_ASSET_ID,
                registration_price: *registration_price,
                registration_period: REGISTRATION_PERIOD,
            },
        )
        .collect()
}

pub fn controller_matches_configured(m: usize, pubkeys: &[PublicKey]) -> Result<bool, CliError> {
    let configured = price_singleton_public_keys()?;
    Ok(m == PRICE_SINGLETON_M && pubkeys == configured.as_slice())
}

pub fn schedule_records_match_configured(records: &[XchandlesStateScheduleRecord]) -> bool {
    records == mainnet_price_schedule_records().as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_schedule_has_exact_fifteen_entries() {
        assert_eq!(PRICE_SCHEDULE.len(), 15);
        assert_eq!(mainnet_price_schedule_records().len(), 15);
    }

    #[test]
    fn mainnet_schedule_entry_by_entry() {
        let expected = [
            (1_787_216_400_u64, 1000_u64, 5_000_000_u64),
            (1_787_302_800, 750, 3_750_000),
            (1_787_389_200, 500, 2_500_000),
            (1_787_475_600, 250, 1_250_000),
            (1_787_562_000, 100, 500_000),
            (1_787_648_400, 75, 375_000),
            (1_787_734_800, 50, 250_000),
            (1_787_821_200, 33, 165_000),
            (1_787_907_600, 25, 125_000),
            (1_787_994_000, 15, 75_000),
            (1_788_080_400, 10, 50_000),
            (1_788_166_800, 5, 25_000),
            (1_788_253_200, 3, 15_000),
            (1_788_339_600, 2, 10_000),
            (1_788_426_000, 1, 5_000),
        ];

        for (i, (timestamp, factor, price)) in expected.iter().enumerate() {
            assert_eq!(PRICE_SCHEDULE[i].0, *timestamp, "timestamp at index {i}");
            assert_eq!(PRICE_SCHEDULE[i].1, *factor, "factor at index {i}");
            assert_eq!(PRICE_SCHEDULE[i].2, *price, "price at index {i}");
            assert_eq!(
                *price,
                BASE_PRICE_AT_FACTOR_ONE * *factor,
                "price must equal 5000 * factor at index {i}"
            );
            assert_eq!(PRICE_SCHEDULE[i].0, LAUNCH_INSTANT + (i as u64) * 86_400);
        }
    }

    #[test]
    fn mainnet_asset_period_launch_royalty_constants_are_exact() {
        assert_eq!(
            hex::encode(WUSDC_B_ASSET_ID),
            "fa4a180ac326e67ea289b869e3448256f6af05721f7cf934cb9901baa6b7a99d"
        );
        assert_eq!(REGISTRATION_PERIOD, 31_557_600);
        assert_eq!(LAUNCH_INSTANT, 1_787_216_400);
        assert_eq!(ROYALTY_BASIS_POINTS, 500);
        assert_eq!(
            hex::encode(ROYALTY_PUZZLE_HASH),
            "36da8c738011bfdd51d457397543c8f710a28598f6cbecc0199529a359cebc81"
        );
        assert_eq!(
            royalty_puzzle_hash_from_address().unwrap(),
            ROYALTY_PUZZLE_HASH
        );
        assert_eq!(
            ROYALTY_ADDRESS,
            "xch1xmdgcuuqzxla65w52uuh2s7g7ug29pvc7m97esqej556xkwwhjqssy7fzl"
        );
    }

    #[test]
    fn post_schedule_controller_is_ordered_six_of_ten() {
        let keys = price_singleton_public_keys().unwrap();
        assert_eq!(PRICE_SINGLETON_M, 6);
        assert_eq!(keys.len(), 10);
        assert_eq!(PRICE_SINGLETON_PUBLIC_KEYS.len(), 10);
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(hex::encode(key.to_bytes()), PRICE_SINGLETON_PUBLIC_KEYS[i]);
        }
        assert!(controller_matches_configured(6, &keys).unwrap());
        assert!(!controller_matches_configured(5, &keys).unwrap());
        let mut reordered = keys.clone();
        reordered.swap(0, 1);
        assert!(!controller_matches_configured(6, &reordered).unwrap());
    }

    #[test]
    fn validator_ten_is_not_included() {
        // Former validator 10 is absent from the configured ordered set.
        assert!(!PRICE_SINGLETON_PUBLIC_KEYS.iter().any(|k| k.is_empty()));
        assert_eq!(PRICE_SINGLETON_PUBLIC_KEYS.len(), 10);
        let unique: std::collections::BTreeSet<_> =
            PRICE_SINGLETON_PUBLIC_KEYS.iter().copied().collect();
        assert_eq!(unique.len(), 10);
    }

    #[test]
    fn mainnet_csv_matches_typed_schedule() {
        let records =
            crate::load_xchandles_state_schedule_csv("xchandles_price_schedule_mainnet.csv")
                .expect("mainnet schedule csv must load");
        assert!(schedule_records_match_configured(&records));
    }

    #[test]
    fn schedule_prices_compose_with_factor_pricing_artifacts() {
        use chia_wallet_sdk::types::puzzles::XchandlesFactorPricingPuzzleArgs;

        for (timestamp, factor, base_price) in PRICE_SCHEDULE {
            assert_eq!(base_price, BASE_PRICE_AT_FACTOR_ONE * factor);
            // Length-3 handles cost base_price * 128 for one period.
            assert_eq!(
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, "abc", 1),
                base_price * 128
            );
            // Grammar accepts 3-63; length ≥6 (without digits) uses factor 2.
            let long_handle = "a".repeat(63);
            assert_eq!(
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &long_handle, 1),
                base_price * 2
            );
            assert_eq!(timestamp % 86_400, LAUNCH_INSTANT % 86_400);
        }
    }
}

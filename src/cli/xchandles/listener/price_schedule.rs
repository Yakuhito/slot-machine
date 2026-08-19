//! Launch Price Singleton generation list and last-due / remaining-unroll helpers.

use chia_wallet_sdk::prelude::Mod;
use chia_wallet_sdk::types::puzzles::XchandlesFactorPricingPuzzleArgs;
use serde::{Deserialize, Serialize};

use crate::PRICE_SCHEDULE;

/// Base price before the first generation row (launch / premine).
pub const LAUNCH_BASE_PRICE: u64 = 1;

/// `(activation_timestamp, base_price)` from `xchandles_price_schedule_testnet11.csv`.
pub const TESTNET11_PRICE_SCHEDULE: [(u64, u64); 9] = [
    (1_786_885_200, 9),
    (1_786_892_400, 8),
    (1_786_924_800, 7),
    (1_786_935_600, 6),
    (1_786_953_600, 5),
    (1_786_971_600, 4),
    (1_786_978_800, 3),
    (1_787_011_200, 2),
    (1_787_022_000, 1),
];

/// One Price Singleton generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleGeneration {
    pub activation_timestamp: u64,
    pub base_price: u64,
}

/// Query for `GET /price` and `GET /schedule`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PriceQuery {
    pub launcher_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceResponse {
    pub indexed_peak_height: u32,
    pub confirmed_timestamp: u64,
    pub current_base_price: u64,
    pub unrolls: Vec<ScheduleGeneration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleResponse {
    pub generations: Vec<ScheduleGeneration>,
}

pub fn testnet11_generations() -> Vec<ScheduleGeneration> {
    TESTNET11_PRICE_SCHEDULE
        .iter()
        .map(|&(activation_timestamp, base_price)| ScheduleGeneration {
            activation_timestamp,
            base_price,
        })
        .collect()
}

pub fn mainnet_generations() -> Vec<ScheduleGeneration> {
    PRICE_SCHEDULE
        .iter()
        .map(|&(activation_timestamp, _factor, base_price)| ScheduleGeneration {
            activation_timestamp,
            base_price,
        })
        .collect()
}

pub fn generations_for_network(testnet11: bool) -> Vec<ScheduleGeneration> {
    if testnet11 {
        testnet11_generations()
    } else {
        mainnet_generations()
    }
}

/// Latest generation whose `activation_timestamp <= at`, else launch price 1.
pub fn effective_base_at(schedule: &[ScheduleGeneration], at: u64) -> u64 {
    let mut price = LAUNCH_BASE_PRICE;
    for row in schedule {
        if at >= row.activation_timestamp {
            price = row.base_price;
        }
    }
    price
}

/// Index of the first remaining unroll (next scheduler generation).
///
/// When committed base already matches the last-due generation, due rows are
/// treated as applied. When it lags (launch 1 while later rows are due),
/// remaining starts at generation 0 so those due rows stay executable.
pub fn remaining_unroll_start(
    schedule: &[ScheduleGeneration],
    committed_base: u64,
    confirmed_timestamp: u64,
) -> usize {
    let last_due = schedule
        .iter()
        .rposition(|row| row.activation_timestamp <= confirmed_timestamp);
    let last_due_price = last_due
        .map(|i| schedule[i].base_price)
        .unwrap_or(LAUNCH_BASE_PRICE);
    if committed_base == last_due_price {
        return last_due.map(|i| i + 1).unwrap_or(0);
    }
    if committed_base == LAUNCH_BASE_PRICE {
        return 0;
    }
    schedule
        .iter()
        .position(|row| row.base_price == committed_base)
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Match a registry pricing-puzzle hash against launch 1 and every schedule row.
pub fn committed_base_from_pricing_puzzle(
    pricing_puzzle_hash: chia_protocol::Bytes32,
    registration_period: u64,
    schedule: &[ScheduleGeneration],
) -> Option<u64> {
    let mut candidates = vec![LAUNCH_BASE_PRICE];
    for row in schedule {
        if !candidates.contains(&row.base_price) {
            candidates.push(row.base_price);
        }
    }
    for base_price in candidates {
        let expected = XchandlesFactorPricingPuzzleArgs {
            base_price,
            registration_period,
        }
        .curry_tree_hash();
        if expected == pricing_puzzle_hash.into() {
            return Some(base_price);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_base_is_one_before_first_row() {
        let schedule = testnet11_generations();
        assert_eq!(effective_base_at(&schedule, 0), 1);
        assert_eq!(effective_base_at(&schedule, 1_786_885_199), 1);
        assert_eq!(effective_base_at(&schedule, 1_786_885_200), 9);
        assert_eq!(effective_base_at(&schedule, 1_787_022_000), 1);
    }

    #[test]
    fn remaining_starts_after_applied_last_due() {
        let schedule = testnet11_generations();
        let confirmed = 1_786_935_600;
        assert_eq!(effective_base_at(&schedule, confirmed), 6);
        assert_eq!(remaining_unroll_start(&schedule, 6, confirmed), 4);
    }

    #[test]
    fn remaining_is_full_schedule_when_committed_lags_at_launch_one() {
        let schedule = testnet11_generations();
        let confirmed = 1_786_935_600;
        assert_eq!(remaining_unroll_start(&schedule, 1, confirmed), 0);
    }
}

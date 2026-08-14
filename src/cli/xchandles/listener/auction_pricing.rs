//! Protocol-faithful expiration-auction pricing for directory reads.
//!
//! Mirrors `exponential_premium.rue` / ENS ExponentialPremiumPriceOracle so the
//! listener can price pages without inventing heights or secrets.

use chia_wallet_sdk::types::puzzles::{
    XchandlesExponentialPremiumRenewPuzzleArgs, XchandlesFactorPricingPuzzleArgs,
    PREMIUM_BITS_LIST, PREMIUM_PRECISION,
};

/// One registration-period year used for directory base fees.
pub const DIRECTORY_REGISTRATION_PERIODS: u64 = 1;

/// Auction length in whole days; premium is zero at and after this elapsed time.
pub const AUCTION_DURATION_SECONDS: u64 = 28 * 86_400;

/// Committed registration / directory projection offset from confirmed chain time.
pub const PRICING_PROJECTION_OFFSET_SECONDS: u64 = 420;

/// Expiring-soon window upper bound (inclusive).
pub const SOON_WINDOW_SECONDS: u64 = 30 * 86_400;

const HALVING_PERIOD: u64 = 86_400;
const SCALE_FACTOR: u64 = 1000;

/// Unix timestamp when the auction premium reaches zero for `expiration`.
pub fn reaches_base_at(expiration: u64) -> u64 {
    expiration.saturating_add(AUCTION_DURATION_SECONDS)
}

/// Latest confirmed chain timestamp plus the fixed 420-second projection.
pub fn projected_pricing_timestamp(confirmed_timestamp: u64) -> u64 {
    confirmed_timestamp.saturating_add(PRICING_PROJECTION_OFFSET_SECONDS)
}

/// One-year base registration fee under the current confirmed base price.
pub fn base_registration_fee(base_price: u64, handle: &str) -> u64 {
    XchandlesFactorPricingPuzzleArgs::get_price(base_price, handle, DIRECTORY_REGISTRATION_PERIODS)
}

/// Day-0-through-day-28 auction premium at `buy_time` for a handle that expired at `expiration`.
///
/// Returns `0` when `buy_time` is before expiration or at/after the day-28 boundary.
pub fn auction_premium(expiration: u64, buy_time: u64) -> u64 {
    if buy_time < expiration {
        return 0;
    }
    let elapsed = buy_time - expiration;
    if elapsed >= AUCTION_DURATION_SECONDS {
        return 0;
    }
    premium_after_elapsed(elapsed)
}

fn premium_after_elapsed(elapsed: u64) -> u64 {
    let start_premium =
        XchandlesExponentialPremiumRenewPuzzleArgs::<()>::get_start_premium(SCALE_FACTOR);
    let end_value = XchandlesExponentialPremiumRenewPuzzleArgs::<()>::get_end_value(SCALE_FACTOR);

    let whole_periods = elapsed / HALVING_PERIOD;
    let fraction_part = (65536 * (elapsed % HALVING_PERIOD)) / HALVING_PERIOD;

    // START_PREMIUM / 2^whole_periods — periods stay below 28 while premium is positive.
    let mut premium = start_premium / (1u64 << whole_periods);
    let mut acc: u64 = 1;
    for bit in PREMIUM_BITS_LIST {
        acc <<= 1;
        if fraction_part & acc != 0 {
            premium = ((premium as u128) * (bit as u128) / (PREMIUM_PRECISION as u128)) as u64;
        }
    }

    premium.saturating_sub(end_value)
}

/// Total registration fee = one-year base + current auction premium.
pub fn total_registration_fee(
    base_price: u64,
    handle: &str,
    expiration: u64,
    buy_time: u64,
) -> u64 {
    base_registration_fee(base_price, handle).saturating_add(auction_premium(expiration, buy_time))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chia_wallet_sdk::driver::{SpendContext, XchandlesExpirePricingPuzzle};

    #[test]
    fn day0_and_day28_boundaries_match_sdk_puzzle() {
        let mut ctx = SpendContext::new();
        let args = XchandlesExpirePricingPuzzle::from_info(&mut ctx, 0, 31_557_600).unwrap();

        assert_eq!(auction_premium(0, 0), 99_999_999_628);
        assert_eq!(
            XchandlesExpirePricingPuzzle::get_price(
                &mut ctx,
                args.clone(),
                "yakuhito".into(),
                0,
                0,
                1
            )
            .unwrap() as u64,
            99_999_999_628
        );

        let day28 = AUCTION_DURATION_SECONDS;
        assert_eq!(auction_premium(0, day28), 0);
        assert_eq!(
            XchandlesExpirePricingPuzzle::get_price(
                &mut ctx,
                args.clone(),
                "yakuhito".into(),
                0,
                day28,
                1
            )
            .unwrap() as u64,
            0
        );

        assert!(auction_premium(0, day28 - 1) > 0);
        assert_eq!(reaches_base_at(1_000), 1_000 + day28);
    }

    #[test]
    fn total_fee_adds_base_and_premium() {
        let base = 5_000;
        let handle = "alice";
        let expiration = 1_000_000;
        let buy = expiration + 86_400;
        let premium = auction_premium(expiration, buy);
        let expected_base = base_registration_fee(base, handle);
        assert_eq!(
            total_registration_fee(base, handle, expiration, buy),
            expected_base + premium
        );
        assert_eq!(expected_base, base * 16); // length 5 letters → factor 16
    }

    #[test]
    fn projection_offset_is_exactly_420() {
        assert_eq!(projected_pricing_timestamp(1_700_000_000), 1_700_000_420);
        assert_eq!(PRICING_PROJECTION_OFFSET_SECONDS, 420);
    }
}

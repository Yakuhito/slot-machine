//! Premine buy_time / num_periods derivation from CSV expiration.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, TimeZone, Utc};

use crate::{CliError, REGISTRATION_PERIOD};

pub fn unix_now_secs() -> Result<u64, CliError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| CliError::Custom(format!("system clock before unix epoch: {e}")))
}

/// Buffered "past" reference for premine `buy_time`: UTC midnight on the 1st of the
/// month containing `now_secs`, except when `now` falls on the 1st — then use the
/// previous month's 1st so precommit vs deploy within that day cannot flip `n`.
pub fn buy_time_past_reference(now_secs: u64) -> u64 {
    let now = Utc
        .timestamp_opt(now_secs as i64, 0)
        .single()
        .expect("unix seconds fit chrono DateTime");
    let (y, m) = if now.day() == 1 {
        let prev = now - chrono::Months::new(1);
        (prev.year(), prev.month())
    } else {
        (now.year(), now.month())
    };
    Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
        .single()
        .expect("first of month is a valid UTC datetime")
        .timestamp() as u64
}

/// Lowest `n >= 1` such that `expiration - n * REGISTRATION_PERIOD` is strictly
/// before [`buy_time_past_reference`]. Handles that expire far ahead (e.g. ~2030)
/// therefore use `n > 1`.
///
/// After choosing `n`, asserts `buy_time + n * REGISTRATION_PERIOD == expiration`.
pub fn premine_buy_time(expiration: u64, now_secs: u64) -> Result<(u64, u64), CliError> {
    let reference = buy_time_past_reference(now_secs);
    if expiration < REGISTRATION_PERIOD {
        return Err(CliError::Custom(format!(
            "expiration {expiration} is below one registration period"
        )));
    }
    let mut n = 1u64;
    loop {
        let span = n.checked_mul(REGISTRATION_PERIOD).ok_or_else(|| {
            CliError::Custom(format!(
                "registration-period span overflow for n={n} expiration={expiration}"
            ))
        })?;
        let Some(buy_time) = expiration.checked_sub(span) else {
            return Err(CliError::Custom(format!(
                "expiration {expiration} cannot yield buy_time before reference {reference}"
            )));
        };
        if buy_time < reference {
            assert_intended_expiration(buy_time, n, expiration, REGISTRATION_PERIOD)?;
            return Ok((buy_time, n));
        }
        n = n.checked_add(1).ok_or_else(|| {
            CliError::Custom(format!(
                "n overflow while computing buy_time for expiration {expiration}"
            ))
        })?;
    }
}

/// Fail-closed check: intended on-chain expiration must equal the CSV expiration.
pub fn assert_intended_expiration(
    buy_time: u64,
    n: u64,
    csv_expiration: u64,
    registration_period: u64,
) -> Result<(), CliError> {
    let span = n.checked_mul(registration_period).ok_or_else(|| {
        CliError::Custom(format!(
            "registration-period span overflow for n={n} period={registration_period}"
        ))
    })?;
    let reconstituted = buy_time.checked_add(span).ok_or_else(|| {
        CliError::Custom("buy_time + n*period overflow".into())
    })?;
    if reconstituted != csv_expiration {
        return Err(CliError::Custom(format!(
            "buy_time invariant failed: {buy_time} + {n}*{registration_period} = {reconstituted}, expected {csv_expiration}"
        )));
    }
    Ok(())
}

/// Resolve timing for every row and assert each reconstitutes its CSV expiration.
/// Call before broadcasting any mint/register spend for a batch.
pub fn assert_batch_csv_expirations(
    rows: &[(String, u64)],
    now_secs: u64,
    registration_period: u64,
) -> Result<Vec<(u64, u64)>, CliError> {
    let mut timings = Vec::with_capacity(rows.len());
    for (handle, csv_expiration) in rows {
        let (buy_time, n) = premine_buy_time(*csv_expiration, now_secs)?;
        assert_intended_expiration(buy_time, n, *csv_expiration, registration_period).map_err(
            |e| {
                CliError::Custom(format!(
                    "handle '{handle}' expiration assert failed before broadcast: {e}"
                ))
            },
        )?;
        timings.push((buy_time, n));
    }
    Ok(timings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CONTRIBUTION_PREMINE_EXPIRATION;

    /// Fixed generation clock: 2026-08-03 UTC (within August → reference = 2026-08-01).
    const GEN_NOW: u64 = 1_785_629_000;

    #[test]
    fn buy_time_past_reference_uses_first_of_month_buffer() {
        assert_eq!(buy_time_past_reference(GEN_NOW), 1_785_542_400);
        assert_eq!(
            buy_time_past_reference(1_785_542_400),
            1_782_864_000 // 2026-07-01 UTC
        );
    }

    #[test]
    fn premine_buy_time_picks_lowest_n_in_the_past() {
        let (buy, n) = premine_buy_time(CONTRIBUTION_PREMINE_EXPIRATION, GEN_NOW).unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            buy,
            CONTRIBUTION_PREMINE_EXPIRATION - 2 * REGISTRATION_PERIOD
        );
        assert_eq!(buy + n * REGISTRATION_PERIOD, CONTRIBUTION_PREMINE_EXPIRATION);

        let (buy, n) = premine_buy_time(1_797_757_200, GEN_NOW).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buy, 1_797_757_200 - REGISTRATION_PERIOD);
        assert_eq!(buy + n * REGISTRATION_PERIOD, 1_797_757_200);

        let far = 1_893_456_000u64;
        let (buy, n) = premine_buy_time(far, GEN_NOW).unwrap();
        assert!(buy < buy_time_past_reference(GEN_NOW));
        assert!(n >= 2);
        assert_eq!(buy + n * REGISTRATION_PERIOD, far);
    }

    #[test]
    fn assert_intended_expiration_accepts_match_and_rejects_mismatch() {
        assert_intended_expiration(100, 2, 100 + 2 * REGISTRATION_PERIOD, REGISTRATION_PERIOD)
            .unwrap();
        let err =
            assert_intended_expiration(100, 2, 100 + REGISTRATION_PERIOD, REGISTRATION_PERIOD)
                .unwrap_err();
        assert!(err.to_string().contains("buy_time invariant failed"));
    }

    #[test]
    fn assert_batch_csv_expirations_fail_closed() {
        let ok_rows = vec![
            ("alice".to_string(), CONTRIBUTION_PREMINE_EXPIRATION),
            ("bob".to_string(), 1_797_757_200),
        ];
        let timings =
            assert_batch_csv_expirations(&ok_rows, GEN_NOW, REGISTRATION_PERIOD).unwrap();
        assert_eq!(timings.len(), 2);
        assert_eq!(timings[0].1, 2);
        assert_eq!(timings[1].1, 1);

        let bad_rows = vec![("broken".to_string(), 1u64)];
        let err = assert_batch_csv_expirations(&bad_rows, GEN_NOW, REGISTRATION_PERIOD).unwrap_err();
        assert!(err.to_string().contains("expiration"));
    }
}

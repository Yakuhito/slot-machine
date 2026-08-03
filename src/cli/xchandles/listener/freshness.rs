use std::time::{SystemTime, UNIX_EPOCH};

/// Freshness gate for every chain-dependent public read.
#[derive(Debug, Clone)]
pub struct FreshnessState {
    pub indexed_peak_height: u32,
    pub upstream_peak_height: u32,
    pub last_successful_peak_unix: u64,
    /// Latest confirmed transaction-block timestamp used for auction pricing.
    pub confirmed_timestamp: u64,
    pub rolling_back: bool,
    pub resyncing: bool,
}

impl FreshnessState {
    pub fn fresh_at(peak: u32, now_unix: u64) -> Self {
        Self {
            indexed_peak_height: peak,
            upstream_peak_height: peak,
            last_successful_peak_unix: now_unix,
            // Tests that need pricing override this; production updates on each peak.
            confirmed_timestamp: now_unix,
            rolling_back: false,
            resyncing: false,
        }
    }

    pub fn with_confirmed_timestamp(mut self, confirmed_timestamp: u64) -> Self {
        self.confirmed_timestamp = confirmed_timestamp;
        self
    }

    pub fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    pub fn is_fresh(&self, now_unix: u64) -> bool {
        if self.rolling_back || self.resyncing {
            return false;
        }
        if self.upstream_peak_height.saturating_sub(self.indexed_peak_height) > 16 {
            return false;
        }
        if now_unix.saturating_sub(self.last_successful_peak_unix) > 300 {
            return false;
        }
        true
    }
}

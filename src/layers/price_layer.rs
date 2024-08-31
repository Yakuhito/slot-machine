use once_cell::sync::Lazy;

// https://docs.chia.net/block-rewards/#rewards-schedule
pub static BLOCK_REWARD_SCHEDULE: Lazy<Vec<(u32, u64)>> = Lazy::new(|| {
    vec![
        (5_045_760, 1_000_000_000_000),
        (10_091_520, 500_000_000_000),
        (15_137_280, 250_000_000_000),
        (20_183_040, 125_000_000_000),
    ]
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceLayer {
    pub price_schedule: Vec<(u32, u64)>,
    pub generation: u32,
}

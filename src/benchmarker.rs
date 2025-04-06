#[allow(dead_code)]
#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;

    use chia::{
        bls::{SecretKey, Signature},
        consensus::spendbundle_conditions::get_conditions_from_spendbundle,
        protocol::{CoinSpend, SpendBundle},
    };
    use chia_wallet_sdk::{driver::SpendContext, test::Simulator, types::TESTNET11_CONSTANTS};

    pub struct Benchmark {
        pub data: HashMap<String, Vec<u64>>,
    }

    impl Benchmark {
        pub fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }

        pub fn add_spends(
            &mut self,
            ctx: &mut SpendContext,
            sim: &mut Simulator,
            key: &str,
            spends: Vec<CoinSpend>,
            keys: &[SecretKey],
        ) -> anyhow::Result<()> {
            let sb = SpendBundle::new(spends, Signature::default());
            let sb_conds = get_conditions_from_spendbundle(
                ctx,
                &sb,
                u64::MAX,
                sim.height(),
                &TESTNET11_CONSTANTS,
            )?;
            // add execution cost to storage cost
            let cost = sb_conds.cost
                + 12000
                    * sb.coin_spends.iter().fold(0, |acc, cs| {
                        acc + cs.puzzle_reveal.len() as u64 + cs.solution.len() as u64
                    });

            self.data.entry(key.to_string()).or_default().push(cost);

            sim.spend_coins(sb.coin_spends, keys)?;
            Ok(())
        }

        pub fn print_summary(&self) {
            for (key, data) in &self.data {
                let total = data.iter().sum::<u64>();
                let avg = total as f64 / data.len() as f64;
                let mut sorted = data.clone();
                sorted.sort();
                let data_min = sorted[0];
                let data_max = sorted[sorted.len() - 1];
                let data_median = if sorted.len() % 2 == 0 {
                    (sorted[sorted.len() / 2] + sorted[sorted.len() / 2 - 1]) as f64 / 2.0
                } else {
                    sorted[sorted.len() / 2] as f64
                };
                println!(
                    "{}: average={:.2}, num_samples={}, min={}, max={}, median={:.1}",
                    key,
                    avg,
                    data.len(),
                    data_min,
                    data_max,
                    data_median
                );
            }
        }
    }
}

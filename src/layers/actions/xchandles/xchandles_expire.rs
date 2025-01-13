use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{PrecommitLayer, Slot, SpendContextExt};

use super::XchandlesFactorPricingPuzzleArgs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesExpireAction {
    pub launcher_id: Bytes32,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
}

impl XchandlesExpireAction {
    pub fn new(
        launcher_id: Bytes32,
        precommit_payout_puzzle_hash: Bytes32,
        relative_block_height: u32,
    ) -> Self {
        Self {
            launcher_id,
            precommit_payout_puzzle_hash,
            relative_block_height,
        }
    }
}

impl Layer for XchandlesExpireAction {
    type Solution = XchandlesExpireActionSolution<NodePtr, (), NodePtr, u64>;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_expire_puzzle()?,
            args: XchandlesExpireActionArgs::new(
                self.launcher_id,
                self.relative_block_height,
                self.precommit_payout_puzzle_hash,
            ),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }

    fn parse_puzzle(
        _: &clvmr::Allocator,
        _: chia_wallet_sdk::Puzzle,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        unimplemented!()
    }

    fn parse_solution(_: &clvmr::Allocator, _: NodePtr) -> Result<Self::Solution, DriverError> {
        unimplemented!()
    }
}

impl ToTreeHash for XchandlesExpireAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesExpireActionArgs::curry_tree_hash(
            self.launcher_id,
            self.relative_block_height,
            self.precommit_payout_puzzle_hash,
        )
    }
}

pub const XCHANDLES_EXPIRE_PUZZLE: [u8; 1200] =
    hex!("ff02ffff01ff02ffff03ffff22ffff09ffff02ff2effff04ff02ffff04ff82015fff80808080ff4f80ffff09ffff02ff2effff04ff02ffff04ff8205dfff80808080ff81ef8080ffff01ff04ff2fffff04ffff04ff10ffff04ff83017fdfff808080ffff04ffff04ff10ffff04ff82bfdfff808080ffff04ffff04ff38ffff04ffff0effff0178ffff0bffff0102ffff0bffff0101ff819f80ffff0bffff0102ffff0bffff0101ffff10ffff06ffff02ff8205dfffff04ff819fffff04ff82bfdfffff04ff83017fdfffff04ff820bdfff80808080808080ff83017fdf8080ff8303ffdf808080ff808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff26ffff04ff02ffff04ffff0bffff0101ff819f80ffff04ff825fdfffff04ff82bfdfffff04ff8302ffdfff80808080808080ff8080808080ffff04ffff02ff2affff04ff02ffff04ff0bffff04ffff02ff26ffff04ff02ffff04ffff0bffff0101ff819f80ffff04ff825fdfffff04ffff10ffff06ffff02ff8205dfffff04ff819fffff04ff82bfdfffff04ff83017fdfffff04ff820bdfff80808080808080ff83017fdf80ffff04ff8303ffdfff80808080808080ff8080808080ffff04ffff04ff14ffff04ffff0113ffff04ff17ffff04ffff02ff82015fffff04ffff02ff3affff04ff02ffff04ff05ffff04ff8217dfffff04ffff0bffff0102ffff0bffff0102ff822fdfffff0bffff0101ff819f8080ffff0bffff0102ff8303ffdfffff0bffff0101ff83017fdf808080ff808080808080ffff04ff8202dfff80808080ffff04ffff05ffff02ff8205dfffff04ff819fffff04ff82bfdfffff04ff83017fdfffff04ff820bdfff80808080808080ff808080808080ff8080808080808080ffff01ff088080ff0180ffff04ffff01ffffff51ff333eff42ff02ff02ffff03ff05ffff01ff0bff72ffff02ff36ffff04ff02ffff04ff09ffff04ffff02ff3cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff04ff28ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ff0bff52ffff02ff36ffff04ff02ffff04ff05ffff04ffff02ff3cffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bffff0102ff05ffff0bffff0102ff0bffff0bffff0102ffff0bffff0101ff1780ff2f808080ff0bff2cffff0bff2cff62ff0580ffff0bff2cff0bff428080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff14ffff04ffff0112ffff04ff80ffff04ffff02ff3affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    a405c7f3f04b57c72bfc2d7876ae40492d2e496ac90640f22b1b672e754a6177
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExpireActionArgs {
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
    pub payout_puzzle_hash: Bytes32,
}

impl XchandlesExpireActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            precommit_1st_curry_hash: PrecommitLayer::<()>::first_curry_hash(
                SingletonStruct::new(launcher_id).tree_hash().into(),
                relative_block_height,
            )
            .into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
            payout_puzzle_hash,
        }
    }
}

impl XchandlesExpireActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPIRE_PUZZLE_HASH,
            args: XchandlesExpireActionArgs::new(
                launcher_id,
                relative_block_height,
                payout_puzzle_hash,
            ),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExpireActionSolution<CMP, CMS, P, S> {
    pub handle: String,
    pub cat_maker_puzzle_reveal: CMP,
    pub cat_maker_puzzle_solution: CMS,
    pub expired_handle_pricing_puzzle_reveal: P,
    pub expired_handle_pricing_puzzle_solution: S,
    pub refund_puzzle_hash_hash: Bytes32,
    pub secret_hash: Bytes32,
    pub neighbors_hash: Bytes32,
    pub current_expiration: u64,
    pub buy_time: u64,
    pub old_launcher_id_hash: Bytes32,
    #[clvm(rest)]
    pub new_launcher_id_hash: Bytes32,
}

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE: [u8; 369] =
    hex!("ff02ffff01ff04ffff10ffff05ffff02ff05ffff04ff81bfffff04ff8205ffff8080808080ffff02ff0effff04ff02ffff04ffff02ff0affff04ff02ffff04ff2fffff04ff5fffff04ffff0101ffff04ffff05ffff14ffff12ffff0183010000ffff06ffff14ffff11ff8202ffff82017f80ff04808080ff048080ffff04ffff05ffff14ff0bffff17ffff0101ffff05ffff14ffff11ff8202ffff82017f80ff048080808080ff8080808080808080ffff04ff17ff808080808080ffff06ffff02ff05ffff04ff81bfffff04ff8205ffff808080808080ffff04ffff01ff83015180ffff02ffff03ff0bffff01ff02ff0affff04ff02ffff04ff05ffff04ff1bffff04ffff17ff17ffff010180ffff04ff2fffff04ffff02ffff03ffff18ff2fff1780ffff01ff05ffff14ffff12ff5fff1380ff058080ffff015f80ff0180ff8080808080808080ffff015f80ff0180ff02ffff03ffff15ff05ff0b80ffff01ff11ff05ff0b80ff8080ff0180ff018080");

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    ae02a6a2992900bbb46a4216f38b74572b3efb71915932afbdcc827bd8576361
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExponentialPremiumRenewPuzzleArgs<P> {
    pub base_program: P,
    pub start_premium: u64,
    pub end_value: u64,
    pub precision: u64,
    pub bits_list: Vec<u64>,
}

pub const PREMIUM_PRECISION: u64 = 1_000_000_000_000_000_000; // 10^18

// https://github.com/ensdomains/ens-contracts/blob/master/contracts/ethregistrar/ExponentialPremiumPriceOracle.sol
pub const PREMIUM_BITS_LIST: [u64; 16] = [
    999989423469314432, // 0.5 ^ 1/65536 * (10 ** 18)
    999978847050491904, // 0.5 ^ 2/65536 * (10 ** 18)
    999957694548431104,
    999915390886613504,
    999830788931929088,
    999661606496243712,
    999323327502650752,
    998647112890970240,
    997296056085470080,
    994599423483633152,
    989228013193975424,
    978572062087700096,
    957603280698573696,
    917004043204671232,
    840896415253714560,
    707106781186547584,
];

impl XchandlesExponentialPremiumRenewPuzzleArgs<NodePtr> {
    pub fn get_start_premium(scale_factor: u64) -> u64 {
        100000000 * scale_factor // start auction at $100 million
    }

    pub fn get_end_value(scale_factor: u64) -> u64 {
        // 100000000 * 10 ** 18 // 2 ** 28 = 372529029846191406
        (372529029846191406_u128 * scale_factor as u128 / 1_000_000_000_000_000_000) as u64
    }

    // A scale factor is how many units of the payment token equate to $1
    // For exampe, you'd use scale_factor=1000 for wUSDC.b
    pub fn from_scale_factor(
        ctx: &mut SpendContext,
        base_price: u64,
        scale_factor: u64,
    ) -> Result<Self, DriverError> {
        Ok(Self {
            base_program: XchandlesFactorPricingPuzzleArgs::get_puzzle(ctx, base_price)?,
            start_premium: Self::get_start_premium(scale_factor),
            end_value: Self::get_end_value(scale_factor),
            precision: PREMIUM_PRECISION,
            bits_list: PREMIUM_BITS_LIST.to_vec(),
        })
    }

    pub fn curry_tree_hash(base_price: u64, scale_factor: u64) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH,
            args: XchandlesExponentialPremiumRenewPuzzleArgs::<TreeHash> {
                base_program: XchandlesFactorPricingPuzzleArgs::curry_tree_hash(base_price),
                start_premium: Self::get_start_premium(scale_factor),
                end_value: Self::get_end_value(scale_factor),
                precision: PREMIUM_PRECISION,
                bits_list: PREMIUM_BITS_LIST.to_vec(),
            },
        }
        .tree_hash()
    }

    pub fn get_puzzle(self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.xchandles_exponential_premium_renew_puzzle()?,
            args: self,
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_price(
        self,
        ctx: &mut SpendContext,
        handle: String,
        expiration: u64,
        buy_time: u64,
        num_years: u64,
    ) -> Result<u128, DriverError> {
        let puzzle = self.get_puzzle(ctx)?;
        let solution = XchandlesExponentialPremiumRenewPuzzleSolution::<u64> {
            handle,
            expiration,
            buy_time,
            pricing_program_solution: num_years,
        }
        .to_clvm(&mut ctx.allocator)?;
        let output = ctx.run(puzzle, solution)?;

        Ok(<(u128, u64)>::from_clvm(&ctx.allocator, output)?.0)
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExponentialPremiumRenewPuzzleSolution<S> {
    pub handle: String,
    pub expiration: u64,
    pub buy_time: u64,
    pub pricing_program_solution: S,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
    #[clvm(list)]
    pub struct XchandlesPricingOutput {
        pub price: u128,
        #[clvm(rest)]
        pub registered_time: u64,
    }

    #[test]
    fn test_xchandles_exponential_premium_puzzle() -> Result<(), DriverError> {
        let mut ctx = SpendContext::new();

        let puzzle =
            XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(&mut ctx, 0, 1000)?
                .get_puzzle(&mut ctx)?;

        let mut last_price = 100_000_000_000;
        for day in 0..28 {
            for hour in 0..24 {
                let buy_time = day * 24 * 60 * 60 + hour * 60 * 60;
                let solution = XchandlesExponentialPremiumRenewPuzzleSolution::<u64> {
                    handle: "yakuhito".to_string(),
                    expiration: 0,
                    buy_time,
                    pricing_program_solution: 1,
                }
                .to_clvm(&mut ctx.allocator)?;

                let output = ctx.run(puzzle, solution)?;
                let output = XchandlesPricingOutput::from_clvm(&ctx.allocator, output)?;

                assert_eq!(output.registered_time, 366 * 24 * 60 * 60);

                if hour == 0 {
                    let scale_factor =
                        372529029846191406_u128 * 1000_u128 / 1_000_000_000_000_000_000_u128;
                    assert_eq!(
                        output.price,
                        (100_000_000 * 1000) / (1 << day) - scale_factor
                    );
                }

                assert!(output.price < last_price);
                last_price = output.price;

                assert_eq!(
                    XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
                        &mut ctx, 0, 1000
                    )?
                    .get_price(
                        &mut ctx,
                        "yakuhito".to_string(),
                        0,
                        buy_time,
                        1
                    )?,
                    output.price
                );
            }
        }

        // check premium after auction is 0
        let solution = XchandlesExponentialPremiumRenewPuzzleSolution::<u64> {
            handle: "yakuhito".to_string(),
            expiration: 0,
            buy_time: 28 * 24 * 60 * 60,
            pricing_program_solution: 1,
        }
        .to_clvm(&mut ctx.allocator)?;

        let output = ctx.run(puzzle, solution)?;
        let output = XchandlesPricingOutput::from_clvm(&ctx.allocator, output)?;

        assert_eq!(output.registered_time, 366 * 24 * 60 * 60);
        assert_eq!(output.price, 0);

        Ok(())
    }
}

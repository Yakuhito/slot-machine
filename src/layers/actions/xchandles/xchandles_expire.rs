use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{Slot, SpendContextExt};

use super::XchandlesFactorPricingPuzzleArgs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesExpireAction {
    pub launcher_id: Bytes32,
}

impl XchandlesExpireAction {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self { launcher_id }
    }
}

impl Layer for XchandlesExpireAction {
    type Solution = XchandlesExpireActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_expire_puzzle()?,
            args: XchandlesExpireActionArgs::new(self.launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: XchandlesExpireActionSolution,
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
        XchandlesExpireActionArgs::curry_tree_hash(self.launcher_id)
    }
}

pub const XCHANDLES_EXPIRE_PUZZLE: [u8; 359] =
    hex!("ff02ffff01ff04ffff10ffff05ffff02ff05ffff04ff81bfffff04ff8205ffff8080808080ffff02ff0effff04ff02ffff04ffff02ff0affff04ff02ffff04ff2fffff04ff5fffff04ffff0101ffff04ffff05ffff14ffff12ffff0183010000ffff3dffff11ff8202ffff82017f80ff048080ff048080ffff04ffff05ffff14ff0bffff17ffff0101ffff05ffff14ffff11ff8202ffff82017f80ff048080808080ff8080808080808080ff8080808080ffff06ffff02ff05ffff04ff81bfffff04ff8205ffff808080808080ffff04ffff01ff83015180ffff02ffff03ff0bffff01ff02ff0affff04ff02ffff04ff05ffff04ff1bffff04ffff17ff17ffff010180ffff04ff2fffff04ffff02ffff03ffff18ff2fff1780ffff01ff05ffff14ffff12ff5fff1380ff058080ffff015f80ff0180ff8080808080808080ffff015f80ff0180ff02ffff03ffff15ff05ff0b80ffff01ff11ff05ff0b80ff8080ff0180ff018080");

pub const XCHANDLES_EXPIRE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    cc83b3e95c48efc00053cdfa3c2aaa837695331a3b98016c5023bdfb5be54587
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesExpireActionArgs {
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesExpireActionArgs {
    pub fn new(launcher_id: Bytes32) -> Self {
        Self {
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id).into(),
        }
    }
}

impl XchandlesExpireActionArgs {
    pub fn curry_tree_hash(launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPIRE_PUZZLE_HASH,
            args: XchandlesExpireActionArgs::new(launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesExpireActionSolution {
    pub value: Bytes32,
    pub left_value: Bytes32,
    pub left_left_value: Bytes32,
    pub left_rest_hash: Bytes32,
    pub right_value: Bytes32,
    pub right_right_value: Bytes32,
    pub right_rest_hash: Bytes32,
    pub expiration: u64,
    #[clvm(rest)]
    pub launcher_id_hash: Bytes32,
}

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE: [u8; 125] =
    hex!("ff02ffff01ff04ffff04ffff11ff8202ffff82017f80ffff04ff02ffff04ffff0101ffff04ffff3dffff11ff8202ffff82017f80ff0280ffff04ffff14ffff11ff8202ffff82017f80ff0280ff808080808080ffff06ffff02ff05ffff04ff81bfffff04ff8205ffff808080808080ffff04ffff0183015180ff018080");

pub const XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    faa63cb8ac1800f95e0a4fa823e6755df7331c68d8f617b9adf5ea63fc889770
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

impl XchandlesExponentialPremiumRenewPuzzleArgs<NodePtr> {
    // A scale factor is how many units of the payment token equate to $1
    // For exampe, you'd use scale_factor=1000 for wUSDC.b
    pub fn from_scale_factor(
        ctx: &mut SpendContext,
        base_price: u64,
        scale_factor: u64,
    ) -> Result<Self, DriverError> {
        Ok(Self {
            base_program: XchandlesFactorPricingPuzzleArgs::new(base_price).get_puzzle(ctx)?,
            start_premium: 100000000 * scale_factor, // start auction at $100 million
            end_value: (372529029846191406_u128 * scale_factor as u128 / 1_000_000_000_000_000_000)
                as u64, // 100000000 * 10 ** 18 // 2 ** 28
            precision: 1_000_000_000_000_000_000,    // 10^18
            // https://github.com/ensdomains/ens-contracts/blob/master/contracts/ethregistrar/ExponentialPremiumPriceOracle.sol
            bits_list: vec![
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
            ],
        })
    }

    pub fn get_puzzle(self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.xchandles_exponential_premium_renew_puzzle()?,
            args: self,
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }
}

impl<P> XchandlesExponentialPremiumRenewPuzzleArgs<P>
where
    P: ToTreeHash,
{
    pub fn curry_tree_hash(self) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_EXPONENTIAL_PREMIUM_RENEW_PUZZLE_HASH,
            args: XchandlesExponentialPremiumRenewPuzzleArgs::<TreeHash> {
                base_program: self.base_program.tree_hash(),
                start_premium: self.start_premium,
                end_value: self.end_value,
                precision: self.precision,
                bits_list: self.bits_list,
            },
        }
        .tree_hash()
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
    use clvmr::serde::node_to_bytes;

    use super::*;

    #[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
    #[clvm(list)]
    pub struct XchandlesPricingOutput {
        pub price: u64,
        #[clvm(rest)]
        pub registered_time: u64,
    }

    #[test]
    fn test_xchandles_exponential_premium_puzzle() -> Result<(), DriverError> {
        let mut ctx = SpendContext::new();

        let puzzle =
            XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(&mut ctx, 0, 1000)?
                .get_puzzle(&mut ctx)?;
        println!(
            "puzzle: {:?}",
            hex::encode(node_to_bytes(&ctx.allocator, puzzle).unwrap())
        );

        for day in 0..2 {
            for hour in 0..24 {
                let solution = XchandlesExponentialPremiumRenewPuzzleSolution::<u64> {
                    handle: "yakuhito".to_string(),
                    expiration: 0,
                    buy_time: day * 24 * 60 * 60 + hour * 60 * 60,
                    pricing_program_solution: 1,
                }
                .to_clvm(&mut ctx.allocator)?;

                let output = ctx.run(puzzle, solution)?;
                // let output = XchandlesPricingOutput::from_clvm(&ctx.allocator, output)?;
                println!(
                    "day:\t{}\thour:\t{}\tbuy_time:\t{}\t- result is\t{:}",
                    day,
                    hour,
                    day * 24 * 60 * 60 + hour * 60 * 60,
                    // output.price
                    hex::encode(node_to_bytes(&ctx.allocator, output).unwrap())
                );
                // todo: debug
                if hour > 0 && hour % 6 == 0 {
                    println!(
                        "solution: {:?}",
                        hex::encode(node_to_bytes(&ctx.allocator, solution).unwrap())
                    );
                }
            }
        }

        Ok(())
    }
}

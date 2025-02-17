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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesRegisterAction {
    pub launcher_id: Bytes32,
    pub relative_block_height: u32,
    pub payout_puzzle_hash: Bytes32,
}

impl XchandlesRegisterAction {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            launcher_id,
            relative_block_height,
            payout_puzzle_hash,
        }
    }
}

impl Layer for XchandlesRegisterAction {
    type Solution =
        XchandlesRegisterActionSolution<NodePtr, XchandlesFactorPricingSolution, NodePtr, ()>;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_register_puzzle()?,
            args: XchandlesRegisterActionArgs::new(
                self.launcher_id,
                self.relative_block_height,
                self.payout_puzzle_hash,
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

impl ToTreeHash for XchandlesRegisterAction {
    fn tree_hash(&self) -> TreeHash {
        XchandlesRegisterActionArgs::curry_tree_hash(
            self.launcher_id,
            self.relative_block_height,
            self.payout_puzzle_hash,
        )
    }
}

pub const XCHANDLES_REGISTER_PUZZLE: [u8; 1499] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff4fffff0bffff0101ff8215ef8080ffff20ff8209ef80ffff15ff4fff81af80ffff15ff82016fff4f80ffff09ff27ffff02ff2effff04ff02ffff04ff820befff8080808080ffff09ff57ffff02ff2effff04ff02ffff04ff8202efff8080808080ffff010180ffff01ff02ff36ffff04ff02ffff04ff05ffff04ff0bffff04ff17ffff04ff8207efffff04ff4fffff04ffff0bffff0101ff4f80ffff04ffff0bffff0101ff81af80ffff04ffff0bffff0101ff82016f80ffff04ffff02ff8202efff8205ef80ffff04ffff02ff2effff04ff02ffff04ff8205efff80808080ff80808080808080808080808080ffff01ff088080ff0180ffff04ffff01ffffff51ff333effff4202ffff02ffff03ff05ffff01ff0bff81fcffff02ff26ffff04ff02ffff04ff09ffff04ffff02ff2cffff04ff02ffff04ff0dff80808080ff808080808080ffff0181dc80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff04ff28ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffff0bff81bcffff02ff26ffff04ff02ffff04ff05ffff04ffff02ff2cffff04ff02ffff04ff07ff80808080ff808080808080ff0bffff0102ff05ffff0bffff0102ffff0bffff0102ff0bff1780ff2f8080ffffff0bff34ffff0bff34ff81dcff0580ffff0bff34ff0bff819c8080ff04ff17ffff04ffff04ff10ffff04ff8202efff808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff82017fffff04ff8217efffff04ff8202ffffff04ff822fefff80808080808080ff8080808080ffff04ffff02ff3effff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff8202ffffff04ff82017fffff04ff825fefffff04ff82bfefff80808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff81bfffff04ff82017fffff04ff8202ffffff04ffff0bffff0102ffff0bffff0101ffff10ff8202efff820dff8080ff82016f80ff80808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff82017fffff04ff8217efffff04ff81bfffff04ff822fefff80808080808080ff8080808080ffff04ffff02ff12ffff04ff02ffff04ff0bffff04ffff02ff3affff04ff02ffff04ff8202ffffff04ff81bfffff04ff825fefffff04ff82bfefff80808080808080ff8080808080ffff04ffff04ff38ffff04ffff0effff0172ffff0bffff0102ff5fffff0bffff0102ffff0bffff0101ffff10ff820dffff8202ef8080ff82016f808080ff808080ffff04ffff04ff24ffff04ffff0113ffff04ffff0101ffff04ffff02ff4fffff04ffff02ff2affff04ff02ffff04ff05ffff04ff820befffff04ffff0bffff0102ffff0bffff0101ffff0bffff0102ffff0bffff0102ff27ffff02ff2effff04ff02ffff04ff81afff8080808080ffff0bffff0102ff57ff820bff808080ffff0bffff0102ffff0bffff0102ff8205efff5f80ffff0bffff0102ffff0bffff0101ff8202ef80ff82016f808080ff808080808080ffff04ff81afff80808080ffff04ff8209ffff808080808080ff80808080808080808080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff04ff24ffff04ffff0112ffff04ff80ffff04ffff02ff2affff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ff018080");

pub const XCHANDLES_REGISTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    42060231b8f1dea4c93951e7da34b86350f1d511d44e107683c147e9ab070b67
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesRegisterActionArgs {
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
}

impl XchandlesRegisterActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            precommit_1st_curry_hash: PrecommitLayer::<()>::first_curry_hash(
                SingletonStruct::new(launcher_id).tree_hash().into(),
                relative_block_height,
                payout_puzzle_hash,
            )
            .into(),
            slot_1st_curry_hash: Slot::<()>::first_curry_hash(launcher_id, None).into(),
        }
    }
}

impl XchandlesRegisterActionArgs {
    pub fn curry_tree_hash(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_REGISTER_PUZZLE_HASH,
            args: XchandlesRegisterActionArgs::new(
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
pub struct XchandlesRegisterActionSolution<PP, PS, CMP, CMS> {
    pub handle_hash: Bytes32,
    pub left_value: Bytes32,
    pub right_value: Bytes32,
    pub pricing_puzzle_reveal: PP,
    pub pricing_puzzle_solution: PS,
    pub cat_maker_reveal: CMP,
    pub cat_maker_solution: CMS,
    pub rest_data_hash: Bytes32,
    pub start_time: u64,
    pub secret_hash: Bytes32,
    pub refund_puzzle_hash_hash: Bytes32,
    pub left_left_value_hash: Bytes32,
    pub left_data_hash: Bytes32,
    pub right_right_value_hash: Bytes32,
    pub right_data_hash: Bytes32,
}

pub const XCHANDLES_FACTOR_PRICING_PUZZLE: [u8; 481] = hex!("ff02ffff01ff02ffff03ffff15ff1fff8080ffff01ff04ffff12ff1fff05ffff02ff0effff04ff02ffff04ffff0dff1780ffff04ffff02ff0affff04ff02ffff04ff17ff80808080ff808080808080ffff12ff1fff048080ffff01ff088080ff0180ffff04ffff01ff8401e28500ffff02ffff03ff05ffff01ff02ffff03ffff22ffff15ffff0cff05ff80ffff010180ffff016080ffff15ffff017bffff0cff05ff80ffff0101808080ffff01ff02ff0affff04ff02ffff04ffff0cff05ffff010180ff80808080ffff01ff02ffff03ffff22ffff15ffff0cff05ff80ffff010180ffff012f80ffff15ffff013affff0cff05ff80ffff0101808080ffff01ff10ffff0101ffff02ff0affff04ff02ffff04ffff0cff05ffff010180ff8080808080ffff01ff088080ff018080ff0180ff8080ff0180ff05ffff14ffff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0110ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010280ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff01ff01820080ffff01ff014080ff018080ff0180ffff01ff088080ff0180ffff03ff0bffff0102ffff0101808080ff018080");

pub const XCHANDLES_FACTOR_PRICING_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    129896065abb6e13cce6f46c784add16c771336cfa39a5647644a95a0ee0abd7
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesFactorPricingPuzzleArgs {
    pub base_price: u64,
}

impl XchandlesFactorPricingPuzzleArgs {
    pub fn new(base_price: u64) -> Self {
        Self { base_price }
    }

    pub fn get_puzzle(ctx: &mut SpendContext, base_price: u64) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.xchandles_factor_pricing_puzzle()?,
            args: XchandlesFactorPricingPuzzleArgs::new(base_price),
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    pub fn get_price(base_price: u64, handle: &str, num_years: u64) -> u64 {
        base_price
            * match handle.len() {
                3 => 128,
                4 => 64,
                5 => 16,
                _ => 2,
            }
            / if handle.contains(|c: char| c.is_numeric()) {
                2
            } else {
                1
            }
            * num_years
    }
}

impl XchandlesFactorPricingPuzzleArgs {
    pub fn curry_tree_hash(base_price: u64) -> TreeHash {
        CurriedProgram {
            program: XCHANDLES_FACTOR_PRICING_PUZZLE_HASH,
            args: XchandlesFactorPricingPuzzleArgs::new(base_price),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct XchandlesFactorPricingSolution {
    pub current_expiration: u64,
    pub handle: String,
    #[clvm(rest)]
    pub num_years: u64,
}

#[cfg(test)]
mod tests {
    use clvmr::reduction::EvalErr;

    use super::*;

    #[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
    #[clvm(list)]
    pub struct XchandlesFactorPricingOutput {
        pub price: u64,
        #[clvm(rest)]
        pub registered_time: u64,
    }

    #[test]
    fn test_factor_pricing_puzzle() -> Result<(), DriverError> {
        let mut ctx = SpendContext::new();
        let base_price = 1; // puzzle will only spit out factors

        let puzzle = XchandlesFactorPricingPuzzleArgs::get_puzzle(&mut ctx, base_price)?;

        for handle_length in 3..=31 {
            for num_years in 1..=3 {
                for has_number in [false, true] {
                    let handle = if has_number {
                        "a".repeat(handle_length - 1) + "1"
                    } else {
                        "a".repeat(handle_length)
                    };

                    let solution = XchandlesFactorPricingSolution {
                        current_expiration: (handle_length - 3) as u64, // shouldn't matter
                        handle,
                        num_years,
                    }
                    .to_clvm(&mut ctx.allocator)?;

                    let output = ctx.run(puzzle, solution)?;
                    let output = XchandlesFactorPricingOutput::from_clvm(&ctx.allocator, output)?;

                    let mut expected_price = if handle_length == 3 {
                        128
                    } else if handle_length == 4 {
                        64
                    } else if handle_length == 5 {
                        16
                    } else {
                        2
                    };
                    if has_number {
                        expected_price /= 2;
                    }
                    expected_price *= num_years;

                    assert_eq!(output.price, expected_price);
                    assert_eq!(output.registered_time, num_years * 366 * 24 * 60 * 60);
                }
            }
        }

        // make sure the puzzle won't let us register a handle of length 2

        let solution = XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: "aa".to_string(),
            num_years: 1,
        }
        .to_clvm(&mut ctx.allocator)?;

        let Err(DriverError::Eval(EvalErr(_, s))) = ctx.run(puzzle, solution) else {
            panic!("Expected error");
        };
        assert_eq!(s, "clvm raise");

        // make sure the puzzle won't let us register a handle of length 32

        let solution = XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: "a".repeat(32),
            num_years: 1,
        }
        .to_clvm(&mut ctx.allocator)?;

        let Err(DriverError::Eval(EvalErr(_, s))) = ctx.run(puzzle, solution) else {
            panic!("Expected error");
        };
        assert_eq!(s, "clvm raise");

        // make sure the puzzle won't let us register a handle with invalid characters

        let solution = XchandlesFactorPricingSolution {
            current_expiration: 0,
            handle: "yak@test".to_string(),
            num_years: 1,
        }
        .to_clvm(&mut ctx.allocator)?;

        let Err(DriverError::Eval(EvalErr(_, s))) = ctx.run(puzzle, solution) else {
            panic!("Expected error");
        };
        assert_eq!(s, "clvm raise");

        Ok(())
    }
}

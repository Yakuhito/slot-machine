use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::{cat::CAT_PUZZLE_HASH, singleton::SingletonStruct},
};
use chia_wallet_sdk::{DriverError, Layer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::{PrecommitLayer, Slot, SpendContextExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XchandlesRegisterAction {
    pub launcher_id: Bytes32,
    pub precommit_payout_puzzle_hash: Bytes32,
    pub relative_block_height: u32,
}

impl XchandlesRegisterAction {
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

    pub fn get_price_factor(handle: &str) -> Option<u64> {
        match handle.len() {
            0..3 => None,
            3 => Some(64),
            4 => Some(32),
            5 => Some(8),
            6..31 => Some(1),
            _ => None,
        }
    }
}

impl Layer for XchandlesRegisterAction {
    type Solution = XchandlesRegisterActionSolution;

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.xchandles_register_puzzle()?,
            args: XchandlesRegisterActionArgs::new(
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
        solution: XchandlesRegisterActionSolution,
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
            self.precommit_payout_puzzle_hash,
        )
    }
}

pub const XCHANDLES_REGISTER_PUZZLE: [u8; 1672] = hex!("ff02ffff01ff02ffff03ffff22ffff09ff82013fffff0bffff0101ff8202bf8080ffff15ff82013fff8205bf80ffff15ff820bbfff82013f8080ffff01ff02ff26ffff04ff02ffff04ff05ffff04ff0bffff04ff17ffff04ff2fffff04ff5fffff04ff820fbfffff04ff82013fffff04ffff0bffff0101ff82013f80ffff04ffff0bffff0101ff8205bf80ffff04ffff0bffff0101ff820bbf80ffff04ffff12ffff02ff32ffff04ff02ffff04ffff02ff2effff04ff02ffff04ff8202bfff80808080ff80808080ff81df80ff8080808080808080808080808080ffff01ff088080ff0180ffff04ffff01ffffffff5133ff4202ffffff02ffff03ff05ffff01ff0bff81ecffff02ff3affff04ff02ffff04ff09ffff04ffff02ff24ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181cc80ff0180ff02ffff03ffff15ff05ff8080ffff0105ffff01ff088080ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff04ff30ffff04ffff02ff22ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ffff04ff80ffff04ffff04ff05ff8080ff8080808080ffffffff0bff81acffff02ff3affff04ff02ffff04ff05ffff04ffff02ff24ffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0108ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010180ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff01ff0140ffff01ff012080ff018080ff0180ffff01ff088080ff0180ffff0bffff0102ff05ffff0bffff0102ffff0bffff0102ff0bff1780ff2f8080ff0bff38ffff0bff38ff81ccff0580ffff0bff38ff0bff818c8080ffffff04ff5fffff04ffff04ff20ffff04ff8202bfff808080ffff04ffff02ff36ffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8205ffffff04ff822fbfffff04ff820bffffff04ff825fbfff80808080808080ff8080808080ffff04ffff02ff36ffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff820bffffff04ff8205ffffff04ff82bfbfffff04ff83017fbfff80808080808080ff8080808080ffff04ffff02ff3cffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8202ffffff04ff8205ffffff04ff820bffffff04ffff0bffff0102ffff0bffff0101ffff10ff8202bfffff12ffff013cffff013cffff0118ffff0182016effff02ff34ffff04ff02ffff04ffff05ffff14ff8217bfff8217ff8080ff80808080808080ffff0bffff0101ff82013f8080ff80808080808080ff8080808080ffff04ffff02ff3cffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff8205ffffff04ff822fbfffff04ff8202ffffff04ff825fbfff80808080808080ff8080808080ffff04ffff02ff3cffff04ff02ffff04ff17ffff04ffff02ff2affff04ff02ffff04ff820bffffff04ff8202ffffff04ff82bfbfffff04ff83017fbfff80808080808080ff8080808080ffff04ffff04ff28ffff04ffff0113ffff04ff2fffff04ffff02ff22ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0580ffff04ff819fffff04ffff02ff22ffff04ff02ffff04ff0bffff04ff820bbfffff04ffff0bffff0102ffff0bffff0102ff8205bfff82017f80ffff0bffff0102ffff0bffff0101ff82013f80ffff0bffff0101ff8202bf808080ff808080808080ff80808080808080ffff04ff8217bfff808080808080ff808080808080808080ff04ff28ffff04ffff0112ffff04ff80ffff04ffff02ff22ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0b80ff8080808080ff8080808080ffff02ffff03ff05ffff01ff02ffff03ffff02ff3effff04ff02ffff04ffff0cff05ff80ffff010180ff80808080ffff01ff10ffff0101ffff02ff2effff04ff02ffff04ffff0cff05ffff010180ff8080808080ffff01ff088080ff0180ff8080ff0180ff21ffff22ffff15ff05ffff016080ffff15ffff017bff058080ffff22ffff15ff05ffff012f80ffff15ffff013aff05808080ff018080");

pub const XCHANDLES_REGISTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    f1bfa50fe9a079c3660ab3922502827541333f138317b006033d02c661fe0806
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct XchandlesRegisterActionArgs {
    pub cat_mod_hash: Bytes32,
    pub precommit_1st_curry_hash: Bytes32,
    pub slot_1st_curry_hash: Bytes32,
    pub payout_puzzle_hash: Bytes32,
}

impl XchandlesRegisterActionArgs {
    pub fn new(
        launcher_id: Bytes32,
        relative_block_height: u32,
        payout_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            cat_mod_hash: CAT_PUZZLE_HASH.into(),
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
pub struct XchandlesRegisterActionSolution {
    pub handle_hash: Bytes32,
    pub handle_reveal: String,
    pub left_value: Bytes32,
    pub right_value: Bytes32,
    pub handle_nft_launcher_id: Bytes32,
    pub start_time: u64,
    pub secret_hash: Bytes32,
    pub refund_puzzle_hash_hash: Bytes32,
    pub precommitment_amount: u64,
    pub left_left_value_hash: Bytes32,
    pub left_data_hash: Bytes32,
    pub right_right_value_hash: Bytes32,
    pub right_data_hash: Bytes32,
}

pub const XCHANDLES_FACTOR_PRICING_PUZZLE: [u8; 481] = hex!("ff02ffff01ff02ffff03ffff15ff17ff8080ffff01ff04ffff12ff17ff05ffff02ff0effff04ff02ffff04ffff0dff0b80ffff04ffff02ff0affff04ff02ffff04ff0bff80808080ff808080808080ffff12ff17ff048080ffff01ff088080ff0180ffff04ffff01ff8401e28500ffff02ffff03ff05ffff01ff02ffff03ffff22ffff15ffff0cff05ff80ffff010180ffff016080ffff15ffff017bffff0cff05ff80ffff0101808080ffff01ff02ff0affff04ff02ffff04ffff0cff05ffff010180ff80808080ffff01ff02ffff03ffff22ffff15ffff0cff05ff80ffff010180ffff012f80ffff15ffff013affff0cff05ff80ffff0101808080ffff01ff10ffff0101ffff02ff0affff04ff02ffff04ffff0cff05ffff010180ff8080808080ffff01ff088080ff018080ff0180ff8080ff0180ff05ffff14ffff02ffff03ffff15ff05ffff010280ffff01ff02ffff03ffff15ff05ffff010480ffff01ff02ffff03ffff09ff05ffff010580ffff01ff0110ffff01ff02ffff03ffff15ff05ffff011f80ffff01ff0880ffff01ff010280ff018080ff0180ffff01ff02ffff03ffff09ff05ffff010380ffff01ff01820080ffff01ff014080ff018080ff0180ffff01ff088080ff0180ffff03ff0bffff0102ffff0101808080ff018080");

pub const XCHANDLES_FACTOR_PRICING_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    2cd58a34df34d7deb42d9401672ee28a7d5149b36e89e320cdbeb5d504b34ff9
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

    pub fn get_puzzle(self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.xchandles_factor_pricing_puzzle()?,
            args: self,
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
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
    pub handle: String,
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
    fn test_xchandles_factor_pricing_puzzle() -> Result<(), DriverError> {
        let mut ctx = SpendContext::new();
        let base_price = 1; // puzzle will only spit out factors

        let puzzle = XchandlesFactorPricingPuzzleArgs::new(base_price).get_puzzle(&mut ctx)?;

        for handle_length in 3..=31 {
            for num_years in 1..=3 {
                for has_number in [false, true] {
                    let handle = if has_number {
                        "a".repeat(handle_length - 1) + "1"
                    } else {
                        "a".repeat(handle_length)
                    };

                    let solution = XchandlesFactorPricingSolution { handle, num_years }
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

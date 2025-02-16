use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{DriverError, Layer};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigAddIncentivesAction {
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
}

impl Layer for DigAddIncentivesAction {
    type Solution = DigAddIncentivesActionSolution;

    fn construct_puzzle(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
    ) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.dig_add_incentives_action_puzzle()?,
            args: DigAddIncentivesActionArgs {
                validator_payout_puzzle_hash: self.validator_payout_puzzle_hash,
                validator_fee_bps: self.validator_fee_bps,
            },
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut chia_wallet_sdk::SpendContext,
        solution: DigAddIncentivesActionSolution,
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

impl ToTreeHash for DigAddIncentivesAction {
    fn tree_hash(&self) -> TreeHash {
        Self::curry_tree_hash(self.validator_payout_puzzle_hash, self.validator_fee_bps)
    }
}

impl DigAddIncentivesAction {
    pub fn curry_tree_hash(
        validator_payout_puzzle_hash: Bytes32,
        validator_fee_bps: u64,
    ) -> TreeHash {
        CurriedProgram {
            program: DIG_ADD_INCENTIVES_PUZZLE_HASH,
            args: DigAddIncentivesActionArgs {
                validator_payout_puzzle_hash,
                validator_fee_bps,
            },
        }
        .tree_hash()
    }
}

pub const DIG_ADD_INCENTIVES_PUZZLE: [u8; 262] = hex!("ff02ffff01ff02ffff03ffff22ffff15ff820377ff82027780ffff15ff4fff8080ffff09ff6fffff05ffff14ffff12ff4fff0b80ffff0182271080808080ffff01ff04ffff04ffff10ff27ffff11ff4fff6f8080ffff04ff57ffff04ffff04ff820137ffff10ff8201b7ffff11ff4fff6f808080ffff04ffff04ff820277ff82037780ff8080808080ffff04ffff04ff06ffff04ffff0effff0163ffff0bffff0102ffff0bffff0101ff4f80ffff0bffff0101ff820377808080ff808080ffff04ffff04ffff0181d6ffff04ff04ffff04ff05ffff04ff6fffff04ffff04ff05ff8080ff808080808080ff80808080ffff01ff088080ff0180ffff04ffff01ff333eff018080");

pub const DIG_ADD_INCENTIVES_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    58e78921c64a28b6caa8bb4ca6b0eebbb5f78a237fbc1f9278cf24594e1a6e58
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DigAddIncentivesActionArgs {
    pub validator_payout_puzzle_hash: Bytes32,
    pub validator_fee_bps: u64,
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DigAddIncentivesActionSolution {
    pub amount: u64,
    #[clvm(rest)]
    pub validator_fee: u64,
}

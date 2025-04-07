use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::LineageProof,
};
use chia_puzzles::{SINGLETON_LAUNCHER_HASH, SINGLETON_TOP_LAYER_V1_1_HASH};
use chia_wallet_sdk::driver::{DriverError, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Copy, Clone)]
#[must_use]
pub struct VerificationPayments {
    pub verifier_singleton_struct_hash: Bytes32,
    pub verification_inner_puzzle_hash: Bytes32,
}

impl VerificationPayments {
    pub fn new(
        verifier_singleton_struct_hash: Bytes32,
        verification_inner_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            verifier_singleton_struct_hash,
            verification_inner_puzzle_hash,
        }
    }

    pub fn tree_hash(&self) -> TreeHash {
        CurriedProgram {
            program: VERIFICATION_PAYMENTS_PUZZLE_HASH,
            args: VerificationPaymentsArgs::new(
                self.verifier_singleton_struct_hash,
                self.verification_inner_puzzle_hash,
            ),
        }
        .tree_hash()
    }

    pub fn inner_spend(
        &self,
        ctx: &mut SpendContext,
        solution: &VerificationPaymentsSolution,
    ) -> Result<Spend, DriverError> {
        let program = ctx.verification_payments_puzzle()?;
        let puzzle = ctx.alloc(&CurriedProgram {
            program,
            args: VerificationPaymentsArgs::new(
                self.verifier_singleton_struct_hash,
                self.verification_inner_puzzle_hash,
            ),
        })?;

        let solution = ctx.alloc(&solution)?;

        Ok(Spend::new(puzzle, solution))
    }
}

pub const VERIFICATION_PAYMENTS_PUZZLE: [u8; 511] = hex!("ff02ffff01ff04ffff04ff08ffff04ff82017fff808080ffff04ffff04ff0affff04ff81bfff808080ffff04ffff04ff1cffff04ff81bfffff04ff82017fffff04ffff04ff81bfff8080ff8080808080ffff04ffff04ff14ffff04ffff0bffff0bff5effff0bff16ffff0bff16ff6eff0580ffff0bff16ffff0bff7effff0bff16ffff0bff16ff6effff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ffff30ffff30ff819fffff0bff5effff0bff16ffff0bff16ff6eff0580ffff0bff16ffff0bff7effff0bff16ffff0bff16ff6eff1780ffff0bff16ffff0bff7effff0bff16ffff0bff16ff6eff82015f80ffff0bff16ff6eff4e808080ff4e808080ff4e808080ff8202df80ff0bff808080ffff0bffff0101ff0b80808080ffff0bff16ffff0bff7effff0bff16ffff0bff16ff6eff2f80ffff0bff16ff6eff4e808080ff4e808080ff4e808080ff8080ff808080ff8080808080ffff04ffff01ffff49ff3f33ff3cff02ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff018080");

pub const VERIFICATION_PAYMENTS_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    7a56da50e9587f6fb3031919d9feaa20a2870d15eee3b70a0cc9a517f935a139
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct VerificationPaymentsArgs {
    pub singleton_mod_hash: Bytes32,
    pub launcher_puzzle_hash: Bytes32,
    pub verifier_singleton_struct_hash: Bytes32,
    pub verification_inner_puzzle_hash: Bytes32,
}

impl VerificationPaymentsArgs {
    pub fn new(
        verifier_singleton_struct_hash: Bytes32,
        verification_inner_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            singleton_mod_hash: SINGLETON_TOP_LAYER_V1_1_HASH.into(),
            launcher_puzzle_hash: SINGLETON_LAUNCHER_HASH.into(),
            verifier_singleton_struct_hash,
            verification_inner_puzzle_hash,
        }
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(solution)]
pub struct VerificationPaymentsSolution {
    pub verifier_proof: LineageProof,
    pub payout_puzzle_hash: Bytes32,
    pub my_amount: u64,
}

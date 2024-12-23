use chia::{
    clvm_utils::{CurriedProgram, TreeHash},
    protocol::Bytes32,
    puzzles::{
        singleton::{SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH},
        LineageProof,
    },
};
use chia_wallet_sdk::{DriverError, Spend, SpendContext};
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

    pub fn inner_spend(
        &self,
        ctx: &mut SpendContext,
        solution: &VerificationPaymentsSolution,
    ) -> Result<Spend, DriverError> {
        let puzzle = CurriedProgram {
            program: ctx.verification_payments_puzzle()?,
            args: VerificationPaymentsArgs::new(
                self.verifier_singleton_struct_hash,
                self.verification_inner_puzzle_hash,
            ),
        }
        .to_clvm(&mut ctx.allocator)?;

        let solution = solution.to_clvm(&mut ctx.allocator)?;

        Ok(Spend::new(puzzle, solution))
    }
}

pub const VERIFICATION_PAYMENTS_PUZZLE: [u8; 581] = hex!("ff02ffff01ff04ffff04ff10ffff04ff82017fff808080ffff04ffff04ff1cffff04ff81bfff808080ffff04ffff04ff14ffff04ff81bfffff04ff82017fffff04ffff04ff81bfff8080ff8080808080ffff04ffff04ff18ffff04ffff0bff56ffff0bff12ffff0bff12ff66ff0580ffff0bff12ffff0bff76ffff0bff12ffff0bff12ff66ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ffff30ff819fffff02ff2effff04ff02ffff04ff05ffff04ff17ffff04ff82015fff808080808080ff8202df8080ffff0bffff0101ff0b80808080ffff0bff12ffff0bff76ffff0bff12ffff0bff12ff66ff2f80ffff0bff12ff66ff46808080ff46808080ff46808080ff808080ff8080808080ffff04ffff01ffffff493fff333cffff02ff02ffff03ff05ffff01ff0bff76ffff02ff3effff04ff02ffff04ff09ffff04ffff02ff1affff04ff02ffff04ff0dff80808080ff808080808080ffff016680ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff56ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff1affff04ff02ffff04ff07ff80808080ff808080808080ff0bff12ffff0bff12ff66ff0580ffff0bff12ff0bff468080ff018080");

pub const VERIFICATION_PAYMENTS_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    8dac7372c4a705c78d900efa0883c6d5b6a51d2994ebc3788fae9434b9215bb9
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
            singleton_mod_hash: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            launcher_puzzle_hash: SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
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

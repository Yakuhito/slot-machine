use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_puzzle_types::LineageProof;
use chia_puzzles::{SINGLETON_LAUNCHER_HASH, SINGLETON_TOP_LAYER_V1_1_HASH};
use chia_wallet_sdk::driver::{DriverError, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use hex_literal::hex;

#[derive(Debug, Copy, Clone)]
#[must_use]
pub struct VerificationAsserter {
    pub verifier_singleton_struct_hash: Bytes32,
    pub verification_inner_puzzle_hash: Bytes32,
}

impl VerificationAsserter {
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
            program: VERIFICATION_ASSERTER_PUZZLE_HASH,
            args: VerificationAsserterArgs::new(
                self.verifier_singleton_struct_hash,
                self.verification_inner_puzzle_hash,
            ),
        }
        .tree_hash()
    }

    pub fn inner_spend(
        &self,
        ctx: &mut SpendContext,
        solution: &VerificationAsserterSolution,
    ) -> Result<Spend, DriverError> {
        let program = ctx.verification_asserter_puzzle()?;
        let puzzle = ctx.alloc(&CurriedProgram {
            program,
            args: VerificationAsserterArgs::new(
                self.verifier_singleton_struct_hash,
                self.verification_inner_puzzle_hash,
            ),
        })?;

        let solution = ctx.alloc(&solution)?;

        Ok(Spend::new(puzzle, solution))
    }
}

pub const VERIFICATION_ASSERTER_PUZZLE: [u8; 434] = hex!("ff02ffff01ff04ffff04ff04ffff04ffff0bffff0bff2effff0bff0affff0bff0aff36ff0580ffff0bff0affff0bff3effff0bff0affff0bff0aff36ffff0bffff0102ffff0bffff0101ff0580ffff0bffff0102ffff0bffff0101ffff30ffff30ff819fffff0bff2effff0bff0affff0bff0aff36ff0580ffff0bff0affff0bff3effff0bff0affff0bff0aff36ff1780ffff0bff0affff0bff3effff0bff0affff0bff0aff36ff82015f80ffff0bff0aff36ff26808080ff26808080ff26808080ff8201df80ff0bff81ff8080ffff0bffff0101ff0b80808080ffff0bff0affff0bff3effff0bff0affff0bff0aff36ffff02ff2fff81bf8080ffff0bff0aff36ff26808080ff26808080ff26808080ff8080ff808080ff8080ffff04ffff01ff3fff02ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff018080");

pub const VERIFICATION_ASSERTER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    eb7809960285f33123a900f3098b60b76dec74abe0484517fb4ec83071a7e945
    "
));

pub const CATALOG_VERIFICATION_MAKER_PUZZLE: [u8; 299] = hex!("ff02ffff01ff0bff16ffff0bff04ffff0bff04ff1aff0580ffff0bff04ffff0bff1effff0bff04ffff0bff04ff1affff0bffff0101ff058080ffff0bff04ffff0bff1effff0bff04ffff0bff04ff1affff0bffff0102ffff0bffff0101ff1380ffff0bffff0102ff2bffff0bffff0102ff3bffff0bffff0101ff178080808080ffff0bff04ff1aff12808080ff12808080ff12808080ffff04ffff01ff02ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff018080");

pub const CATALOG_VERIFICATION_MAKER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    0beb6422b95501474de0ce47e8c29478f263902179cf72c06ba9b63622e63885
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct VerificationAsserterArgs {
    pub singleton_mod_hash: Bytes32,
    pub launcher_puzzle_hash: Bytes32,
    pub verifier_singleton_struct_hash: Bytes32,
    pub verification_inner_puzzle_hash: Bytes32,
}

impl VerificationAsserterArgs {
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
pub struct VerificationAsserterSolution<S> {
    pub verifier_proof: LineageProof,
    pub verification_inner_puzzle_maker_solution: S,
    #[clvm(rest)]
    pub launcher_amount: u64,
}

#[derive(ToClvm, FromClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct CatalogVerificationInnerPuzzleMakerSolution {
    pub comment: String,
}

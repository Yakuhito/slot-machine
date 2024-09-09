use std::fmt::Debug;

use chia::{
    clvm_traits::{FromClvm, ToClvm},
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
    puzzles::singleton::SingletonStruct,
};
use hex_literal::hex;

pub const VERIFICATION_LAYER_PUZZLE: [u8; 576] = hex!("ff02ffff01ff02ffff03ffff09ff2fff8080ffff01ff04ffff04ff14ffff01ff808080ffff04ffff04ff08ffff04ffff0bff56ffff0bff0affff0bff0aff66ff0b80ffff0bff0affff0bff76ffff0bff0affff0bff0aff66ffff0bffff0101ff0b8080ffff0bff0affff0bff76ffff0bff0affff0bff0aff66ffff02ff1effff04ff02ffff04ff17ff8080808080ffff0bff0aff66ff46808080ff46808080ff46808080ffff01ff01808080ff808080ffff01ff04ffff04ff08ffff01ff80ff818f8080ffff04ffff04ff1cffff04ffff0112ffff04ff80ffff04ffff0bff56ffff0bff0affff0bff0aff66ff0980ffff0bff0affff0bff76ffff0bff0affff0bff0aff66ffff02ff1effff04ff02ffff04ff05ff8080808080ffff0bff0affff0bff76ffff0bff0affff0bff0aff66ff2f80ffff0bff0aff66ff46808080ff46808080ff46808080ff8080808080ff80808080ff0180ffff04ffff01ffff33ff3e43ff02ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff1effff04ff02ffff04ff09ff80808080ffff02ff1effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const VERIFICATION_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    72600e1408134c0def58ce09d1b9edce15ffcfd5f5a2ebcd421d4a47ec4518c2
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct VerificationLayerArgs1stCurry {
    pub revocation_singleton_struct: SingletonStruct,
}

impl VerificationLayerArgs1stCurry {
    pub fn curry_tree_hash(revocation_singleton_launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: VERIFICATION_LAYER_PUZZLE_HASH,
            args: VerificationLayerArgs1stCurry {
                revocation_singleton_struct: SingletonStruct::new(revocation_singleton_launcher_id),
            },
        }
        .tree_hash()
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct VerificationLayerArgs2ndCurry<T> {
    pub self_hash: Bytes32,
    pub verified_data: T,
}

impl<T> VerificationLayerArgs2ndCurry<T>
where
    T: ToTreeHash,
{
    pub fn curry_tree_hash(
        revocation_singleton_launcher_id: Bytes32,
        verified_data: T,
    ) -> TreeHash {
        let self_hash =
            VerificationLayerArgs1stCurry::curry_tree_hash(revocation_singleton_launcher_id);

        CurriedProgram {
            program: VERIFICATION_LAYER_PUZZLE_HASH,
            args: VerificationLayerArgs2ndCurry {
                self_hash: self_hash.into(),
                verified_data: verified_data.tree_hash(),
            },
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct VerificationLayerSolution {
    pub revocation_singleton_inner_puzzle_hash: Option<Bytes32>,
}

impl VerificationLayerSolution {
    pub fn oracle() -> Self {
        Self {
            revocation_singleton_inner_puzzle_hash: None,
        }
    }

    pub fn revocation(revocation_singleton_inner_puzzle_hash: Bytes32) -> Self {
        Self {
            revocation_singleton_inner_puzzle_hash: Some(revocation_singleton_inner_puzzle_hash),
        }
    }
}

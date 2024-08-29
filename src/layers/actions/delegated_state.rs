use chia::{clvm_utils::{CurriedProgram, ToTreeHash, TreeHash}, protocol::Bytes32, puzzles::singleton::SingletonStruct};
use clvm_traits::{FromClvm, ToClvm};
use hex_literal::hex;

pub const DELEGATED_STATE_ACTION_PUZZLE: [u8; 477] = hex!("ff02ffff01ff04ff27ffff04ff08ffff04ffff0112ffff04ffff02ff1effff04ff02ffff04ff27ff80808080ffff04ffff02ff1affff04ff02ffff04ff09ffff04ffff02ff1effff04ff02ffff04ff05ff80808080ffff04ff57ff808080808080ff808080808080ffff04ffff01ffff43ff02ff02ffff03ff05ffff01ff0bff72ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff1cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff0bff52ffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff1cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff14ffff0bff14ff62ff0580ffff0bff14ff0bff428080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff1effff04ff02ffff04ff09ff80808080ffff02ff1effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const  DELEGATED_STATE_ACTION_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1e5759069429397243b808748e5bd5270ea0891953ea06df9a46b87ce4ade466
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DelegatedStateActionArgs {
    pub other_singleton_struct: SingletonStruct,
}

impl DelegatedStateActionArgs {
    pub fn new(other_launcher_id: Bytes32) -> Self {
        Self {
            other_singleton_struct: SingletonStruct::new(other_launcher_id),
        }
    }
}

impl DelegatedStateActionArgs { 
    pub fn curry_tree_hash(
        other_launcher_id: Bytes32
    ) -> TreeHash {
        CurriedProgram {
            program: DELEGATED_STATE_ACTION_PUZZLE_HASH,
            args: DelegatedStateActionArgs::new(other_launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct DelegatedStateActionSolution<S> {
    pub new_state: S,
    pub other_singleton_inner_puzzle_hash: Bytes32,
}
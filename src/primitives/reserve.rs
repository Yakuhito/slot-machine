use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash, TreeHasher},
    protocol::Bytes32,
    puzzles::singleton::{SINGLETON_LAUNCHER_PUZZLE_HASH, SINGLETON_TOP_LAYER_PUZZLE_HASH},
};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

pub const RESERVE_PUZZLE: [u8; 297] = hex!("ff02ffff01ff04ffff04ff04ffff04ffff0113ffff04ff80ffff04ffff0bff2effff0bff0affff0bff0aff36ff0580ffff0bff0affff0bff3effff0bff0affff0bff0aff36ffff0bffff0102ffff0bffff0101ff0580ff0b8080ffff0bff0affff0bff3effff0bff0affff0bff0aff36ff2f80ffff0bff0aff36ff26808080ff26808080ff26808080ff8080808080ff1780ffff04ffff01ff43ff02ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff018080");

pub const RESERVE_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    a16c0d18ef30b4c82fc5ad29ea72adf5b6686f1d838b077abc6be0f17f7720ce
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(curry)]
pub struct ReserveArgs<C = NodePtr>
where
    C: FromClvm<Allocator> + ToClvm<Allocator> + Clone,
{
    pub singleton_mod: Bytes32,
    pub singleton_struct_rest: Bytes32,
    pub base_conditions: C,
}

impl<C> ReserveArgs<C>
where
    C: FromClvm<Allocator> + ToClvm<Allocator> + Clone,
{
    pub fn new(launcher_id: Bytes32, base_conditions: C) -> Self {
        let singleton_struct: (Bytes32, Bytes32) =
            (launcher_id, SINGLETON_LAUNCHER_PUZZLE_HASH.into());

        Self {
            singleton_mod: SINGLETON_TOP_LAYER_PUZZLE_HASH.into(),
            singleton_struct_rest: singleton_struct.tree_hash().into(),
            base_conditions,
        }
    }

    pub fn curry_tree_hash(launcher_id: Bytes32, base_conditions: C) -> TreeHash
    where
        C: ToClvm<TreeHasher>,
    {
        CurriedProgram {
            program: RESERVE_PUZZLE_HASH,
            args: ReserveArgs::<C>::new(launcher_id, base_conditions),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Copy, Eq)]
#[clvm(solution)]
pub struct ReserveSolution {
    pub owner_inner_puzzle_hash: Bytes32,
}

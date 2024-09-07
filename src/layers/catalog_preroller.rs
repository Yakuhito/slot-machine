use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash, TreeHasher},
    protocol::Bytes32,
    puzzles::singleton::SINGLETON_LAUNCHER_PUZZLE_HASH,
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::{SpendContextExt, UniquenessPrelauncher};

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrerollerNftInfo {
    pub eve_nft_full_puzzle_hash: Bytes32,
    #[clvm(rest)]
    pub asset_id_hash: Bytes32,
}

impl CatalogPrerollerNftInfo {
    pub fn from_asset_id(eve_nft_full_puzzle_hash: Bytes32, asset_id: Bytes32) -> Self {
        Self {
            eve_nft_full_puzzle_hash,
            asset_id_hash: asset_id.tree_hash().into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPrerollerLayer {
    pub nft_infos: Vec<CatalogPrerollerNftInfo>,
    pub base_conditions: Conditions<NodePtr>,
}

impl CatalogPrerollerLayer {
    pub fn new(
        nft_infos: Vec<CatalogPrerollerNftInfo>,
        base_conditions: Conditions<NodePtr>,
    ) -> Self {
        Self {
            nft_infos,
            base_conditions,
        }
    }
}

impl Layer for CatalogPrerollerLayer {
    type Solution = CatalogPrerollerSolution;

    fn parse_puzzle(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(puzzle) = puzzle.as_curried() else {
            return Ok(None);
        };

        if puzzle.mod_hash != CATALOG_PREROLLER_PUZZLE_HASH {
            return Ok(None);
        }

        let args = CatalogPrerollerArgs::<NodePtr>::from_clvm(allocator, puzzle.args)?;
        if args.mod_hashes != CatalogPrerollerModHashes::default() {
            return Err(DriverError::NonStandardLayer);
        }

        Ok(Some(Self {
            nft_infos: args.nft_infos,
            base_conditions: args.base_conditions,
        }))
    }

    fn parse_solution(
        allocator: &Allocator,
        solution: NodePtr,
    ) -> Result<Self::Solution, DriverError> {
        CatalogPrerollerSolution::from_clvm(allocator, solution).map_err(DriverError::FromClvm)
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        CurriedProgram {
            program: ctx.catalog_preroller_puzzle()?,
            args: CatalogPrerollerArgs::<NodePtr> {
                mod_hashes: CatalogPrerollerModHashes::default(),
                nft_infos: self.nft_infos.clone(),
                base_conditions: self.base_conditions.clone(),
            },
        }
        .to_clvm(&mut ctx.allocator)
        .map_err(DriverError::ToClvm)
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        solution
            .to_clvm(&mut ctx.allocator)
            .map_err(DriverError::ToClvm)
    }
}

impl CatalogPrerollerLayer {
    pub fn spend(self, ctx: &mut SpendContext, my_coin_id: Bytes32) -> Result<Spend, DriverError> {
        let puzzle = self.construct_puzzle(ctx)?;
        let solution = self.construct_solution(ctx, CatalogPrerollerSolution { my_coin_id })?;

        Ok(Spend { puzzle, solution })
    }
}

pub const CATALOG_PREROLLER_PUZZLE: [u8; 375] = hex!("ff02ffff01ff04ffff04ff0cffff04ff2fff808080ffff02ff16ffff04ff02ffff04ff05ffff04ff2fffff04ff0bffff04ff17ff8080808080808080ffff04ffff01ffff4046ff02ffff02ffff03ff17ffff01ff04ffff04ff08ffff04ffff30ffff30ffff30ff0bffff0bff5effff0bff0affff0bff0aff6eff0d80ffff0bff0affff0bff7effff0bff0affff0bff0aff6eff6780ffff0bff0aff6eff4e808080ff4e808080ff8080ff09ffff010180ff47ffff010180ff808080ffff02ff16ffff04ff02ffff04ff05ffff04ff0bffff04ff37ffff04ff2fff8080808080808080ffff012f80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff018080");

pub const CATALOG_PREROLLER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    f081173cc82c6940a0c0a9f35b7ae5e75ff7befa431ac97f216af94328b9a8be
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrerollerModHashes {
    pub launcher_mod_hash: Bytes32,
    #[clvm(rest)]
    pub uniqueness_prelauncher_1st_curry_hash: Bytes32,
}

impl Default for CatalogPrerollerModHashes {
    fn default() -> Self {
        Self {
            launcher_mod_hash: SINGLETON_LAUNCHER_PUZZLE_HASH.into(),
            uniqueness_prelauncher_1st_curry_hash: UniquenessPrelauncher::<()>::first_curry_hash()
                .into(),
        }
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrerollerArgs<T = NodePtr>
where
    T: FromClvm<Allocator> + ToClvm<Allocator> + Clone,
{
    pub mod_hashes: CatalogPrerollerModHashes,
    pub nft_infos: Vec<CatalogPrerollerNftInfo>,
    pub base_conditions: Conditions<T>,
}

impl<T> CatalogPrerollerArgs<T>
where
    T: FromClvm<Allocator> + ToClvm<Allocator> + Clone,
{
    pub fn new(nft_infos: Vec<CatalogPrerollerNftInfo>, base_conditions: Conditions<T>) -> Self {
        Self {
            mod_hashes: CatalogPrerollerModHashes::default(),
            nft_infos,
            base_conditions,
        }
    }

    pub fn curry_tree_hash(
        nft_infos: Vec<CatalogPrerollerNftInfo>,
        base_conditions: Conditions<T>,
    ) -> TreeHash
    where
        T: ToClvm<TreeHasher>,
    {
        CurriedProgram {
            program: CATALOG_PREROLLER_PUZZLE_HASH,
            args: CatalogPrerollerArgs::<T> {
                mod_hashes: CatalogPrerollerModHashes::default(),
                nft_infos,
                base_conditions,
            },
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct CatalogPrerollerSolution {
    pub my_coin_id: Bytes32,
}

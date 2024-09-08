use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash, TreeHasher},
    protocol::Bytes32,
};
use chia_wallet_sdk::{Conditions, DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::{SpendContextExt, UniquenessPrelauncher};

use super::NftPack;

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct CatalogPrerollerNftInfo {
    pub asset_id_hash: Bytes32,
    #[clvm(rest)]
    pub eve_nft_inner_puzzle_hash: Bytes32,
}

impl CatalogPrerollerNftInfo {
    pub fn from_asset_id(eve_nft_inner_puzzle_hash: Bytes32, asset_id: Bytes32) -> Self {
        Self {
            eve_nft_inner_puzzle_hash,
            asset_id_hash: asset_id.tree_hash().into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPrerollerLayer {
    pub nft_infos: Vec<CatalogPrerollerNftInfo>,
    pub base_conditions: Conditions<NodePtr>,
    pub royalty_address_hash: Bytes32,
    pub trade_price_percentage: u16,
}

impl CatalogPrerollerLayer {
    pub fn new(
        nft_infos: Vec<CatalogPrerollerNftInfo>,
        base_conditions: Conditions<NodePtr>,
        royalty_address_hash: Bytes32,
        trade_price_percentage: u16,
    ) -> Self {
        Self {
            nft_infos,
            base_conditions,
            royalty_address_hash,
            trade_price_percentage,
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
        if args.uniqueness_prelauncher_1st_curry_hash
            != UniquenessPrelauncher::<()>::first_curry_hash().into()
            || args.nft_pack
                != NftPack::new(
                    args.nft_pack.royalty_address_hash,
                    args.nft_pack.trade_price_percentage,
                )
        {
            return Err(DriverError::NonStandardLayer);
        }

        Ok(Some(Self {
            nft_infos: args.nft_infos,
            base_conditions: args.base_conditions,
            royalty_address_hash: args.nft_pack.royalty_address_hash,
            trade_price_percentage: args.nft_pack.trade_price_percentage,
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
                uniqueness_prelauncher_1st_curry_hash:
                    UniquenessPrelauncher::<()>::first_curry_hash().into(),
                nft_pack: NftPack::new(self.royalty_address_hash, self.trade_price_percentage),
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

pub const CATALOG_PREROLLER_PUZZLE: [u8; 810] = hex!("ff02ffff01ff04ffff04ff18ffff04ff5fff808080ffff02ff1cffff04ff02ffff04ff05ffff04ffff018a4d4f445f484153484553ffff04ff5fffff04ff17ffff04ff2fff808080808080808080ffff04ffff01ffffff4046ff02ff02ffff03ff2fffff01ff04ffff04ff10ffff04ffff02ff1affff04ff02ffff04ff0bffff04ff2fffff04ffff30ffff30ff17ffff02ff2effff04ff02ffff04ff05ffff04ff818fff8080808080ff8080ff13ffff010180ff808080808080ff808080ffff02ff1cffff04ff02ffff04ff05ffff04ff0bffff04ff17ffff04ff6fffff04ff5fff808080808080808080ffff015f80ff0180ffffff02ffff03ff05ffff01ff0bff76ffff02ff3effff04ff02ffff04ff09ffff04ffff02ff12ffff04ff02ffff04ff0dff80808080ff808080808080ffff016680ff0180ff30ff17ffff02ff2effff04ff02ffff04ff15ffff04ffff0bffff0102ffff0bffff0101ff1580ffff0bffff0102ffff0bffff0101ff1780ffff0bffff0101ff09808080ffff04ffff02ff2effff04ff02ffff04ff2dffff04ffff0bffff0101ff2d80ffff04ff46ffff04ff5dffff04ffff02ff2effff04ff02ffff04ff81bdffff04ffff0bffff0101ff81bd80ffff04ff46ffff04ffff02ff2effff04ff02ffff04ff82017dffff04ffff0bffff0102ffff0bffff0101ff1580ffff0bffff0102ffff0bffff0101ff1780ffff0bffff0101ff09808080ffff04ff8202fdffff04ffff0bffff0101ff8205fd80ff80808080808080ffff04ff1bff8080808080808080ff8080808080808080ff808080808080ffff010180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffff0bff56ffff02ff3effff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff07ff80808080ff808080808080ff0bff14ffff0bff14ff66ff0580ffff0bff14ff0bff468080ff018080");

pub const CATALOG_PREROLLER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    0372d71339fd2a9862d0dacc22a0f8e7883f1642a10f2a223a48203350fabbc4
    "
));

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(curry)]
pub struct CatalogPrerollerArgs<T = NodePtr>
where
    T: FromClvm<Allocator> + ToClvm<Allocator> + Clone,
{
    pub uniqueness_prelauncher_1st_curry_hash: Bytes32,
    pub nft_pack: NftPack,
    pub nft_infos: Vec<CatalogPrerollerNftInfo>,
    pub base_conditions: Conditions<T>,
}

impl<T> CatalogPrerollerArgs<T>
where
    T: FromClvm<Allocator> + ToClvm<Allocator> + Clone,
{
    pub fn new(
        nft_infos: Vec<CatalogPrerollerNftInfo>,
        base_conditions: Conditions<T>,
        royalty_address_hash: Bytes32,
        trade_price_percentage: u16,
    ) -> Self {
        Self {
            uniqueness_prelauncher_1st_curry_hash: UniquenessPrelauncher::<()>::first_curry_hash()
                .into(),
            nft_pack: NftPack::new(royalty_address_hash, trade_price_percentage),
            nft_infos,
            base_conditions,
        }
    }

    pub fn curry_tree_hash(
        nft_infos: Vec<CatalogPrerollerNftInfo>,
        base_conditions: Conditions<T>,
        royalty_address_hash: Bytes32,
        trade_price_percentage: u16,
    ) -> TreeHash
    where
        T: ToClvm<TreeHasher>,
    {
        CurriedProgram {
            program: CATALOG_PREROLLER_PUZZLE_HASH,
            args: CatalogPrerollerArgs::<T> {
                uniqueness_prelauncher_1st_curry_hash:
                    UniquenessPrelauncher::<()>::first_curry_hash().into(),
                nft_pack: NftPack::new(royalty_address_hash, trade_price_percentage),
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

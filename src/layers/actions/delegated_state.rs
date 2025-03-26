use chia::{
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin},
    puzzles::singleton::SingletonStruct,
};
use chia_wallet_sdk::{Conditions, DriverError, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::{
    Action, CatalogRegistry, CatalogRegistryConstants, SpendContextExt, XchandlesConstants,
    XchandlesRegistry,
};

pub struct DelegatedStateAction {
    pub other_launcher_id: Bytes32,
}

impl ToTreeHash for DelegatedStateAction {
    fn tree_hash(&self) -> TreeHash {
        DelegatedStateActionArgs::curry_tree_hash(self.other_launcher_id)
    }
}

impl Action<CatalogRegistry> for DelegatedStateAction {
    fn from_constants(constants: &CatalogRegistryConstants) -> Self {
        Self {
            other_launcher_id: constants.price_singleton_launcher_id,
        }
    }
}

impl Action<XchandlesRegistry> for DelegatedStateAction {
    fn from_constants(constants: &XchandlesConstants) -> Self {
        Self {
            other_launcher_id: constants.price_singleton_launcher_id,
        }
    }
}

impl DelegatedStateAction {
    pub fn curry_tree_hash(price_singleton_launcher_id: Bytes32) -> TreeHash {
        DelegatedStateActionArgs::curry_tree_hash(price_singleton_launcher_id)
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        Ok(CurriedProgram {
            program: ctx.delegated_state_action_puzzle()?,
            args: DelegatedStateActionArgs::new(self.other_launcher_id),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    pub fn spend<S>(
        self,
        ctx: &mut SpendContext,
        my_coin: Coin,
        new_state: S,
        other_singleton_inner_puzzle_hash: Bytes32,
    ) -> Result<(Conditions, Spend), DriverError>
    where
        S: ToClvm<Allocator>,
    {
        let state = new_state.to_clvm(&mut ctx.allocator)?;
        let my_solution = DelegatedStateActionSolution::<NodePtr> {
            new_state: state,
            other_singleton_inner_puzzle_hash,
        }
        .to_clvm(&mut ctx.allocator)?;
        let my_puzzle = self.construct_puzzle(ctx)?;

        let message: Bytes32 = ctx.tree_hash(state).into();
        let conds = Conditions::new().send_message(
            18,
            message.into(),
            vec![my_coin.puzzle_hash.to_clvm(&mut ctx.allocator)?],
        );
        Ok((conds, Spend::new(my_puzzle, my_solution)))
    }
}

pub const DELEGATED_STATE_ACTION_PUZZLE: [u8; 399] = hex!("ff02ffff01ff04ff27ffff04ffff04ff08ffff04ffff0112ffff04ffff02ff0effff04ff02ffff04ff27ff80808080ffff04ffff0bff2affff0bff0cffff0bff0cff32ff0980ffff0bff0cffff0bff3affff0bff0cffff0bff0cff32ffff02ff0effff04ff02ffff04ff05ff8080808080ffff0bff0cffff0bff3affff0bff0cffff0bff0cff32ff3780ffff0bff0cff32ff22808080ff22808080ff22808080ff8080808080ff808080ffff04ffff01ffff4302ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff0effff04ff02ffff04ff09ff80808080ffff02ff0effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");

pub const DELEGATED_STATE_ACTION_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    61d91188f5464625ec0224d47e0c18c2aa13c6f7e0d6ecbd2d928c64e068dfba
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
    pub fn curry_tree_hash(other_launcher_id: Bytes32) -> TreeHash {
        CurriedProgram {
            program: DELEGATED_STATE_ACTION_PUZZLE_HASH,
            args: DelegatedStateActionArgs::new(other_launcher_id),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct DelegatedStateActionSolution<S> {
    pub new_state: S,
    #[clvm(rest)]
    pub other_singleton_inner_puzzle_hash: Bytes32,
}

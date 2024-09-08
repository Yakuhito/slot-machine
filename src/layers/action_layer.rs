use std::fmt::Debug;

use chia::{
    clvm_traits::{FromClvm, ToClvm},
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{run_puzzle, DriverError, Layer, MerkleTree, Puzzle, Spend, SpendContext};
use clvm_traits::{clvm_list, match_tuple};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionLayer<S> {
    pub action_puzzle_hashes: Vec<Bytes32>,
    pub state: S,
}

#[derive(Debug, Clone)]
pub struct ActionLayerSolution {
    pub proofs: Vec<(u32, Vec<Bytes32>)>,
    pub action_spends: Vec<Spend>,
}

impl<S> ActionLayer<S> {
    pub fn new(action_puzzle_hashes: Vec<Bytes32>, state: S) -> Self {
        Self {
            action_puzzle_hashes,
            state,
        }
    }

    pub fn get_proofs(
        &self,
        action_spends_puzzle_hashes: &[Bytes32],
    ) -> Option<Vec<(u32, Vec<Bytes32>)>> {
        let merkle_tree = MerkleTree::new(&self.action_puzzle_hashes);

        let proofs: Vec<(u32, Vec<Bytes32>)> = action_spends_puzzle_hashes
            .iter()
            .filter_map(|puzzle_hash| {
                let proof = merkle_tree.get_proof(*puzzle_hash)?;

                Some(proof)
            })
            .collect();

        if proofs.len() != action_spends_puzzle_hashes.len() {
            return None;
        }

        Some(proofs)
    }

    pub fn extract_merkle_root_and_state(
        allocator: &Allocator,
        inner_puzzle: Puzzle,
    ) -> Result<Option<(Bytes32, S)>, DriverError>
    where
        S: FromClvm<Allocator>,
    {
        let Some(puzzle) = inner_puzzle.as_curried() else {
            return Ok(None);
        };

        if inner_puzzle.mod_hash() != ACTION_LAYER_PUZZLE_HASH {
            return Ok(None);
        }

        let args = ActionLayerArgs::<S>::from_clvm(allocator, puzzle.args)?;

        if args.my_mod_hash != ACTION_LAYER_PUZZLE_HASH.into() {
            return Ok(None);
        }

        Ok(Some((args.merkle_root, args.state)))
    }

    pub fn get_new_state(
        allocator: &mut Allocator,
        initial_state: S,
        solution: NodePtr,
    ) -> Result<S, DriverError>
    where
        S: ToClvm<Allocator> + FromClvm<Allocator>,
    {
        let solution = RawActionLayerSolution::<NodePtr, NodePtr>::from_clvm(allocator, solution)?;

        let mut state: S = initial_state;
        for raw_action in solution.actions {
            let actual_solution = clvm_list!(state.to_clvm(allocator)?, raw_action.action_solution);
            let actual_solution = actual_solution.to_clvm(allocator)?;

            let output = run_puzzle(allocator, raw_action.action_puzzle_reveal, actual_solution)?;
            (state, _) = <match_tuple!(S, NodePtr)>::from_clvm(allocator, output)?;
        }

        Ok(state)
    }
}

impl<S> Layer for ActionLayer<S>
where
    S: ToClvm<Allocator> + Clone,
{
    type Solution = ActionLayerSolution;

    /// Not available for this layer
    fn parse_puzzle(_: &Allocator, _: Puzzle) -> Result<Option<Self>, DriverError> {
        Ok(None)
    }

    fn parse_solution(
        allocator: &Allocator,
        solution: NodePtr,
    ) -> Result<Self::Solution, DriverError> {
        let solution = RawActionLayerSolution::<NodePtr, NodePtr>::from_clvm(allocator, solution)?;

        let action_spends = solution
            .actions
            .iter()
            .map(|action| Spend::new(action.action_puzzle_reveal, action.action_solution))
            .collect();

        let proofs = solution
            .actions
            .into_iter()
            .map(|action| action.action_proof.to_rust())
            .collect();

        Ok(ActionLayerSolution {
            proofs,
            action_spends,
        })
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let merkle_tree = MerkleTree::new(&self.action_puzzle_hashes);
        Ok(CurriedProgram {
            program: ctx.action_layer_puzzle()?,
            args: ActionLayerArgs::<S>::new(merkle_tree.root, self.state.clone()),
        }
        .to_clvm(&mut ctx.allocator)?)
    }

    fn construct_solution(
        &self,
        ctx: &mut SpendContext,
        solution: Self::Solution,
    ) -> Result<NodePtr, DriverError> {
        Ok(RawActionLayerSolution {
            actions: solution
                .action_spends
                .into_iter()
                .zip(solution.proofs)
                .map(|(spend, proof)| RawActionLayerSolutionItem {
                    action_proof: RawProof::from_rust(proof),
                    action_puzzle_reveal: spend.puzzle,
                    action_solution: spend.solution,
                })
                .collect(),
        }
        .to_clvm(&mut ctx.allocator)?)
    }
}

pub const ACTION_LAYER_PUZZLE: [u8; 906] = hex!("ff02ffff01ff02ff3affff04ff02ffff04ff05ffff04ff0bffff04ff17ffff04ff80ffff04ff2fff8080808080808080ffff04ffff01ffffff3302ffff02ffff03ff05ffff01ff0bff81ecffff02ff12ffff04ff02ffff04ff09ffff04ffff02ff14ffff04ff02ffff04ff0dff80808080ff808080808080ffff0181cc80ff0180ffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff0bff81acffff02ff12ffff04ff02ffff04ff05ffff04ffff02ff14ffff04ff02ffff04ff07ff80808080ff808080808080ffffff0bff18ffff0bff18ff81ccff0580ffff0bff18ff0bff818c8080ffff02ffff03ff0bffff01ff04ff13ffff02ff2affff04ff02ffff04ff05ffff04ff1bff808080808080ffff010580ff0180ff02ffff03ff5fffff01ff02ffff03ffff09ff0bffff02ff3effff04ff02ffff04ffff0bffff0101ffff02ff2effff04ff02ffff04ff82029fff8080808080ffff04ff82011fff808080808080ffff01ff02ff16ffff04ff02ffff04ff05ffff04ff0bffff04ffff02ff82029fffff04ff17ffff04ff82039fff80808080ffff04ff81dfffff04ff2fff8080808080808080ffff01ff088080ff0180ffff01ff04ffff04ff10ffff04ffff02ff3cffff04ff02ffff04ff05ffff04ffff0bffff0101ff0580ffff04ffff0bffff0101ff0b80ffff04ffff02ff2effff04ff02ffff04ff17ff80808080ff80808080808080ffff01ff01808080ff2f8080ff0180ffff02ff3affff04ff02ffff04ff05ffff04ff0bffff04ff27ffff04ffff02ff2affff04ff02ffff04ff5fffff04ff37ff8080808080ffff04ff2fff8080808080808080ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff2effff04ff02ffff04ff09ff80808080ffff02ff2effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff02ffff03ff1bffff01ff02ff3effff04ff02ffff04ffff02ffff03ffff18ffff0101ff1380ffff01ff0bffff0102ff2bff0580ffff01ff0bffff0102ff05ff2b8080ff0180ffff04ffff04ffff17ff13ffff0181ff80ff3b80ff8080808080ffff010580ff0180ff018080");

pub const ACTION_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    d15bd225cd87db7d17f7b707a9706980301776474f2925c3b1e91908fae37d72
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct ActionLayerArgs<S> {
    pub my_mod_hash: Bytes32,
    pub merkle_root: Bytes32,
    pub state: S,
}

impl<S> ActionLayerArgs<S> {
    pub fn new(merkle_root: Bytes32, state: S) -> Self {
        Self {
            my_mod_hash: ACTION_LAYER_PUZZLE_HASH.into(),
            merkle_root,
            state,
        }
    }
}

impl ActionLayerArgs<TreeHash> {
    pub fn curry_tree_hash(merkle_root: Bytes32, state_hash: TreeHash) -> TreeHash {
        CurriedProgram {
            program: ACTION_LAYER_PUZZLE_HASH,
            args: ActionLayerArgs::<TreeHash>::new(merkle_root, state_hash),
        }
        .tree_hash()
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct RawProof {
    pub path: u32,
    #[clvm(rest)]
    pub hashes: Vec<Bytes32>,
}

impl RawProof {
    pub fn to_rust(&self) -> (u32, Vec<Bytes32>) {
        (self.path, self.hashes.clone())
    }

    pub fn from_rust(proof: (u32, Vec<Bytes32>)) -> Self {
        Self {
            path: proof.0,
            hashes: proof.1,
        }
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct RawActionLayerSolutionItem<P, S> {
    pub action_proof: RawProof,
    pub action_puzzle_reveal: P,
    #[clvm(rest)]
    pub action_solution: S,
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(solution)]
pub struct RawActionLayerSolution<P, S> {
    pub actions: Vec<RawActionLayerSolutionItem<P, S>>,
}

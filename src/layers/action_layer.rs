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
    pub merkle_root: Bytes32,
    pub state: S,
    pub hint: Bytes32,
}

#[derive(Debug, Clone)]
pub struct ActionLayerSolution {
    pub proofs: Vec<(u32, Vec<Bytes32>)>,
    pub action_spends: Vec<Spend>,
}

impl<S> ActionLayer<S> {
    pub fn new(merkle_root: Bytes32, state: S, hint: Bytes32) -> Self {
        Self {
            merkle_root,
            state,
            hint,
        }
    }

    pub fn from_action_puzzle_hashes(leaves: &[Bytes32], state: S, hint: Bytes32) -> Self {
        let merkle_root = MerkleTree::new(leaves).root;

        Self {
            merkle_root,
            state,
            hint,
        }
    }

    pub fn get_proofs(
        &self,
        action_puzzle_hashes: &[Bytes32],
        action_spends_puzzle_hashes: &[Bytes32],
    ) -> Option<Vec<(u32, Vec<Bytes32>)>> {
        let merkle_tree = MerkleTree::new(action_puzzle_hashes);

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

        let args = ActionLayerArgs::<NodePtr, S>::from_clvm(allocator, puzzle.args)?;

        let finalizer = Puzzle::parse(allocator, args.finalizer);
        let Some(finalizer) = finalizer.as_curried() else {
            return Ok(None);
        };

        let finalizer_args = DefaultFinalizerArgs::from_clvm(allocator, finalizer.args)?;
        if finalizer.mod_hash != DEFAULT_FINALIZER_PUZZLE_HASH
            || finalizer_args.finalizer_mod_hash != DEFAULT_FINALIZER_PUZZLE_HASH.into()
            || finalizer_args.action_layer_mod_hash != ACTION_LAYER_PUZZLE_HASH.into()
        {
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
        let solution =
            RawActionLayerSolution::<NodePtr, NodePtr, NodePtr>::from_clvm(allocator, solution)?;

        let mut state: S = initial_state;
        for raw_action in solution.actions {
            let actual_solution =
                clvm_list!(state, raw_action.action_solution).to_clvm(allocator)?;

            let output = run_puzzle(allocator, raw_action.action_puzzle_reveal, actual_solution)?;
            (state, _) = <match_tuple!(S, NodePtr)>::from_clvm(allocator, output)?;
        }

        Ok(state)
    }
}

impl<S> Layer for ActionLayer<S>
where
    S: ToClvm<Allocator> + FromClvm<Allocator> + Clone,
{
    type Solution = ActionLayerSolution;

    fn parse_puzzle(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(puzzle) = puzzle.as_curried() else {
            return Ok(None);
        };

        if puzzle.mod_hash != ACTION_LAYER_PUZZLE_HASH {
            return Ok(None);
        }

        let args = ActionLayerArgs::<NodePtr, S>::from_clvm(allocator, puzzle.args)?;
        let finalizer = Puzzle::parse(allocator, args.finalizer);
        let Some(finalizer) = finalizer.as_curried() else {
            return Ok(None);
        };

        let finalizer_args = DefaultFinalizerArgs::from_clvm(allocator, finalizer.args)?;
        if finalizer.mod_hash != DEFAULT_FINALIZER_PUZZLE_HASH
            || finalizer_args.finalizer_mod_hash != DEFAULT_FINALIZER_PUZZLE_HASH.into()
            || finalizer_args.action_layer_mod_hash != ACTION_LAYER_PUZZLE_HASH.into()
        {
            return Err(DriverError::NonStandardLayer);
        }

        Ok(Some(Self {
            merkle_root: args.merkle_root,
            state: args.state,
            hint: finalizer_args.hint,
        }))
    }

    fn parse_solution(
        allocator: &Allocator,
        solution: NodePtr,
    ) -> Result<Self::Solution, DriverError> {
        let solution =
            RawActionLayerSolution::<NodePtr, NodePtr, NodePtr>::from_clvm(allocator, solution)?;

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
        let finalizer = CurriedProgram {
            program: ctx.default_finalizer_puzzle()?,
            args: DefaultFinalizerArgs::new(self.hint),
        }
        .to_clvm(&mut ctx.allocator)?;

        Ok(CurriedProgram {
            program: ctx.action_layer_puzzle()?,
            args: ActionLayerArgs::<NodePtr, S>::new(
                finalizer,
                self.merkle_root,
                self.state.clone(),
            ),
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
            finalizer_solution: (),
        }
        .to_clvm(&mut ctx.allocator)?)
    }
}

pub const DEFAULT_FINALIZER_PUZZLE: [u8; 639] = hex!("ff02ffff01ff04ffff04ff10ffff04ffff02ff12ffff04ff02ffff04ff0bffff04ffff02ff12ffff04ff02ffff04ff05ffff04ffff0bffff0101ff0580ffff04ffff0bffff0101ff0b80ffff04ffff0bffff0101ff1780ff80808080808080ffff04ffff0bffff0101ff2f80ffff04ffff02ff1effff04ff02ffff04ff82013fff80808080ff80808080808080ffff04ffff0101ffff04ff17ff8080808080ffff02ff1affff04ff02ffff04ff8201bfff8080808080ffff04ffff01ffffff3302ffff02ffff03ff05ffff01ff0bff7cffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff14ffff04ff02ffff04ff0dff80808080ff808080808080ffff016c80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff0bff5cffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff14ffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ff09ffff01ff04ff11ffff02ff1affff04ff02ffff04ffff04ff19ff0d80ff8080808080ffff01ff02ffff03ff0dffff01ff02ff1affff04ff02ffff04ff0dff80808080ff8080ff018080ff0180ffff0bff18ffff0bff18ff6cff0580ffff0bff18ff0bff4c8080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff1effff04ff02ffff04ff09ff80808080ffff02ff1effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");
pub const DEFAULT_FINALIZER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    f5eec01bcf283ea04420b37e8e25a68993ccc0c6c8a0f3378fb8982156f3ff30
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DefaultFinalizerArgs {
    pub finalizer_mod_hash: Bytes32,
    pub action_layer_mod_hash: Bytes32,
    pub hint: Bytes32,
}

impl DefaultFinalizerArgs {
    pub fn new(hint: Bytes32) -> Self {
        Self {
            finalizer_mod_hash: DEFAULT_FINALIZER_PUZZLE_HASH.into(),
            action_layer_mod_hash: ACTION_LAYER_PUZZLE_HASH.into(),
            hint,
        }
    }

    pub fn curry_tree_hash(hint: Bytes32) -> TreeHash {
        CurriedProgram {
            program: DEFAULT_FINALIZER_PUZZLE_HASH,
            args: DefaultFinalizerArgs::new(hint),
        }
        .tree_hash()
    }
}

pub const ACTION_LAYER_PUZZLE: [u8; 455] = hex!("ff02ffff01ff02ff05ffff04ff0bffff04ff17ffff04ffff02ff0cffff04ff02ffff04ff0bffff04ffff04ff17ff8080ffff04ff2fff808080808080ffff04ff5fff808080808080ffff04ffff01ffffff04ff13ffff04ff05ff1b8080ff02ffff03ff17ffff01ff02ffff03ffff09ff05ffff02ff0effff04ff02ffff04ffff0bffff0101ffff02ff0affff04ff02ffff04ff81a7ff8080808080ffff04ff47ff808080808080ffff01ff02ff08ffff04ff02ffff04ff1bffff04ffff02ff0cffff04ff02ffff04ff05ffff04ffff02ff81a7ffff04ff13ffff04ff81e7ff80808080ffff04ff37ff808080808080ff8080808080ffff01ff088080ff0180ffff01ff04ff13ff1b8080ff0180ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff0affff04ff02ffff04ff09ff80808080ffff02ff0affff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff02ffff03ff1bffff01ff02ff0effff04ff02ffff04ffff02ffff03ffff18ffff0101ff1380ffff01ff0bffff0102ff2bff0580ffff01ff0bffff0102ff05ff2b8080ff0180ffff04ffff04ffff17ff13ffff0181ff80ff3b80ff8080808080ffff010580ff0180ff018080");
pub const ACTION_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    1cbe9c6df9d92135235e26e1882d414a56fe62e63d92a98793bd8de95473ff33
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct ActionLayerArgs<F, S> {
    pub finalizer: F,
    pub merkle_root: Bytes32,
    pub state: S,
}

impl<F, S> ActionLayerArgs<F, S> {
    pub fn new(finalizer: F, merkle_root: Bytes32, state: S) -> Self {
        Self {
            finalizer,
            merkle_root,
            state,
        }
    }
}

impl ActionLayerArgs<TreeHash, TreeHash> {
    pub fn curry_tree_hash(
        finalizer: TreeHash,
        merkle_root: Bytes32,
        state_hash: TreeHash,
    ) -> TreeHash {
        CurriedProgram {
            program: ACTION_LAYER_PUZZLE_HASH,
            args: ActionLayerArgs::<TreeHash, TreeHash>::new(finalizer, merkle_root, state_hash),
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
pub struct RawActionLayerSolution<P, S, F> {
    pub actions: Vec<RawActionLayerSolutionItem<P, S>>,
    pub finalizer_solution: F,
}

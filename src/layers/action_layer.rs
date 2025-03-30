use std::fmt::Debug;

use chia::{
    clvm_traits::{FromClvm, ToClvm},
    clvm_utils::{CurriedProgram, ToTreeHash, TreeHash},
    protocol::Bytes32,
};
use chia_wallet_sdk::{
    run_puzzle, DriverError, Layer, MerkleProof, MerkleTree, Puzzle, Spend, SpendContext,
};
use clvm_traits::{clvm_list, match_tuple};
use clvmr::{Allocator, NodePtr};
use hex_literal::hex;

use crate::SpendContextExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finalizer {
    Default {
        hint: Bytes32,
    },
    Reserve {
        hint: Bytes32,
        reserve_full_puzzle_hash: Bytes32,
        reserve_inner_puzzle_hash: Bytes32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionLayer<S> {
    pub merkle_root: Bytes32,
    pub state: S,
    pub finalizer: Finalizer,
}

#[derive(Debug, Clone)]
pub struct ActionLayerSolution<F> {
    pub proofs: Vec<MerkleProof>,
    pub action_spends: Vec<Spend>,
    pub finalizer_solution: F,
}

impl<S> ActionLayer<S> {
    pub fn new(merkle_root: Bytes32, state: S, finalizer: Finalizer) -> Self {
        Self {
            merkle_root,
            state,
            finalizer,
        }
    }

    pub fn from_action_puzzle_hashes(leaves: &[Bytes32], state: S, finalizer: Finalizer) -> Self {
        let merkle_root = MerkleTree::new(leaves).root();

        Self {
            merkle_root,
            state,
            finalizer,
        }
    }

    pub fn get_proofs(
        &self,
        action_puzzle_hashes: &[Bytes32],
        action_spends_puzzle_hashes: &[Bytes32],
    ) -> Option<Vec<MerkleProof>> {
        let merkle_tree = MerkleTree::new(action_puzzle_hashes);

        let proofs: Vec<MerkleProof> = action_spends_puzzle_hashes
            .iter()
            .filter_map(|puzzle_hash| {
                let proof = merkle_tree.proof(*puzzle_hash)?;

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

        Ok(Some((args.merkle_root, args.state)))
    }

    pub fn get_new_state(
        allocator: &mut Allocator,
        initial_state: S,
        action_layer_solution: NodePtr,
    ) -> Result<S, DriverError>
    where
        S: ToClvm<Allocator> + FromClvm<Allocator>,
    {
        let solution = RawActionLayerSolution::<NodePtr, NodePtr, NodePtr>::from_clvm(
            allocator,
            action_layer_solution,
        )?;

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
    type Solution = ActionLayerSolution<NodePtr>;

    fn parse_puzzle(allocator: &Allocator, puzzle: Puzzle) -> Result<Option<Self>, DriverError> {
        let Some(puzzle) = puzzle.as_curried() else {
            return Ok(None);
        };

        if puzzle.mod_hash != ACTION_LAYER_PUZZLE_HASH {
            return Ok(None);
        }

        let args = ActionLayerArgs::<NodePtr, S>::from_clvm(allocator, puzzle.args)?;
        let finalizer_2nd_curry =
            CurriedProgram::<NodePtr, NodePtr>::from_clvm(allocator, args.finalizer);
        let Ok(finalizer_2nd_curry) = finalizer_2nd_curry else {
            return Ok(None);
        };

        let finalizer_1st_curry = Puzzle::from_clvm(allocator, finalizer_2nd_curry.program)?;
        let Some(finalizer_1st_curry) = finalizer_1st_curry.as_curried() else {
            return Ok(None);
        };

        match finalizer_1st_curry.mod_hash {
            DEFAULT_FINALIZER_PUZZLE_HASH => {
                let finalizer_2nd_curry_args =
                    DefaultFinalizer2ndCurryArgs::from_clvm(allocator, finalizer_2nd_curry.args)?;
                let finalizer_1st_curry_args =
                    DefaultFinalizer1stCurryArgs::from_clvm(allocator, finalizer_1st_curry.args)?;

                if finalizer_1st_curry.mod_hash != DEFAULT_FINALIZER_PUZZLE_HASH
                    || finalizer_1st_curry_args.action_layer_mod_hash
                        != ACTION_LAYER_PUZZLE_HASH.into()
                    || finalizer_2nd_curry_args.finalizer_self_hash
                        != DefaultFinalizer1stCurryArgs::curry_tree_hash(
                            finalizer_1st_curry_args.hint,
                        )
                        .into()
                {
                    return Err(DriverError::NonStandardLayer);
                }

                Ok(Some(Self {
                    merkle_root: args.merkle_root,
                    state: args.state,
                    finalizer: Finalizer::Default {
                        hint: finalizer_1st_curry_args.hint,
                    },
                }))
            }
            RESERVE_FINALIZER_PUZZLE_HASH => {
                let finalizer_2nd_curry_args =
                    ReserveFinalizer2ndCurryArgs::from_clvm(allocator, finalizer_2nd_curry.args)?;
                let finalizer_1st_curry_args =
                    ReserveFinalizer1stCurryArgs::from_clvm(allocator, finalizer_1st_curry.args)?;

                if finalizer_1st_curry.mod_hash != RESERVE_FINALIZER_PUZZLE_HASH
                    || finalizer_1st_curry_args.action_layer_mod_hash
                        != ACTION_LAYER_PUZZLE_HASH.into()
                    || finalizer_2nd_curry_args.finalizer_self_hash
                        != ReserveFinalizer1stCurryArgs::curry_tree_hash(
                            finalizer_1st_curry_args.reserve_full_puzzle_hash,
                            finalizer_1st_curry_args.reserve_inner_puzzle_hash,
                            finalizer_1st_curry_args.hint,
                        )
                        .into()
                {
                    return Err(DriverError::NonStandardLayer);
                }

                Ok(Some(Self {
                    merkle_root: args.merkle_root,
                    state: args.state,
                    finalizer: Finalizer::Reserve {
                        hint: finalizer_1st_curry_args.hint,
                        reserve_full_puzzle_hash: finalizer_1st_curry_args.reserve_full_puzzle_hash,
                        reserve_inner_puzzle_hash: finalizer_1st_curry_args
                            .reserve_inner_puzzle_hash,
                    },
                }))
            }
            _ => Err(DriverError::NonStandardLayer),
        }
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
            .map(|action| action.action_proof)
            .collect();

        Ok(ActionLayerSolution {
            proofs,
            action_spends,
            finalizer_solution: solution.finalizer_solution,
        })
    }

    fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let finalizer_1st_curry = match self.finalizer {
            Finalizer::Default { hint } => CurriedProgram {
                program: ctx.default_finalizer_puzzle()?,
                args: DefaultFinalizer1stCurryArgs::new(hint),
            }
            .to_clvm(&mut ctx.allocator)?,
            Finalizer::Reserve {
                hint,
                reserve_full_puzzle_hash,
                reserve_inner_puzzle_hash,
            } => CurriedProgram {
                program: ctx.reserve_finalizer_puzzle()?,
                args: ReserveFinalizer1stCurryArgs::new(
                    hint,
                    reserve_full_puzzle_hash,
                    reserve_inner_puzzle_hash,
                ),
            }
            .to_clvm(&mut ctx.allocator)?,
        };

        let finalizer = match self.finalizer {
            Finalizer::Default { hint } => CurriedProgram {
                program: finalizer_1st_curry,
                args: DefaultFinalizer2ndCurryArgs::new(hint),
            }
            .to_clvm(&mut ctx.allocator)?,
            Finalizer::Reserve {
                hint,
                reserve_full_puzzle_hash,
                reserve_inner_puzzle_hash,
            } => CurriedProgram {
                program: finalizer_1st_curry,
                args: ReserveFinalizer2ndCurryArgs::new(
                    reserve_full_puzzle_hash,
                    reserve_inner_puzzle_hash,
                    hint,
                ),
            }
            .to_clvm(&mut ctx.allocator)?,
        };

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
                    action_proof: proof,
                    action_puzzle_reveal: spend.puzzle,
                    action_solution: spend.solution,
                })
                .collect(),
            finalizer_solution: solution.finalizer_solution,
        }
        .to_clvm(&mut ctx.allocator)?)
    }
}

pub const DEFAULT_FINALIZER_PUZZLE: [u8; 617] = hex!("ff02ffff01ff04ffff04ff10ffff04ffff02ff12ffff04ff02ffff04ff05ffff04ffff02ff12ffff04ff02ffff04ff17ffff04ffff0bffff0101ff1780ff8080808080ffff04ffff0bffff0101ff2f80ffff04ffff02ff1effff04ff02ffff04ff82013fff80808080ff80808080808080ffff04ffff0101ffff04ffff04ff0bff8080ff8080808080ffff02ff1affff04ff02ffff04ff8201bfff8080808080ffff04ffff01ffffff3302ffff02ffff03ff05ffff01ff0bff7cffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff14ffff04ff02ffff04ff0dff80808080ff808080808080ffff016c80ff0180ffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ffffff0bff5cffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff14ffff04ff02ffff04ff07ff80808080ff808080808080ff02ffff03ff09ffff01ff04ff11ffff02ff1affff04ff02ffff04ffff04ff19ff0d80ff8080808080ffff01ff02ffff03ff0dffff01ff02ff1affff04ff02ffff04ff0dff80808080ff8080ff018080ff0180ffff0bff18ffff0bff18ff6cff0580ffff0bff18ff0bff4c8080ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff1effff04ff02ffff04ff09ff80808080ffff02ff1effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");
pub const DEFAULT_FINALIZER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    bccb34ebcecbc3ff9348ed8089a7118695d6b24b65ecfcff80c78b9a15f548db
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DefaultFinalizer1stCurryArgs {
    pub action_layer_mod_hash: Bytes32,
    pub hint: Bytes32,
}

impl DefaultFinalizer1stCurryArgs {
    pub fn new(hint: Bytes32) -> Self {
        Self {
            action_layer_mod_hash: ACTION_LAYER_PUZZLE_HASH.into(),
            hint,
        }
    }

    pub fn curry_tree_hash(hint: Bytes32) -> TreeHash {
        CurriedProgram {
            program: DEFAULT_FINALIZER_PUZZLE_HASH,
            args: DefaultFinalizer1stCurryArgs::new(hint),
        }
        .tree_hash()
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct DefaultFinalizer2ndCurryArgs {
    pub finalizer_self_hash: Bytes32,
}

impl DefaultFinalizer2ndCurryArgs {
    pub fn new(hint: Bytes32) -> Self {
        Self {
            finalizer_self_hash: DefaultFinalizer1stCurryArgs::curry_tree_hash(hint).into(),
        }
    }

    pub fn curry_tree_hash(hint: Bytes32) -> TreeHash {
        let self_hash: TreeHash = DefaultFinalizer1stCurryArgs::curry_tree_hash(hint);

        CurriedProgram {
            program: self_hash,
            args: DefaultFinalizer2ndCurryArgs {
                finalizer_self_hash: self_hash.into(),
            },
        }
        .tree_hash()
    }
}

pub const RESERVE_FINALIZER_PUZZLE: [u8; 863] = hex!("ff02ffff01ff04ffff04ff10ffff04ffff02ff1affff04ff02ffff04ff05ffff04ffff02ff1affff04ff02ffff04ff5fffff04ffff0bffff0101ff5f80ff8080808080ffff04ffff0bffff0101ff81bf80ffff04ffff02ff3effff04ff02ffff04ff8204ffff80808080ff80808080808080ffff04ffff0101ffff04ff2fff8080808080ffff04ffff04ff18ffff04ffff0117ffff04ffff02ff3effff04ff02ffff04ffff04ffff0101ffff04ffff04ff10ffff04ff17ffff04ff8208ffffff04ffff04ff17ff8080ff8080808080ffff06ffff02ff2effff04ff02ffff04ff8206ffffff01ff80ff8080808080808080ff80808080ffff04ffff30ff8209ffff0bff82027f80ff8080808080ffff05ffff02ff2effff04ff02ffff04ff8206ffffff01ff80ff8080808080808080ffff04ffff01ffffff3342ff02ff02ffff03ff05ffff01ff0bff72ffff02ff16ffff04ff02ffff04ff09ffff04ffff02ff1cffff04ff02ffff04ff0dff80808080ff808080808080ffff016280ff0180ffffffffa04bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459aa09dcf97a184f32623d11a73124ceb99a5709b083721e878a16d78f596718ba7b2ffa102a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222a102a8d5dd63fba471ebcb1f3e8f7c1e1879b7152a6e7298a91ce119a63400ade7c5ff0bff52ffff02ff16ffff04ff02ffff04ff05ffff04ffff02ff1cffff04ff02ffff04ff07ff80808080ff808080808080ffff0bff14ffff0bff14ff62ff0580ffff0bff14ff0bff428080ffff02ffff03ff09ffff01ff02ffff03ffff09ff21ffff0181d680ffff01ff02ff2effff04ff02ffff04ffff04ff19ff0d80ffff04ff0bffff04ffff04ff31ff1780ff808080808080ffff01ff02ff2effff04ff02ffff04ffff04ff19ff0d80ffff04ffff04ff11ff0b80ffff04ff17ff80808080808080ff0180ffff01ff02ffff03ff0dffff01ff02ff2effff04ff02ffff04ff0dffff04ff0bffff04ff17ff808080808080ffff01ff04ff0bff178080ff018080ff0180ff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff3effff04ff02ffff04ff09ff80808080ffff02ff3effff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff018080");
pub const RESERVE_FINALIZER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    7b68ae49ea8f312a8dc923b15e2f1cacd7f1292c8368d07f4eb6fb77e4fab37c
    "
));

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct ReserveFinalizer1stCurryArgs {
    pub action_layer_mod_hash: Bytes32,
    pub reserve_full_puzzle_hash: Bytes32,
    pub reserve_inner_puzzle_hash: Bytes32,
    pub hint: Bytes32,
}

impl ReserveFinalizer1stCurryArgs {
    pub fn new(
        hint: Bytes32,
        reserve_full_puzzle_hash: Bytes32,
        reserve_inner_puzzle_hash: Bytes32,
    ) -> Self {
        Self {
            action_layer_mod_hash: ACTION_LAYER_PUZZLE_HASH.into(),
            reserve_full_puzzle_hash,
            reserve_inner_puzzle_hash,
            hint,
        }
    }

    pub fn curry_tree_hash(
        reserve_full_puzzle_hash: Bytes32,
        reserve_inner_puzzle_hash: Bytes32,
        hint: Bytes32,
    ) -> TreeHash {
        CurriedProgram {
            program: RESERVE_FINALIZER_PUZZLE_HASH,
            args: ReserveFinalizer1stCurryArgs::new(
                hint,
                reserve_full_puzzle_hash,
                reserve_inner_puzzle_hash,
            ),
        }
        .tree_hash()
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct ReserveFinalizer2ndCurryArgs {
    pub finalizer_self_hash: Bytes32,
}

impl ReserveFinalizer2ndCurryArgs {
    pub fn new(
        reserve_full_puzzle_hash: Bytes32,
        reserve_inner_puzzle_hash: Bytes32,
        hint: Bytes32,
    ) -> Self {
        Self {
            finalizer_self_hash: ReserveFinalizer1stCurryArgs::curry_tree_hash(
                reserve_full_puzzle_hash,
                reserve_inner_puzzle_hash,
                hint,
            )
            .into(),
        }
    }

    pub fn curry_tree_hash(
        reserve_full_puzzle_hash: Bytes32,
        reserve_inner_puzzle_hash: Bytes32,
        hint: Bytes32,
    ) -> TreeHash {
        let self_hash: TreeHash = ReserveFinalizer1stCurryArgs::curry_tree_hash(
            reserve_full_puzzle_hash,
            reserve_inner_puzzle_hash,
            hint,
        );

        CurriedProgram {
            program: self_hash,
            args: ReserveFinalizer2ndCurryArgs {
                finalizer_self_hash: self_hash.into(),
            },
        }
        .tree_hash()
    }
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(solution)]
pub struct ReserveFinalizerSolution {
    pub reserve_parent_id: Bytes32,
}

pub const ACTION_LAYER_PUZZLE: [u8; 445] = hex!("ff02ffff01ff02ff05ffff04ff0bffff04ff17ffff04ffff02ff04ffff04ff02ffff04ff0bffff04ff80ffff04ffff04ff17ff8080ffff04ff2fff80808080808080ffff04ff5fff808080808080ffff04ffff01ffff02ffff03ff2fffff01ff02ffff03ffff09ff05ffff02ff0effff04ff02ffff04ffff0bffff0101ffff02ff0affff04ff02ffff04ff82014fff8080808080ffff04ff818fff808080808080ffff01ff02ff04ffff04ff02ffff04ff05ffff04ffff04ff37ff0b80ffff04ffff02ff82014fffff04ff27ffff04ff8201cfff80808080ffff04ff6fff80808080808080ffff01ff088080ff0180ffff01ff04ff27ffff04ff37ff0b808080ff0180ffff02ffff03ffff07ff0580ffff01ff0bffff0102ffff02ff0affff04ff02ffff04ff09ff80808080ffff02ff0affff04ff02ffff04ff0dff8080808080ffff01ff0bffff0101ff058080ff0180ff02ffff03ff1bffff01ff02ff0effff04ff02ffff04ffff02ffff03ffff18ffff0101ff1380ffff01ff0bffff0102ff2bff0580ffff01ff0bffff0102ff05ff2b8080ff0180ffff04ffff04ffff17ff13ffff0181ff80ff3b80ff8080808080ffff010580ff0180ff018080");
pub const ACTION_LAYER_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "
    ff2e27152258d326dd344e58270d2d4a17253537ff3db6b6d0b2be979d5e7dbf
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
pub struct RawActionLayerSolutionItem<P, S> {
    pub action_proof: MerkleProof,
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

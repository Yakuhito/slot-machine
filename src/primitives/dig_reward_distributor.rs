use chia::{
    clvm_utils::ToTreeHash,
    protocol::{Bytes32, Coin},
    puzzles::{
        singleton::{SingletonSolution, SingletonStruct},
        LineageProof, Proof,
    },
};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    ActionLayer, ActionLayerSolution, DigAddIncentivesAction, DigAddIncentivesActionSolution,
    DigAddMirrorAction, DigAddMirrorActionSolution, DigCommitIncentivesAction,
    DigCommitIncentivesActionSolution, DigInitiatePayoutAction, DigInitiatePayoutActionSolution,
    DigNewEpochAction, DigNewEpochActionSolution, DigRemoveMirrorAction,
    DigRemoveMirrorActionSolution, DigSyncAction, DigSyncActionSolution,
    DigWithdrawIncentivesAction, DigWithdrawIncentivesActionSolution, RawActionLayerSolution,
    ReserveFinalizerSolution,
};

use super::{
    DigRewardDistributorConstants, DigRewardDistributorInfo, DigRewardDistributorState, Reserve,
};

#[derive(Debug, Clone)]
#[must_use]
pub struct DigRewardDistributor {
    pub coin: Coin,
    pub proof: Proof,
    pub info: DigRewardDistributorInfo,
}

impl DigRewardDistributor {
    pub fn new(coin: Coin, proof: Proof, info: DigRewardDistributorInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: DigRewardDistributorConstants,
    ) -> Result<Option<(Self, Reserve)>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) =
            DigRewardDistributorInfo::parse(allocator, parent_puzzle, constants)?
        else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let parent_solution = SingletonSolution::<NodePtr>::from_clvm(allocator, parent_solution)?;
        let new_state = ActionLayer::<DigRewardDistributorState>::get_new_state(
            allocator,
            parent_info.state,
            parent_solution.inner_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        let parent_inner_solution = RawActionLayerSolution::<
            NodePtr,
            NodePtr,
            ReserveFinalizerSolution,
        >::from_clvm(allocator, parent_solution.inner_solution)?;
        let parent_reserve = Coin::new(
            parent_inner_solution.finalizer_solution.reserve_parent_id,
            constants.reserve_full_puzzle_hash,
            parent_info.state.total_reserves,
        );
        let reserve = Reserve::new(
            parent_reserve.coin_id(),
            LineageProof {
                parent_parent_coin_info: parent_reserve.parent_coin_info,
                parent_inner_puzzle_hash: constants.reserve_inner_puzzle_hash,
                parent_amount: parent_reserve.amount,
            },
            constants.reserve_asset_id,
            SingletonStruct::new(parent_info.launcher_id)
                .tree_hash()
                .into(),
            0,
            new_state.total_reserves,
        );

        Ok(Some((
            DigRewardDistributor {
                coin: new_coin,
                proof,
                info: new_info,
            },
            reserve,
        )))
    }
}

#[allow(clippy::large_enum_variant)]
pub enum DigRewardDistributorAction {
    AddIncentives(DigAddIncentivesActionSolution),
    AddMirror(DigAddMirrorActionSolution),
    CommitIncentives(DigCommitIncentivesActionSolution),
    InitiatePayout(DigInitiatePayoutActionSolution),
    NewEpoch(DigNewEpochActionSolution),
    RemoveMirror(DigRemoveMirrorActionSolution),
    Sync(DigSyncActionSolution),
    WithdrawIncentives(DigWithdrawIncentivesActionSolution),
}

impl DigRewardDistributor {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        reserve_parent_id: Bytes32,
        actions: Vec<DigRewardDistributorAction>,
    ) -> Result<Spend, DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let action_spends: Vec<Spend> = actions
            .into_iter()
            .map(|action| match action {
                DigRewardDistributorAction::AddIncentives(solution) => {
                    let layer = DigAddIncentivesAction {};

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::AddMirror(solution) => {
                    let layer = DigAddMirrorAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::CommitIncentives(solution) => {
                    let layer = DigCommitIncentivesAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::InitiatePayout(solution) => {
                    let layer = DigInitiatePayoutAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::NewEpoch(solution) => {
                    let layer = DigNewEpochAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::RemoveMirror(solution) => {
                    let layer = DigRemoveMirrorAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::Sync(solution) => {
                    let layer = DigSyncAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
                DigRewardDistributorAction::WithdrawIncentives(solution) => {
                    let layer = DigWithdrawIncentivesAction::from_info(&self.info);

                    let puzzle = layer.construct_puzzle(ctx)?;
                    let solution = layer.construct_solution(ctx, solution)?;

                    Ok::<Spend, DriverError>(Spend::new(puzzle, solution))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let action_puzzle_hashes = action_spends
            .iter()
            .map(|a| ctx.tree_hash(a.puzzle).into())
            .collect::<Vec<Bytes32>>();

        let finalizer_solution =
            ReserveFinalizerSolution { reserve_parent_id }.to_clvm(&mut ctx.allocator)?;

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(
                            &DigRewardDistributorInfo::action_puzzle_hashes(
                                self.info.launcher_id,
                                &self.info.constants,
                            ),
                            &action_puzzle_hashes,
                        )
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends,
                    finalizer_solution,
                },
            },
        )?;

        Ok(Spend::new(puzzle, solution))
    }
}

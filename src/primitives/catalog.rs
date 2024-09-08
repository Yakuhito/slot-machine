use chia::{
    protocol::{Bytes32, Coin},
    puzzles::{singleton::SingletonSolution, LineageProof, Proof},
};
use chia_wallet_sdk::{DriverError, Layer, Puzzle, Spend, SpendContext};
use clvm_traits::ToClvm;
use clvmr::{Allocator, NodePtr};
use hex::encode;

use crate::{Action, ActionLayer, ActionLayerSolution};

use super::{CatalogAction, CatalogActionSolution, CatalogConstants, CatalogInfo, CatalogState};

#[derive(Debug, Clone)]
#[must_use]
pub struct Catalog {
    pub coin: Coin,
    pub proof: Proof,
    pub info: CatalogInfo,
}

impl Catalog {
    pub fn new(coin: Coin, proof: Proof, info: CatalogInfo) -> Self {
        Self { coin, proof, info }
    }
}

impl Catalog {
    pub fn from_parent_spend(
        allocator: &mut Allocator,
        parent_coin: Coin,
        parent_puzzle: Puzzle,
        parent_solution: NodePtr,
        constants: CatalogConstants,
    ) -> Result<Option<Self>, DriverError>
    where
        Self: Sized,
    {
        let Some(parent_info) = CatalogInfo::parse(allocator, parent_puzzle, constants)? else {
            return Ok(None);
        };

        let proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: parent_coin.parent_coin_info,
            parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
            parent_amount: parent_coin.amount,
        });

        let new_state = ActionLayer::<CatalogState>::get_new_state(
            allocator,
            parent_info.state.clone(),
            parent_solution,
        )?;

        let new_info = parent_info.with_state(new_state);

        let new_coin = Coin::new(parent_coin.coin_id(), new_info.puzzle_hash().into(), 1);

        Ok(Some(Catalog {
            coin: new_coin,
            proof,
            info: new_info,
        }))
    }
}

impl Catalog {
    pub fn spend(
        self,
        ctx: &mut SpendContext,
        actions: Vec<CatalogAction>,
        solutions: Vec<CatalogActionSolution>,
    ) -> Result<(), DriverError> {
        let layers = self.info.into_layers();

        let puzzle = layers.construct_puzzle(ctx)?;

        let actions = actions
            .into_iter()
            .map(|a| a.construct_puzzle(ctx))
            .collect::<Result<Vec<_>, _>>()?;
        let action_puzzle_hashes = actions
            .iter()
            .map(|a| ctx.tree_hash(*a).into())
            .collect::<Vec<Bytes32>>();

        let solutions = solutions
            .into_iter()
            .map(|sol| match sol {
                CatalogActionSolution::Register(solution) => solution.to_clvm(&mut ctx.allocator),
                CatalogActionSolution::UpdatePrice(solution) => {
                    solution.to_clvm(&mut ctx.allocator)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // todo: debug \/
        let puz = ctx.serialize(&solutions[0].clone())?;
        println!(
            "___catalog register solution: {:?}",
            encode(ctx.serialize(&puz)?.into_bytes())
        );

        let puz = layers.inner_puzzle.construct_puzzle(ctx)?;
        println!(
            "action layer puzz: {:?}",
            encode(ctx.serialize(&puz)?.into_bytes())
        );

        let sol = ActionLayerSolution {
            proofs: layers
                .inner_puzzle
                .get_proofs(&action_puzzle_hashes)
                .ok_or(DriverError::Custom(
                    "Couldn't build proofs for one or more actions".to_string(),
                ))?,
            action_spends: actions
                .clone()
                .into_iter()
                .zip(solutions.clone())
                .map(|(a, s)| Spend {
                    puzzle: a,
                    solution: s,
                })
                .collect(),
        };
        let sol = layers.inner_puzzle.construct_solution(ctx, sol)?;
        println!(
            "action layer sol: {:?}",
            encode(ctx.serialize(&sol)?.into_bytes())
        );
        // todo: debug /\

        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof: self.proof,
                amount: self.coin.amount,
                inner_solution: ActionLayerSolution {
                    proofs: layers
                        .inner_puzzle
                        .get_proofs(&action_puzzle_hashes)
                        .ok_or(DriverError::Custom(
                            "Couldn't build proofs for one or more actions".to_string(),
                        ))?,
                    action_spends: actions
                        .into_iter()
                        .zip(solutions)
                        .map(|(a, s)| Spend {
                            puzzle: a,
                            solution: s,
                        })
                        .collect(),
                },
            },
        )?;

        ctx.spend(self.coin, Spend::new(puzzle, solution))
    }
}

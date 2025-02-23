use chia::{
    bls::PublicKey,
    protocol::{Coin, CoinSpend},
    puzzles::{
        singleton::{
            LauncherSolution, SingletonArgs, SingletonSolution, SINGLETON_LAUNCHER_PUZZLE_HASH,
        },
        EveProof, LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    Condition, Conditions, DriverError, Layer, Puzzle, SingletonLayer, Spend, SpendContext,
};
use clvm_traits::{clvm_quote, FromClvm, ToClvm};
use clvmr::NodePtr;

use crate::{MOfNLayer, P2MOfNDelegateDirectArgs, P2MOfNDelegateDirectSolution};

use super::{MedievalVaultHint, MedievalVaultInfo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedievalVault {
    pub coin: Coin,
    pub proof: Proof,

    pub info: MedievalVaultInfo,
}

impl MedievalVault {
    pub fn new(coin: Coin, proof: Proof, info: MedievalVaultInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn from_launcher_spend(
        ctx: &mut SpendContext,
        launcher_spend: CoinSpend,
    ) -> Result<Option<Self>, DriverError> {
        if launcher_spend.coin.puzzle_hash != SINGLETON_LAUNCHER_PUZZLE_HASH.into() {
            return Ok(None);
        }

        let solution = launcher_spend.solution.to_clvm(&mut ctx.allocator)?;
        let solution = LauncherSolution::<NodePtr>::from_clvm(&ctx.allocator, solution)?;

        let Ok(hint) = MedievalVaultHint::from_clvm(&ctx.allocator, solution.key_value_list) else {
            return Ok(None);
        };

        let info = MedievalVaultInfo::from_hint(hint);

        let new_coin = Coin::new(
            launcher_spend.coin.coin_id(),
            SingletonArgs::curry_tree_hash(info.launcher_id, info.inner_puzzle_hash()).into(),
            1,
        );

        if launcher_spend.coin.amount != new_coin.amount
            || new_coin.puzzle_hash != solution.singleton_puzzle_hash
        {
            return Ok(None);
        }

        Ok(Some(Self::new(
            new_coin,
            Proof::Eve(EveProof {
                parent_parent_coin_info: launcher_spend.coin.parent_coin_info,
                parent_amount: launcher_spend.coin.amount,
            }),
            info,
        )))
    }

    pub fn child(&self, new_m: usize, new_public_key_list: Vec<PublicKey>) -> Option<Self> {
        let child_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
            parent_amount: self.coin.amount,
        });

        let child_info = MedievalVaultInfo::new(self.info.launcher_id, new_m, new_public_key_list);
        let child_inner_puzzle_hash = child_info.inner_puzzle_hash();

        Some(Self {
            coin: Coin::new(
                self.coin.coin_id(),
                SingletonArgs::curry_tree_hash(self.info.launcher_id, child_inner_puzzle_hash)
                    .into(),
                1,
            ),
            proof: child_proof,
            info: child_info,
        })
    }

    pub fn from_parent_spend(
        ctx: &mut SpendContext,
        parent_spend: CoinSpend,
    ) -> Result<Option<Self>, DriverError> {
        if parent_spend.coin.puzzle_hash == SINGLETON_LAUNCHER_PUZZLE_HASH.into() {
            return Self::from_launcher_spend(ctx, parent_spend);
        }

        let solution = parent_spend.solution.to_clvm(&mut ctx.allocator)?;
        let puzzle = parent_spend.puzzle_reveal.to_clvm(&mut ctx.allocator)?;

        let puzzle_puzzle = Puzzle::from_clvm(&ctx.allocator, puzzle)?;
        let Some(parent_layers) =
            SingletonLayer::<MOfNLayer>::parse_puzzle(&ctx.allocator, puzzle_puzzle)?
        else {
            return Ok(None);
        };

        let output = ctx.run(puzzle, solution)?;
        let output = Conditions::<NodePtr>::from_clvm(&ctx.allocator, output)?;
        let recreate_condition = output
            .into_iter()
            .find(|c| matches!(c, Condition::CreateCoin(..)));
        let Some(Condition::CreateCoin(recreate_condition)) = recreate_condition else {
            return Ok(None);
        };

        let (new_m, new_pubkeys) = if recreate_condition.memos.is_none() {
            (
                parent_layers.inner_puzzle.m,
                parent_layers.inner_puzzle.public_key_list.clone(),
            )
        } else {
            let memos = recreate_condition.memos.unwrap();
            let memos = memos.to_clvm(&mut ctx.allocator)?;
            let memos = MedievalVaultHint::from_clvm(&ctx.allocator, memos)?;
            (memos.m, memos.public_key_list)
        };

        let parent_info = MedievalVaultInfo::new(
            parent_layers.launcher_id,
            parent_layers.inner_puzzle.m,
            parent_layers.inner_puzzle.public_key_list,
        );
        let new_info = MedievalVaultInfo::new(parent_layers.launcher_id, new_m, new_pubkeys);

        let new_coin = Coin::new(
            parent_spend.coin.coin_id(),
            SingletonArgs::curry_tree_hash(parent_layers.launcher_id, new_info.inner_puzzle_hash())
                .into(),
            1,
        );

        Ok(Some(Self::new(
            new_coin,
            Proof::Lineage(LineageProof {
                parent_parent_coin_info: parent_spend.coin.parent_coin_info,
                parent_inner_puzzle_hash: parent_info.inner_puzzle_hash().into(),
                parent_amount: parent_spend.coin.amount,
            }),
            new_info,
        )))
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        used_pubkeys: &[PublicKey],
        conditions: Conditions,
    ) -> Result<(), DriverError> {
        let lineage_proof = self.proof;
        let coin = self.coin;

        let layers = self.info.into_layers();

        let delegated_puzzle = clvm_quote!(conditions.assert_my_coin_id(self.coin.coin_id()))
            .to_clvm(&mut ctx.allocator)?;

        let puzzle = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(
            ctx,
            SingletonSolution {
                lineage_proof,
                amount: coin.amount,
                inner_solution: P2MOfNDelegateDirectSolution {
                    selectors: P2MOfNDelegateDirectArgs::selectors_for_used_pubkeys(
                        &self.info.public_key_list,
                        used_pubkeys,
                    ),
                    delegated_puzzle,
                    delegated_solution: NodePtr::NIL,
                },
            },
        )?;

        ctx.spend(coin, Spend::new(puzzle, solution))?;

        Ok(())
    }
}

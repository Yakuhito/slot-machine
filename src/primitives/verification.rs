use chia::{
    clvm_utils::{ToTreeHash, TreeHash},
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{
        singleton::{SingletonArgs, SingletonSolution},
        EveProof, Proof,
    },
};
use chia_wallet_sdk::{DriverError, Layer, SingletonLayer, SpendContext};
use clvmr::NodePtr;

use crate::{VerificationLayer, VerificationLayer2ndCurryArgs, VerificationLayerSolution};

use super::VerificationInfo;

type VerificationLayers = SingletonLayer<VerificationLayer>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Verification {
    pub coin: Coin,
    pub proof: Proof,

    pub info: VerificationInfo,
}

impl Verification {
    pub fn new(coin: Coin, proof: Proof, info: VerificationInfo) -> Self {
        Self { coin, proof, info }
    }

    pub fn after_mint(launcher_parent: Bytes32, info: VerificationInfo) -> Self {
        Self {
            coin: Coin::new(info.launcher_id, Self::puzzle_hash(&info).into(), 1),
            proof: Proof::Eve(EveProof {
                parent_parent_coin_info: launcher_parent,
                parent_amount: 0,
            }),
            info,
        }
    }

    pub fn inner_puzzle_hash<T>(
        revocation_singleton_launcher_id: Bytes32,
        verified_data: T,
    ) -> TreeHash
    where
        T: ToTreeHash,
    {
        VerificationLayer2ndCurryArgs::curry_tree_hash(
            revocation_singleton_launcher_id,
            verified_data,
        )
    }

    pub fn puzzle_hash(info: &VerificationInfo) -> TreeHash {
        SingletonArgs::curry_tree_hash(
            info.launcher_id,
            Self::inner_puzzle_hash(
                info.revocation_singleton_launcher_id,
                info.verified_data.tree_hash(),
            ),
        )
    }

    pub fn into_layers(self) -> VerificationLayers {
        SingletonLayer::new(
            self.info.launcher_id,
            VerificationLayer::new(
                self.info.revocation_singleton_launcher_id,
                self.info.verified_data,
            ),
        )
    }

    pub fn into_layers_with_clone(&self) -> VerificationLayers {
        SingletonLayer::new(
            self.info.launcher_id,
            VerificationLayer::new(
                self.info.revocation_singleton_launcher_id,
                self.info.verified_data.clone(),
            ),
        )
    }

    pub fn construct_puzzle(&self, ctx: &mut SpendContext) -> Result<NodePtr, DriverError> {
        let layers = self.into_layers_with_clone();

        layers.construct_puzzle(ctx)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        revocation_singleton_inner_puzzle_hash: Option<Bytes32>,
    ) -> Result<(), DriverError> {
        let sol = SingletonSolution {
            lineage_proof: self.proof,
            amount: self.coin.amount,
            inner_solution: VerificationLayerSolution {
                revocation_singleton_inner_puzzle_hash,
            },
        };
        let my_coin = self.coin;

        let layers = self.into_layers();

        let puzzle_reveal = layers.construct_puzzle(ctx)?;
        let solution = layers.construct_solution(ctx, sol)?;

        let puzzle_reveal = ctx.serialize(&puzzle_reveal)?;
        let solution = ctx.serialize(&solution)?;

        ctx.insert(CoinSpend::new(my_coin, puzzle_reveal, solution));

        Ok(())
    }
}

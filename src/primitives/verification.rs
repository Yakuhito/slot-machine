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

#[cfg(test)]
mod tests {
    use anyhow::Ok;
    use chia::{bls::Signature, puzzles::singleton::SINGLETON_LAUNCHER_PUZZLE_HASH};
    use chia_wallet_sdk::{Conditions, Launcher, Simulator, StandardLayer};

    use crate::{print_spend_bundle_to_file, VerifiedData};

    use super::*;

    #[test]
    fn test_verifications() -> anyhow::Result<()> {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let (sk, pk, _, coin) = sim.new_p2(2)?;
        let p2 = StandardLayer::new(pk);

        let did_launcher = Launcher::new(coin.coin_id(), 1);
        let (create_did, did) = did_launcher.create_simple_did(ctx, &p2)?;
        p2.spend(ctx, coin, create_did)?;

        let did = did.update(
            ctx,
            &p2,
            Conditions::new().create_coin(SINGLETON_LAUNCHER_PUZZLE_HASH.into(), 0, vec![]),
        )?;
        let verification_launcher = Launcher::new(did.coin.parent_coin_info, 0);

        let test_info = VerificationInfo::new(
            verification_launcher.coin().coin_id(),
            did.coin.coin_id(),
            VerifiedData {
                version: 1,
                asset_id: Bytes32::new([2; 32]),
                data_hash: Bytes32::new([3; 32]),
                category: "cat".to_string(),
                subcategory: "subcat".to_string(),
            },
        );
        let verification =
            Verification::after_mint(verification_launcher.coin().parent_coin_info, test_info);

        let (_conds, new_coin) = verification_launcher.with_singleton_amount(1).spend(
            ctx,
            Verification::inner_puzzle_hash(
                verification.info.revocation_singleton_launcher_id,
                verification.info.verified_data.clone(),
            )
            .into(),
            (),
        )?;

        assert_eq!(new_coin, verification.coin);

        let spends = ctx.take();
        print_spend_bundle_to_file(spends.clone(), Signature::default(), "sb.debug");
        sim.spend_coins(spends, &[sk])?;

        Ok(())
    }
}

use chia::{
    bls::PublicKey,
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{
        singleton::{
            LauncherSolution, SingletonArgs, SingletonSolution, SINGLETON_LAUNCHER_PUZZLE_HASH,
        },
        EveProof, LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    Condition, Conditions, DriverError, Layer, Memos, Puzzle, SingletonLayer, Spend, SpendContext,
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
            if let Ok(memos) = MedievalVaultHint::from_clvm(&ctx.allocator, memos.value) {
                (memos.m, memos.public_key_list)
            } else {
                (
                    parent_layers.inner_puzzle.m,
                    parent_layers.inner_puzzle.public_key_list.clone(),
                )
            }
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

    pub fn delegated_conditions(
        conditions: Conditions,
        coin_id: Bytes32,
        genesis_challenge: NodePtr,
    ) -> Conditions {
        MOfNLayer::ensure_non_replayable(conditions, coin_id, genesis_challenge)
    }

    pub fn spend(
        self,
        ctx: &mut SpendContext,
        used_pubkeys: &[PublicKey],
        conditions: Conditions,
        genesis_challenge: Bytes32,
    ) -> Result<(), DriverError> {
        let lineage_proof = self.proof;
        let coin = self.coin;

        let layers = self.info.into_layers();

        let genesis_challenge = genesis_challenge.to_clvm(&mut ctx.allocator)?;
        let delegated_puzzle = clvm_quote!(Self::delegated_conditions(
            conditions,
            coin.coin_id(),
            genesis_challenge
        ))
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

    pub fn delegated_puzzle_for_rekey(
        ctx: &mut SpendContext,
        launcher_id: Bytes32,
        new_m: usize,
        new_pubkeys: Vec<PublicKey>,
        coin_id: Bytes32,
        genesis_challenge: Bytes32,
    ) -> Result<NodePtr, DriverError> {
        let new_info = MedievalVaultInfo::new(launcher_id, new_m, new_pubkeys);
        let new_info = new_info.to_hint();
        let memos = ctx.alloc(&new_info)?;

        let memos = ctx.alloc(&new_info.to_hint())?;
        let conditions = Conditions::new().create_coin(
            new_info.inner_puzzle_hash().into(),
            1,
            Some(Memos::new(memos)),
        );

        ctx.alloc(&clvm_quote!(Self::delegated_conditions(
            conditions,
            coin_id,
            ctx.alloc(&genesis_challenge)?
        )))
    }
}

#[cfg(test)]
mod tests {
    use chia::bls::SecretKey;
    use chia_wallet_sdk::{test_secret_keys, Launcher, Memos, Simulator, TESTNET11_CONSTANTS};

    use super::*;

    #[test]
    fn test_medieval_vault() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        let [sk1, sk2, sk3]: [SecretKey; 3] = test_secret_keys(3)?.try_into().unwrap();
        let (pk1, pk2, pk3) = (sk1.public_key(), sk2.public_key(), sk3.public_key());

        let multisig_configs = [
            (1, vec![pk1, pk2]),
            (2, vec![pk1, pk2]),
            (3, vec![pk1, pk2, pk3]),
            (3, vec![pk1, pk2, pk3]),
            (1, vec![pk1, pk2, pk3]),
            (2, vec![pk1, pk2, pk3]),
        ];

        let launcher_coin = sim.new_coin(SINGLETON_LAUNCHER_PUZZLE_HASH.into(), 1);
        let launcher = Launcher::new(launcher_coin.parent_coin_info, 1);
        let launch_hints = MedievalVaultHint {
            my_launcher_id: launcher_coin.coin_id(),
            m: multisig_configs[0].0,
            public_key_list: multisig_configs[0].1.clone(),
        };
        let (_conds, first_vault_coin) = launcher.spend(
            ctx,
            P2MOfNDelegateDirectArgs::curry_tree_hash(
                multisig_configs[0].0,
                multisig_configs[0].1.clone(),
            )
            .into(),
            launch_hints,
        )?;

        let spends = ctx.take();
        let launcher_spend = spends.first().unwrap().clone();
        sim.spend_coins(spends, &[])?;

        let mut vault = MedievalVault::from_parent_spend(ctx, launcher_spend)?.unwrap();
        assert_eq!(vault.coin, first_vault_coin);

        let mut current_vault_info = MedievalVaultInfo {
            launcher_id: launcher_coin.coin_id(),
            m: multisig_configs[0].0,
            public_key_list: multisig_configs[0].1.clone(),
        };
        assert_eq!(vault.info, current_vault_info);

        for (i, (m, pubkeys)) in multisig_configs.clone().into_iter().enumerate().skip(1) {
            let mut recreate_memos: NodePtr =
                vec![vault.info.launcher_id].to_clvm(&mut ctx.allocator)?;

            let info_changed =
                multisig_configs[i - 1].0 != m || multisig_configs[i - 1].1 != pubkeys;
            if info_changed {
                recreate_memos = MedievalVaultHint {
                    my_launcher_id: vault.info.launcher_id,
                    m,
                    public_key_list: pubkeys.clone(),
                }
                .to_clvm(&mut ctx.allocator)?;
            }
            current_vault_info = MedievalVaultInfo {
                launcher_id: vault.info.launcher_id,
                m,
                public_key_list: pubkeys.clone(),
            };

            let recreate_condition = Conditions::<NodePtr>::new().create_coin(
                current_vault_info.inner_puzzle_hash().into(),
                1,
                Memos::some(recreate_memos),
            );

            let mut used_keys = 0;
            let mut used_pubkeys = vec![];
            while used_keys < vault.info.m {
                used_pubkeys.push(current_vault_info.public_key_list[used_keys]);
                used_keys += 1;
            }
            vault.clone().spend(
                ctx,
                &used_pubkeys,
                recreate_condition,
                TESTNET11_CONSTANTS.genesis_challenge,
            )?;

            let spends = ctx.take();
            let vault_spend = spends.first().unwrap().clone();
            sim.spend_coins(spends, &[sk1.clone(), sk2.clone(), sk3.clone()])?;

            let check_vault = vault.child(m, pubkeys).unwrap();

            vault = MedievalVault::from_parent_spend(ctx, vault_spend)?.unwrap();
            assert_eq!(vault.info, current_vault_info);
            assert_eq!(vault, check_vault);
        }

        Ok(())
    }
}

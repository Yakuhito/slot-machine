use bip39::Mnemonic;
use chia::{
    bls::{sign, SecretKey, Signature},
    clvm_utils::ToTreeHash,
    consensus::consensus_constants::ConsensusConstants,
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{
        offer::{
            NotarizedPayment, Payment, SettlementPaymentsSolution, SETTLEMENT_PAYMENTS_PUZZLE_HASH,
        },
        singleton::{SingletonArgs, SingletonSolution},
        standard::{StandardArgs, StandardSolution},
        EveProof, LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    AggSig, AggSigConstants, AggSigKind, Condition, Conditions, DriverError, Launcher, Layer,
    Offer, RequiredBlsSignature, Spend, SpendContext, StandardLayer,
};
use clvm_traits::{clvm_quote, FromClvm, ToClvm};
use clvmr::NodePtr;

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState,
    CatalogSlotValue, DefaultCatMakerArgs, DigRewardDistributor, DigRewardDistributorConstants,
    DigRewardDistributorInfo, DigRewardDistributorState, DigRewardSlotValue, DigSlotNonce,
    RoundRewardInfo, RoundTimeInfo, Slot, SlotInfo, SlotProof, XchandlesConstants,
    XchandlesRegistry, XchandlesRegistryInfo, XchandlesRegistryState, XchandlesSlotValue,
    SLOT32_MAX_VALUE, SLOT32_MIN_VALUE,
};

pub struct SecuredOneSidedOffer {
    pub coin_spends: Vec<CoinSpend>,
    pub aggregated_signature: Signature,
    pub security_coin: Coin,
    pub security_coin_sk: SecretKey,
}

fn custom_err<T>(e: T) -> DriverError
where
    T: ToString,
{
    DriverError::Custom(e.to_string())
}

pub fn parse_one_sided_offer(
    ctx: &mut SpendContext,
    offer: Offer,
) -> Result<SecuredOneSidedOffer, DriverError> {
    let offer = offer.parse(&mut ctx.allocator).map_err(custom_err)?;

    if !offer.requested_payments.is_empty() {
        return Err(DriverError::Custom(
            "Launch offer should not have any requested payments".to_string(),
        ));
    }

    // we need the security coin puzzle hash to spend the offer coin after finding it
    let mut entropy = [0u8; 32];
    getrandom::getrandom(&mut entropy).map_err(custom_err)?;
    let mnemonic = Mnemonic::from_entropy(&entropy).map_err(custom_err)?;
    let seed = mnemonic.to_seed("");
    let sk = SecretKey::from_seed(&seed);
    let security_coin_puzzle_hash: Bytes32 = StandardArgs::curry_tree_hash(sk.public_key()).into();

    // returned spends will also spend the offer coin (creating the security coin)
    let mut coin_spends = Vec::with_capacity(offer.coin_spends.len() + 1);
    let mut security_coin_parent_id: Option<Bytes32> = None;
    let mut security_coin_amount = 0;

    for coin_spend in offer.coin_spends {
        if security_coin_parent_id.is_none() {
            let puzzle_ptr = coin_spend.puzzle_reveal.to_clvm(&mut ctx.allocator)?;
            let solution_ptr = coin_spend.solution.to_clvm(&mut ctx.allocator)?;

            let res = ctx.run(puzzle_ptr, solution_ptr)?;
            let res = Vec::<Condition<NodePtr>>::from_clvm(&ctx.allocator, res)?;

            if let Some(cc) = res
                .into_iter()
                .filter_map(|cond| {
                    let Condition::CreateCoin(cc) = cond else {
                        return None;
                    };

                    Some(cc)
                })
                .find(|cc| cc.puzzle_hash == SETTLEMENT_PAYMENTS_PUZZLE_HASH.into())
            {
                let offer_coin = Coin::new(
                    coin_spend.coin.coin_id(),
                    SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                    cc.amount,
                );
                let offer_coin_id = offer_coin.coin_id();

                security_coin_parent_id = Some(offer_coin_id);
                security_coin_amount = cc.amount;

                let offer_coin_puzzle = ctx.settlement_payments_puzzle()?;
                let offer_coin_puzzle = ctx.serialize(&offer_coin_puzzle)?;

                let offer_coin_solution = SettlementPaymentsSolution {
                    notarized_payments: vec![NotarizedPayment {
                        nonce: offer_coin_id,
                        payments: vec![Payment {
                            puzzle_hash: security_coin_puzzle_hash,
                            amount: security_coin_amount,
                            memos: None,
                        }],
                    }],
                };
                let offer_coin_solution = ctx.serialize(&offer_coin_solution)?;

                let offer_coin_spend =
                    CoinSpend::new(offer_coin, offer_coin_puzzle, offer_coin_solution);
                coin_spends.push(offer_coin_spend);
            }
        }

        coin_spends.push(coin_spend);
    }

    let Some(security_coin_parent_id) = security_coin_parent_id else {
        return Err(DriverError::Custom(
            "Launch offer should offer XCH".to_string(),
        ));
    };

    let security_coin = Coin::new(
        security_coin_parent_id,
        security_coin_puzzle_hash,
        security_coin_amount,
    );

    Ok(SecuredOneSidedOffer {
        coin_spends,
        aggregated_signature: offer.aggregated_signature,
        security_coin,
        security_coin_sk: sk,
    })
}

pub fn spend_security_coin(
    ctx: &mut SpendContext,
    security_coin: Coin,
    conditions: Conditions<NodePtr>,
    sk: &SecretKey,
    consensus_constants: &ConsensusConstants,
) -> Result<Signature, DriverError> {
    let pk = sk.public_key();

    let layer = StandardLayer::new(pk);
    let puzzle_reveal_ptr = layer.construct_puzzle(ctx)?;

    let quoted_conditions_ptr = clvm_quote!(conditions).to_clvm(&mut ctx.allocator)?;
    let solution_ptr = layer.construct_solution(
        ctx,
        StandardSolution {
            original_public_key: None,
            delegated_puzzle: quoted_conditions_ptr,
            solution: NodePtr::NIL,
        },
    )?;

    let spend = Spend::new(puzzle_reveal_ptr, solution_ptr);
    ctx.spend(security_coin, spend)?;

    sign_standard_transaction(ctx, security_coin, spend, sk, consensus_constants)
}

pub fn sign_standard_transaction(
    ctx: &mut SpendContext,
    coin: Coin,
    spend: Spend,
    sk: &SecretKey,
    consensus_constants: &ConsensusConstants,
) -> Result<Signature, DriverError> {
    let output = ctx.run(spend.puzzle, spend.solution)?;
    let output = Vec::<Condition<NodePtr>>::from_clvm(&ctx.allocator, output)?;
    let Some(agg_sig_me) = output.iter().find_map(|cond| {
        if let Condition::AggSigMe(agg_sig_me) = cond {
            return Some(agg_sig_me);
        }

        None
    }) else {
        return Err(DriverError::Custom(
            "Missing agg_sig_me from security coin".to_string(),
        ));
    };

    let required_signature = RequiredBlsSignature::from_condition(
        &coin,
        AggSig::new(
            AggSigKind::Me,
            agg_sig_me.public_key,
            agg_sig_me.message.clone(),
        ),
        &AggSigConstants::new(consensus_constants.agg_sig_me_additional_data),
    );

    Ok(sign(sk, required_signature.message()))
}

// Spends the eve signleton, whose only job is to create the
//   slot 'premine' (leftmost and rightmost slots) and
//   transition to the actual registry puzzle
#[allow(clippy::type_complexity)]
fn spend_eve_coin_and_create_registry<S>(
    ctx: &mut SpendContext,
    launcher: Launcher,
    target_inner_puzzle_hash: Bytes32,
    left_slot_value: S,
    right_slot_value: S,
) -> Result<(Conditions, Coin, Proof, [Slot<S>; 2]), DriverError>
where
    S: Copy + ToTreeHash,
{
    let launcher_coin = launcher.coin();
    let launcher_id = launcher_coin.coin_id();

    let left_slot_info = SlotInfo::from_value(launcher_id, left_slot_value, None);
    let left_slot_puzzle_hash = Slot::<S>::puzzle_hash(&left_slot_info);

    let right_slot_info = SlotInfo::from_value(launcher_id, right_slot_value, None);
    let right_slot_puzzle_hash = Slot::<S>::puzzle_hash(&right_slot_info);

    let slot_hint: Bytes32 = Slot::<()>::first_curry_hash(launcher_id, None).into();
    let slot_memos = ctx.hint(slot_hint)?;
    let launcher_memos = ctx.hint(launcher_id)?;
    let eve_singleton_inner_puzzle = clvm_quote!(Conditions::new()
        .create_coin(left_slot_puzzle_hash.into(), 0, Some(slot_memos))
        .create_coin(right_slot_puzzle_hash.into(), 0, Some(slot_memos))
        .create_coin(target_inner_puzzle_hash, 1, Some(launcher_memos)))
    .to_clvm(&mut ctx.allocator)?;

    let eve_singleton_inner_puzzle_hash = ctx.tree_hash(eve_singleton_inner_puzzle);
    let eve_singleton_proof = Proof::Eve(EveProof {
        parent_parent_coin_info: launcher_coin.parent_coin_info,
        parent_amount: launcher_coin.amount,
    });

    let (security_coin_conditions, eve_coin) =
        launcher
            .with_singleton_amount(1)
            .spend(ctx, eve_singleton_inner_puzzle_hash.into(), ())?;

    let eve_coin_solution = SingletonSolution {
        lineage_proof: eve_singleton_proof,
        amount: 1,
        inner_solution: NodePtr::NIL,
    }
    .to_clvm(&mut ctx.allocator)?;

    let eve_singleton_puzzle =
        ctx.curry(SingletonArgs::new(launcher_id, eve_singleton_inner_puzzle))?;
    let eve_singleton_spend = Spend::new(eve_singleton_puzzle, eve_coin_solution);
    ctx.spend(eve_coin, eve_singleton_spend)?;

    let new_registry_coin = Coin::new(
        eve_coin.coin_id(),
        SingletonArgs::curry_tree_hash(launcher_id, target_inner_puzzle_hash.into()).into(),
        1,
    );
    let new_proof = Proof::Lineage(LineageProof {
        parent_parent_coin_info: eve_coin.parent_coin_info,
        parent_inner_puzzle_hash: eve_singleton_inner_puzzle_hash.into(),
        parent_amount: 1,
    });

    let slot_proof = SlotProof {
        parent_parent_info: eve_coin.parent_coin_info,
        parent_inner_puzzle_hash: eve_singleton_inner_puzzle_hash.into(),
    };
    let left_slot = Slot::new(slot_proof, left_slot_info);
    let right_slot = Slot::new(slot_proof, right_slot_info);

    Ok((
        security_coin_conditions.assert_concurrent_spend(eve_coin.coin_id()),
        new_registry_coin,
        new_proof,
        [left_slot, right_slot],
    ))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn launch_catalog_registry(
    ctx: &mut SpendContext,
    offer: Offer,
    initial_registration_price: u64,
    initial_registration_asset_id: Bytes32,
    catalog_constants: CatalogRegistryConstants,
    consensus_constants: &ConsensusConstants,
) -> Result<
    (
        Signature,
        SecretKey,
        CatalogRegistry,
        [Slot<CatalogSlotValue>; 2],
    ),
    DriverError,
> {
    let offer = parse_one_sided_offer(ctx, offer)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = Conditions::new();

    // Create coin launcher
    let registry_launcher = Launcher::new(security_coin_id, 1);
    let registry_launcher_coin = registry_launcher.coin();
    let registry_launcher_id = registry_launcher_coin.coin_id();

    let catalog_registry_info = CatalogRegistryInfo::new(
        registry_launcher_id,
        CatalogRegistryState {
            registration_price: initial_registration_price,
            cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(
                initial_registration_asset_id.tree_hash().into(),
            )
            .into(),
        },
        catalog_constants,
    );
    let catalog_inner_puzzle_hash = catalog_registry_info.clone().inner_puzzle_hash();

    let (new_security_coin_conditions, new_catalog_registry_coin, catalog_proof, slots) =
        spend_eve_coin_and_create_registry(
            ctx,
            registry_launcher,
            catalog_inner_puzzle_hash.into(),
            CatalogSlotValue::new(
                SLOT32_MIN_VALUE.into(),
                SLOT32_MIN_VALUE.into(),
                SLOT32_MAX_VALUE.into(),
            ),
            CatalogSlotValue::new(
                SLOT32_MAX_VALUE.into(),
                SLOT32_MIN_VALUE.into(),
                SLOT32_MAX_VALUE.into(),
            ),
        )?;

    // this creates the launcher & secures the spend
    security_coin_conditions = security_coin_conditions.extend(new_security_coin_conditions);

    let catalog_registry = CatalogRegistry::new(
        new_catalog_registry_coin,
        catalog_proof,
        catalog_registry_info,
    );

    // Spend security coin
    let security_coin_sig = spend_security_coin(
        ctx,
        offer.security_coin,
        security_coin_conditions,
        &offer.security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        offer.aggregated_signature + &security_coin_sig,
        offer.security_coin_sk,
        catalog_registry,
        slots,
    ))
}

#[allow(clippy::type_complexity)]
pub fn launch_xchandles_registry(
    ctx: &mut SpendContext,
    offer: Offer,
    initial_base_registration_price: u64,
    initial_registration_asset_id: Bytes32,
    xchandles_constants: XchandlesConstants,
    consensus_constants: &ConsensusConstants,
) -> Result<
    (
        Signature,
        SecretKey,
        XchandlesRegistry,
        [Slot<XchandlesSlotValue>; 2],
    ),
    DriverError,
> {
    let offer = parse_one_sided_offer(ctx, offer)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = Conditions::new();

    // Create registry coin launcher
    let registry_launcher = Launcher::new(security_coin_id, 1);
    let registry_launcher_coin = registry_launcher.coin();
    let registry_launcher_id = registry_launcher_coin.coin_id();

    // Spend intermediary coin and create registry
    let target_xchandles_info = XchandlesRegistryInfo::new(
        registry_launcher_id,
        XchandlesRegistryState::from(
            initial_registration_asset_id.tree_hash().into(),
            initial_base_registration_price,
        ),
        xchandles_constants,
    );
    let target_xchandles_inner_puzzle_hash = target_xchandles_info.clone().inner_puzzle_hash();
    let (new_security_coin_conditions, new_xchandles_coin, xchandles_proof, slots) =
        spend_eve_coin_and_create_registry(
            ctx,
            registry_launcher,
            target_xchandles_inner_puzzle_hash.into(),
            XchandlesSlotValue::new(
                SLOT32_MIN_VALUE.into(),
                SLOT32_MIN_VALUE.into(),
                SLOT32_MAX_VALUE.into(),
                u64::MAX,
                registry_launcher_id,
                registry_launcher_id,
            ),
            XchandlesSlotValue::new(
                SLOT32_MAX_VALUE.into(),
                SLOT32_MIN_VALUE.into(),
                SLOT32_MAX_VALUE.into(),
                u64::MAX,
                registry_launcher_id,
                registry_launcher_id,
            ),
        )?;

    // this creates the launcher & secures the spend
    security_coin_conditions = security_coin_conditions.extend(new_security_coin_conditions);

    let xchandles_registry =
        XchandlesRegistry::new(new_xchandles_coin, xchandles_proof, target_xchandles_info);

    // Spend security coin
    let security_coin_sig = spend_security_coin(
        ctx,
        offer.security_coin,
        security_coin_conditions,
        &offer.security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        offer.aggregated_signature + &security_coin_sig,
        offer.security_coin_sk,
        xchandles_registry,
        slots,
    ))
}

#[allow(clippy::type_complexity)]
pub fn launch_dig_reward_distributor(
    ctx: &mut SpendContext,
    offer: Offer,
    first_epoch_start: u64,
    constants: DigRewardDistributorConstants,
    consensus_constants: &ConsensusConstants,
) -> Result<
    (
        Signature,
        SecretKey,
        DigRewardDistributor,
        Slot<DigRewardSlotValue>,
    ),
    DriverError,
> {
    let offer = parse_one_sided_offer(ctx, offer)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    // Create coin launcher
    let launcher = Launcher::new(security_coin_id, 1);
    let launcher_coin = launcher.coin();
    let launcher_id = launcher_coin.coin_id();

    // Spend intermediary coin and create registry
    let target_info = DigRewardDistributorInfo::new(
        launcher_id,
        DigRewardDistributorState {
            total_reserves: 0,
            active_shares: 0,
            round_reward_info: RoundRewardInfo {
                cumulative_payout: 0,
                remaining_rewards: 0,
            },
            round_time_info: RoundTimeInfo {
                last_update: first_epoch_start,
                epoch_end: first_epoch_start,
            },
        },
        constants.with_launcher_id(launcher_id),
    );
    let target_inner_puzzle_hash = target_info.clone().inner_puzzle_hash();

    let slot_value = DigRewardSlotValue {
        epoch_start: first_epoch_start,
        next_epoch_start: 0,
        rewards: 0,
    };
    let slot_info = SlotInfo::<DigRewardSlotValue>::from_value(
        launcher_id,
        slot_value,
        Some(DigSlotNonce::REWARD.to_u64()),
    );
    let slot_puzzle_hash = Slot::<DigRewardSlotValue>::puzzle_hash(&slot_info);

    let slot_hint: Bytes32 =
        Slot::<()>::first_curry_hash(launcher_id, Some(DigSlotNonce::REWARD.to_u64())).into();
    let slot_memos = ctx.hint(slot_hint)?;
    let launcher_memos = ctx.hint(launcher_id)?;
    let eve_singleton_inner_puzzle = clvm_quote!(Conditions::new()
        .create_coin(slot_puzzle_hash.into(), 0, Some(slot_memos))
        .create_coin(target_inner_puzzle_hash.into(), 1, Some(launcher_memos)))
    .to_clvm(&mut ctx.allocator)?;

    let eve_singleton_inner_puzzle_hash = ctx.tree_hash(eve_singleton_inner_puzzle);
    let eve_singleton_proof = Proof::Eve(EveProof {
        parent_parent_coin_info: launcher_coin.parent_coin_info,
        parent_amount: launcher_coin.amount,
    });

    let (security_coin_conditions, eve_coin) =
        launcher
            .with_singleton_amount(1)
            .spend(ctx, eve_singleton_inner_puzzle_hash.into(), ())?;

    let eve_coin_solution = SingletonSolution {
        lineage_proof: eve_singleton_proof,
        amount: 1,
        inner_solution: NodePtr::NIL,
    }
    .to_clvm(&mut ctx.allocator)?;

    let eve_singleton_puzzle =
        ctx.curry(SingletonArgs::new(launcher_id, eve_singleton_inner_puzzle))?;
    let eve_singleton_spend = Spend::new(eve_singleton_puzzle, eve_coin_solution);
    ctx.spend(eve_coin, eve_singleton_spend)?;

    let new_registry_coin = Coin::new(
        eve_coin.coin_id(),
        SingletonArgs::curry_tree_hash(launcher_id, target_inner_puzzle_hash).into(),
        1,
    );
    let new_proof = Proof::Lineage(LineageProof {
        parent_parent_coin_info: eve_coin.parent_coin_info,
        parent_inner_puzzle_hash: eve_singleton_inner_puzzle_hash.into(),
        parent_amount: 1,
    });

    let slot_proof = SlotProof {
        parent_parent_info: eve_coin.parent_coin_info,
        parent_inner_puzzle_hash: eve_singleton_inner_puzzle_hash.into(),
    };
    let slot = Slot::new(slot_proof, slot_info);

    // this creates the launcher & secures the spend
    let security_coin_conditions =
        security_coin_conditions.assert_concurrent_spend(eve_coin.coin_id());
    let registry = DigRewardDistributor::new(new_registry_coin, new_proof, target_info);

    // Spend security coin
    let security_coin_sig = spend_security_coin(
        ctx,
        offer.security_coin,
        security_coin_conditions,
        &offer.security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        offer.aggregated_signature + &security_coin_sig,
        offer.security_coin_sk,
        registry,
        slot,
    ))
}

#[cfg(test)]
mod tests {
    use chia::{
        protocol::SpendBundle,
        puzzles::{
            cat::GenesisByCoinIdTailArgs,
            singleton::{SingletonSolution, SingletonStruct, SINGLETON_LAUNCHER_PUZZLE_HASH},
            CoinProof,
        },
    };
    use chia_wallet_sdk::{
        test_secret_keys, Cat, CatSpend, Nft, NftMint, Puzzle, Simulator, SingleCatSpend,
        SpendWithConditions, TESTNET11_CONSTANTS,
    };
    use clvm_traits::clvm_list;
    use hex_literal::hex;

    use crate::{
        print_spend_bundle_to_file, CatNftMetadata, CatalogPrecommitValue, CatalogRegistryAction,
        CatalogSlotValue, DelegatedStateActionSolution, PrecommitCoin, Reserve, Slot,
        SpendContextExt, XchandlesExponentialPremiumRenewPuzzleArgs,
        XchandlesExponentialPremiumRenewPuzzleSolution, XchandlesFactorPricingPuzzleArgs,
        XchandlesFactorPricingSolution, XchandlesPrecommitValue, XchandlesRegistryAction,
        ANY_METADATA_UPDATER_HASH,
    };

    use super::*;

    fn cat_nft_metadata_for_testing() -> CatNftMetadata {
        CatNftMetadata {
            code: "TDBX".to_string(),
            name: "Testnet dexie bucks".to_string(),
            description: "    Testnet version of dexie bucks".to_string(),
            precision: 4,
            image_uris: vec!["https://icons-testnet.dexie.space/d82dd03f8a9ad2f84353cd953c4de6b21dbaaf7de3ba3f4ddd9abe31ecba80ad.webp".to_string()],
            image_hash: Bytes32::from(
                hex!("c84607c0e4cb4a878cc34ba913c90504ed0aac0f4484c2078529b9e42387da99")
            ),
            metadata_uris: vec!["https://icons-testnet.dexie.space/test.json".to_string()],
            metadata_hash: Bytes32::from([2; 32]),
            license_uris: vec!["https://icons-testnet.dexie.space/license.pdf".to_string()],
            license_hash: Bytes32::from([3; 32]),
        }
    }

    // Launches a test singleton with an innter puzzle of '1'
    // JUST FOR TESTING PURPOSES PLEASE DO NOT USE THIS THING IN PRODUCTION
    fn launch_test_singleton(
        ctx: &mut SpendContext,
        sim: &mut Simulator,
    ) -> Result<(Bytes32, Coin, Proof, NodePtr, Bytes32, NodePtr), DriverError> {
        let test_singleton_launcher_coin = sim.new_coin(SINGLETON_LAUNCHER_PUZZLE_HASH.into(), 1);
        let test_singleton_launcher =
            Launcher::new(test_singleton_launcher_coin.parent_coin_info, 1);

        let test_singleton_launcher_id = test_singleton_launcher.coin().coin_id();

        let test_singleton_inner_puzzle = ctx.alloc(&1)?;
        let test_singleton_inner_puzzle_hash = ctx.tree_hash(test_singleton_inner_puzzle);
        let (_, test_singleton_coin) =
            test_singleton_launcher.spend(ctx, test_singleton_inner_puzzle_hash.into(), ())?;

        let test_singleton_puzzle = ctx.curry(SingletonArgs::new(
            test_singleton_launcher_id,
            test_singleton_inner_puzzle,
        ))?;
        let test_singleton_proof: Proof = Proof::Eve(EveProof {
            parent_parent_coin_info: test_singleton_launcher_coin.parent_coin_info,
            parent_amount: test_singleton_launcher_coin.amount,
        });

        Ok((
            test_singleton_launcher_id,
            test_singleton_coin,
            test_singleton_proof,
            test_singleton_inner_puzzle,
            test_singleton_inner_puzzle_hash.into(),
            test_singleton_puzzle,
        ))
    }

    // Spends the price singleton to update the price of a registry
    fn spend_price_singleton<S>(
        ctx: &mut SpendContext,
        price_singleton_coin: Coin,
        price_singleton_proof: Proof,
        price_singleton_puzzle: NodePtr,
        new_state: S,
        receiver_puzzle_hash: Bytes32,
    ) -> Result<(Coin, Proof, DelegatedStateActionSolution<S>), DriverError>
    where
        S: ToTreeHash,
    {
        let price_singleton_inner_puzzle = ctx.alloc(&1)?;
        let price_singleton_inner_puzzle_hash = ctx.tree_hash(price_singleton_inner_puzzle);

        let message: Bytes32 = new_state.tree_hash().into();
        let price_singleton_inner_solution = Conditions::new()
            .send_message(
                18,
                message.into(),
                vec![receiver_puzzle_hash.to_clvm(&mut ctx.allocator)?],
            )
            .create_coin(price_singleton_inner_puzzle_hash.into(), 1, None);

        let price_singleton_inner_solution =
            price_singleton_inner_solution.to_clvm(&mut ctx.allocator)?;
        let price_singleton_solution = SingletonSolution {
            lineage_proof: price_singleton_proof,
            amount: 1,
            inner_solution: price_singleton_inner_solution,
        }
        .to_clvm(&mut ctx.allocator)?;

        let price_singleton_spend = Spend::new(price_singleton_puzzle, price_singleton_solution);
        ctx.spend(price_singleton_coin, price_singleton_spend)?;

        // compute price singleton info for next spend
        let next_price_singleton_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: price_singleton_coin.parent_coin_info,
            parent_inner_puzzle_hash: price_singleton_inner_puzzle_hash.into(),
            parent_amount: price_singleton_coin.amount,
        });
        let next_price_singleton_coin = Coin::new(
            price_singleton_coin.coin_id(),
            price_singleton_coin.puzzle_hash,
            1,
        );

        Ok((
            next_price_singleton_coin,
            next_price_singleton_proof,
            DelegatedStateActionSolution {
                new_state,
                other_singleton_inner_puzzle_hash: price_singleton_inner_puzzle_hash.into(),
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn test_refund_for_catalog(
        ctx: &mut SpendContext,
        sim: &mut Simulator,
        reg_amount: u64,
        payment_cat: Cat,
        tail_puzzle_to_refund: Option<NodePtr>,
        catalog: CatalogRegistry,
        catalog_constants: &CatalogRegistryConstants,
        slots: &[Slot<CatalogSlotValue>],
        user_puzzle_hash: Bytes32,
        minter_p2: StandardLayer,
        minter_puzzle_hash: Bytes32,
        sks: &[SecretKey; 2],
    ) -> anyhow::Result<(CatalogRegistry, Cat)> {
        // create precommit coin
        let user_coin = sim.new_coin(user_puzzle_hash, reg_amount);
        // pretty much a random TAIL - we're not actually launching it
        let tail = if let Some(t) = tail_puzzle_to_refund {
            t
        } else {
            ctx.curry(GenesisByCoinIdTailArgs::new(user_coin.coin_id()))?
        };
        let tail_hash = ctx.tree_hash(tail);
        // doesn't matter - we're getting refudned anyway
        let eve_nft_inner_puzzle_hash = tail_hash;

        let value = CatalogPrecommitValue::with_default_cat_maker(
            payment_cat.asset_id.tree_hash(),
            eve_nft_inner_puzzle_hash.into(),
            tail,
        );

        let refund_puzzle = ctx.alloc(&1)?;
        let refund_puzzle_hash = ctx.tree_hash(refund_puzzle);
        let precommit_coin = PrecommitCoin::new(
            ctx,
            payment_cat.coin.coin_id(),
            payment_cat.child_lineage_proof(),
            payment_cat.asset_id,
            SingletonStruct::new(catalog.info.launcher_id)
                .tree_hash()
                .into(),
            catalog_constants.relative_block_height,
            catalog_constants.precommit_payout_puzzle_hash,
            refund_puzzle_hash.into(),
            value,
            reg_amount,
        )?;

        let payment_cat_inner_spend = minter_p2.spend_with_conditions(
            ctx,
            Conditions::new()
                .create_coin(precommit_coin.inner_puzzle_hash, reg_amount, None)
                .create_coin(
                    minter_puzzle_hash,
                    payment_cat.coin.amount - reg_amount,
                    None,
                ),
        )?;
        Cat::spend_all(
            ctx,
            &[CatSpend {
                cat: payment_cat,
                inner_spend: payment_cat_inner_spend,
                extra_delta: 0,
            }],
        )?;

        let new_payment_cat =
            payment_cat.wrapped_child(minter_puzzle_hash, payment_cat.coin.amount - reg_amount);

        sim.spend_coins(ctx.take(), sks)?;

        let slot = slots
            .iter()
            .find(|s| s.info.value.unwrap().asset_id == tail_hash.into());

        let (secure_cond, new_catalog) = catalog.refund(
            ctx,
            tail_hash.into(),
            if let Some(found_slot) = slot {
                found_slot.info.value.unwrap().neighbors.tree_hash().into()
            } else {
                Bytes32::default()
            },
            precommit_coin,
            slot.cloned(),
        )?;

        let sec_puzzle = clvm_quote!(secure_cond.clone()).to_clvm(&mut ctx.allocator)?;
        let sec_coin = sim.new_coin(ctx.tree_hash(sec_puzzle).into(), 0);

        let sec_program = ctx.serialize(&sec_puzzle)?;
        let solution_program = ctx.serialize(&NodePtr::NIL)?;
        ctx.insert(CoinSpend::new(sec_coin, sec_program, solution_program));

        sim.spend_coins(ctx.take(), sks)?;

        Ok((new_catalog, new_payment_cat))
    }

    #[test]
    fn test_catalog() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        // setup config

        let initial_registration_price = 2000;
        let test_price_schedule = [1000, 500, 250];

        let catalog_constants = CatalogRegistryConstants {
            royalty_address: Bytes32::from([7; 32]),
            royalty_ten_thousandths: 100,
            precommit_payout_puzzle_hash: Bytes32::from([8; 32]),
            relative_block_height: 1,
            price_singleton_launcher_id: Bytes32::from(hex!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
        };

        // Create source offer
        let [launcher_sk, user_sk]: [SecretKey; 2] = test_secret_keys(2)?.try_into().unwrap();

        let launcher_pk = launcher_sk.public_key();
        let launcher_puzzle_hash = StandardArgs::curry_tree_hash(launcher_pk).into();

        let user_pk = user_sk.public_key();
        let user_puzzle_hash = StandardArgs::curry_tree_hash(user_pk).into();

        let offer_amount = 1;
        let offer_src_coin = sim.new_coin(launcher_puzzle_hash, offer_amount);
        let offer_spend = StandardLayer::new(launcher_pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(
                SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                offer_amount,
                None,
            ),
        )?;

        let puzzle_reveal = ctx.serialize(&offer_spend.puzzle)?;
        let solution = ctx.serialize(&offer_spend.solution)?;
        let offer = Offer::new(SpendBundle {
            coin_spends: vec![CoinSpend::new(offer_src_coin, puzzle_reveal, solution)],
            aggregated_signature: sign_standard_transaction(
                ctx,
                offer_src_coin,
                offer_spend,
                &launcher_sk,
                &TESTNET11_CONSTANTS,
            )?,
        });

        let (
            price_singleton_launcher_id,
            mut price_singleton_coin,
            mut price_singleton_proof,
            _price_singleton_inner_puzzle,
            _price_singleton_inner_puzzle_hash,
            price_singleton_puzzle,
        ) = launch_test_singleton(ctx, &mut sim)?;

        // Launch test CAT
        let mut payment_cat_amount = 10_000_000;
        let (minter_sk, minter_pk, minter_puzzle_hash, minter_coin) =
            sim.new_p2(payment_cat_amount)?;
        let minter_p2 = StandardLayer::new(minter_pk);

        let (issue_cat, mut payment_cat) = Cat::single_issuance_eve(
            ctx,
            minter_coin.coin_id(),
            payment_cat_amount,
            Conditions::new().create_coin(minter_puzzle_hash, payment_cat_amount, None),
        )?;
        minter_p2.spend(ctx, minter_coin, issue_cat)?;

        payment_cat = payment_cat.wrapped_child(minter_puzzle_hash, payment_cat_amount);
        sim.spend_coins(ctx.take(), &[minter_sk.clone()])?;

        // Launch catalog
        let (_, security_sk, mut catalog, slots) = launch_catalog_registry(
            ctx,
            offer,
            initial_registration_price,
            payment_cat.asset_id,
            catalog_constants.with_price_singleton(price_singleton_launcher_id),
            &TESTNET11_CONSTANTS,
        )?;

        sim.spend_coins(ctx.take(), &[launcher_sk, security_sk])?;

        // Register CAT

        let mut tail: NodePtr = NodePtr::NIL; // will be used for refund as well
        let mut slots: Vec<Slot<CatalogSlotValue>> = slots.into();
        for i in 0..7 {
            // create precommit coin
            let reg_amount = if i % 2 == 1 {
                test_price_schedule[i / 2]
            } else {
                catalog.info.state.registration_price
            };
            let user_coin = sim.new_coin(user_puzzle_hash, reg_amount);
            // pretty much a random TAIL - we're not actually launching it
            tail = ctx.curry(GenesisByCoinIdTailArgs::new(user_coin.coin_id()))?;
            let tail_hash = ctx.tree_hash(tail);

            let eve_nft_inner_puzzle = clvm_quote!(Conditions::new().create_coin(
                Bytes32::new([4 + i as u8; 32]),
                1,
                None
            ))
            .to_clvm(&mut ctx.allocator)?;
            let eve_nft_inner_puzzle_hash = ctx.tree_hash(eve_nft_inner_puzzle);

            let value = CatalogPrecommitValue::with_default_cat_maker(
                payment_cat.asset_id.tree_hash(),
                eve_nft_inner_puzzle_hash.into(),
                tail,
            );

            let refund_puzzle = ctx.alloc(&1)?;
            let refund_puzzle_hash = ctx.tree_hash(refund_puzzle);
            let precommit_coin = PrecommitCoin::new(
                ctx,
                payment_cat.coin.coin_id(),
                payment_cat.child_lineage_proof(),
                payment_cat.asset_id,
                SingletonStruct::new(catalog.info.launcher_id)
                    .tree_hash()
                    .into(),
                catalog_constants.relative_block_height,
                catalog_constants.precommit_payout_puzzle_hash,
                refund_puzzle_hash.into(),
                value,
                reg_amount,
            )?;

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                Conditions::new()
                    .create_coin(precommit_coin.inner_puzzle_hash, reg_amount, None)
                    .create_coin(minter_puzzle_hash, payment_cat_amount - reg_amount, None),
            )?;
            Cat::spend_all(
                ctx,
                &[CatSpend {
                    cat: payment_cat,
                    inner_spend: payment_cat_inner_spend,
                    extra_delta: 0,
                }],
            )?;

            payment_cat_amount -= reg_amount;
            payment_cat = payment_cat.wrapped_child(minter_puzzle_hash, payment_cat_amount);

            sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;

            // call the 'register' action on CATalog
            slots.sort_unstable_by(|a, b| a.info.value.unwrap().cmp(&b.info.value.unwrap()));

            let slot_value_to_insert =
                CatalogSlotValue::new(tail_hash.into(), Bytes32::default(), Bytes32::default());

            let mut left_slot: Option<Slot<CatalogSlotValue>> = None;
            let mut right_slot: Option<Slot<CatalogSlotValue>> = None;
            for slot in slots.iter() {
                let slot_value = slot.info.value.unwrap();

                if slot_value < slot_value_to_insert {
                    // slot belongs to the left
                    if left_slot.is_none() || slot_value > left_slot.unwrap().info.value.unwrap() {
                        left_slot = Some(*slot);
                    }
                } else {
                    // slot belongs to the right
                    if right_slot.is_none() || slot_value < right_slot.unwrap().info.value.unwrap()
                    {
                        right_slot = Some(*slot);
                    }
                }
            }

            let (left_slot, right_slot) = (left_slot.unwrap(), right_slot.unwrap());

            if i % 2 == 1 {
                let new_price = reg_amount;
                assert_ne!(new_price, catalog.info.state.registration_price);

                let (
                    new_price_singleton_coin,
                    new_price_singleton_proof,
                    delegated_state_action_solution,
                ) = spend_price_singleton(
                    ctx,
                    price_singleton_coin,
                    price_singleton_proof,
                    price_singleton_puzzle,
                    CatalogRegistryState {
                        cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(
                            payment_cat.asset_id.tree_hash().into(),
                        )
                        .into(),
                        registration_price: new_price,
                    },
                    catalog.coin.puzzle_hash,
                )?;

                price_singleton_coin = new_price_singleton_coin;
                price_singleton_proof = new_price_singleton_proof;

                let update_action =
                    CatalogRegistryAction::UpdatePrice(delegated_state_action_solution);

                let catalog_coin = catalog.coin;
                let catalog_constants = catalog.info.constants;
                let spend = catalog.spend(ctx, vec![update_action])?;
                ctx.spend(catalog_coin, spend)?;

                sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

                let catalog_puzzle = Puzzle::parse(&ctx.allocator, spend.puzzle);
                if let Some(new_catalog) = CatalogRegistry::from_parent_spend(
                    &mut ctx.allocator,
                    catalog_coin,
                    catalog_puzzle,
                    spend.solution,
                    catalog_constants,
                )? {
                    catalog = new_catalog;
                } else {
                    panic!("Couldn't parse CATalog after price was updated");
                };
            };

            let (secure_cond, new_catalog, new_slots) = catalog.register_cat(
                ctx,
                tail_hash.into(),
                left_slot,
                right_slot,
                precommit_coin,
                Spend {
                    puzzle: eve_nft_inner_puzzle,
                    solution: NodePtr::NIL,
                },
            )?;

            let funds_puzzle = clvm_quote!(secure_cond.clone()).to_clvm(&mut ctx.allocator)?;
            let funds_coin = sim.new_coin(ctx.tree_hash(funds_puzzle).into(), 1);

            let funds_program = ctx.serialize(&funds_puzzle)?;
            let solution_program = ctx.serialize(&NodePtr::NIL)?;
            ctx.insert(CoinSpend::new(funds_coin, funds_program, solution_program));

            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != left_slot && *s != right_slot);
            slots.extend(new_slots);

            catalog = new_catalog;
        }

        assert_eq!(
            catalog.info.state.registration_price,
            test_price_schedule[2], // 1, 3, 5 updated the price
        );

        // Test refunds

        // b - the amount is wrong (by one)
        let (catalog, payment_cat) = test_refund_for_catalog(
            ctx,
            &mut sim,
            catalog.info.state.registration_price + 1,
            payment_cat,
            None,
            catalog,
            &catalog_constants,
            &slots,
            user_puzzle_hash,
            minter_p2,
            minter_puzzle_hash,
            &[user_sk.clone(), minter_sk.clone()],
        )?;

        // a - the CAT maker puzzle has changed
        // i.e., use different payment CAT
        let alternative_payment_cat_amount = 10_000_000;
        let (minter2_sk, minter2_pk, minter2_puzzle_hash, minter2_coin) =
            sim.new_p2(alternative_payment_cat_amount)?;
        let minter_p2_2 = StandardLayer::new(minter2_pk);

        let (issue_cat, mut alternative_payment_cat) = Cat::single_issuance_eve(
            ctx,
            minter2_coin.coin_id(),
            alternative_payment_cat_amount,
            Conditions::new().create_coin(
                minter2_puzzle_hash,
                alternative_payment_cat_amount,
                None,
            ),
        )?;
        minter_p2_2.spend(ctx, minter2_coin, issue_cat)?;

        alternative_payment_cat = alternative_payment_cat
            .wrapped_child(minter2_puzzle_hash, alternative_payment_cat_amount);
        sim.spend_coins(ctx.take(), &[minter2_sk.clone()])?;

        let (catalog, _alternative_payment_cat) = test_refund_for_catalog(
            ctx,
            &mut sim,
            catalog.info.state.registration_price,
            alternative_payment_cat,
            None,
            catalog,
            &catalog_constants,
            &slots,
            user_puzzle_hash,
            minter_p2_2,
            minter2_puzzle_hash,
            &[user_sk.clone(), minter2_sk.clone()],
        )?;

        // c - the tail hash has already been registered
        let (_catalog, _payment_cat) = test_refund_for_catalog(
            ctx,
            &mut sim,
            catalog.info.state.registration_price,
            payment_cat,
            Some(tail),
            catalog,
            &catalog_constants,
            &slots,
            user_puzzle_hash,
            minter_p2,
            minter_puzzle_hash,
            &[user_sk.clone(), minter_sk.clone()],
        )?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn test_refund_for_xchandles(
        ctx: &mut SpendContext,
        sim: &mut Simulator,
        handle_to_refund: String,
        pricing_puzzle: NodePtr,
        pricing_solution: NodePtr,
        slot: Option<Slot<XchandlesSlotValue>>,
        payment_cat: Cat,
        payment_cat_amount: u64,
        registry: XchandlesRegistry,
        minter_p2: StandardLayer,
        minter_puzzle_hash: Bytes32,
        minter_sk: &SecretKey,
        user_sk: &SecretKey,
    ) -> anyhow::Result<(XchandlesRegistry, Cat)> {
        let pricing_puzzle_hash = ctx.tree_hash(pricing_puzzle);
        let pricing_solution_hash = ctx.tree_hash(pricing_solution);

        let value = XchandlesPrecommitValue::for_normal_registration(
            payment_cat.asset_id.tree_hash(),
            pricing_puzzle_hash,
            pricing_solution_hash,
            Bytes32::default(),
            handle_to_refund.clone(),
            0,
            Bytes32::default(),
            Bytes32::default(),
        );

        let refund_puzzle = ctx.alloc(&1)?;
        let refund_puzzle_hash = ctx.tree_hash(refund_puzzle);
        let precommit_coin = PrecommitCoin::new(
            ctx,
            payment_cat.coin.coin_id(),
            payment_cat.child_lineage_proof(),
            payment_cat.asset_id,
            SingletonStruct::new(registry.info.launcher_id)
                .tree_hash()
                .into(),
            registry.info.constants.relative_block_height,
            registry.info.constants.precommit_payout_puzzle_hash,
            refund_puzzle_hash.into(),
            value,
            payment_cat_amount,
        )?;

        let payment_cat_inner_spend = minter_p2.spend_with_conditions(
            ctx,
            Conditions::new()
                .create_coin(precommit_coin.inner_puzzle_hash, payment_cat_amount, None)
                .create_coin(
                    minter_puzzle_hash,
                    payment_cat.coin.amount - payment_cat_amount,
                    None,
                ),
        )?;
        Cat::spend_all(
            ctx,
            &[CatSpend {
                cat: payment_cat,
                inner_spend: payment_cat_inner_spend,
                extra_delta: 0,
            }],
        )?;

        let new_payment_cat = payment_cat.wrapped_child(
            minter_puzzle_hash,
            payment_cat.coin.amount - payment_cat_amount,
        );

        sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;

        let (secure_cond, new_registry) =
            registry.refund(ctx, precommit_coin, pricing_puzzle, pricing_solution, slot)?;

        let sec_puzzle = clvm_quote!(secure_cond.clone()).to_clvm(&mut ctx.allocator)?;
        let sec_coin = sim.new_coin(ctx.tree_hash(sec_puzzle).into(), 0);

        let sec_program = ctx.serialize(&sec_puzzle)?;
        let solution_program = ctx.serialize(&NodePtr::NIL)?;
        ctx.insert(CoinSpend::new(sec_coin, sec_program, solution_program));

        sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

        Ok((new_registry, new_payment_cat))
    }

    #[test]
    fn test_xchandles() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        // setup config
        let initial_registration_price = 2000;
        let test_price_schedule = [1000, 500, 250];

        let xchandles_constants = XchandlesConstants {
            precommit_payout_puzzle_hash: Bytes32::from([8; 32]),
            relative_block_height: 1,
            price_singleton_launcher_id: Bytes32::from(hex!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
        };

        // Create source offer
        let [launcher_sk, user_sk]: [SecretKey; 2] = test_secret_keys(2)?.try_into().unwrap();

        let launcher_pk = launcher_sk.public_key();
        let launcher_puzzle_hash = StandardArgs::curry_tree_hash(launcher_pk).into();

        let user_pk = user_sk.public_key();
        let user_puzzle = StandardLayer::new(user_pk);
        let user_puzzle_hash: Bytes32 = StandardArgs::curry_tree_hash(user_pk).into();

        let offer_amount = 1;
        let offer_src_coin = sim.new_coin(launcher_puzzle_hash, offer_amount);
        let offer_spend = StandardLayer::new(launcher_pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(
                SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                offer_amount,
                None,
            ),
        )?;

        let puzzle_reveal = ctx.serialize(&offer_spend.puzzle)?;
        let solution = ctx.serialize(&offer_spend.solution)?;
        let offer = Offer::new(SpendBundle {
            coin_spends: vec![CoinSpend::new(offer_src_coin, puzzle_reveal, solution)],
            aggregated_signature: sign_standard_transaction(
                ctx,
                offer_src_coin,
                offer_spend,
                &launcher_sk,
                &TESTNET11_CONSTANTS,
            )?,
        });

        // Launch CAT
        let mut payment_cat_amount = 10_000_000;
        let (minter_sk, minter_pk, minter_puzzle_hash, minter_coin) =
            sim.new_p2(payment_cat_amount)?;
        let minter_p2 = StandardLayer::new(minter_pk);

        let (issue_cat, mut payment_cat) = Cat::single_issuance_eve(
            ctx,
            minter_coin.coin_id(),
            payment_cat_amount,
            Conditions::new().create_coin(minter_puzzle_hash, payment_cat_amount, None),
        )?;
        minter_p2.spend(ctx, minter_coin, issue_cat)?;

        payment_cat = payment_cat.wrapped_child(minter_puzzle_hash, payment_cat_amount);
        sim.spend_coins(ctx.take(), &[minter_sk.clone()])?;

        // Launch price singleton
        let (
            price_singleton_launcher_id,
            mut price_singleton_coin,
            mut price_singleton_proof,
            _price_singleton_inner_puzzle,
            _price_singleton_inner_puzzle_hash,
            price_singleton_puzzle,
        ) = launch_test_singleton(ctx, &mut sim)?;

        // Launch XCHandles
        let (_, security_sk, mut registry, slots) = launch_xchandles_registry(
            ctx,
            offer,
            initial_registration_price,
            payment_cat.asset_id,
            xchandles_constants.with_price_singleton(price_singleton_launcher_id),
            &TESTNET11_CONSTANTS,
        )?;

        sim.spend_coins(ctx.take(), &[launcher_sk, security_sk])?;

        // Register 7 handles

        let mut base_price = initial_registration_price;

        let mut slots: Vec<Slot<XchandlesSlotValue>> = slots.into();
        for i in 0..7 {
            // mint controller singleton (it's a DID, not an NFT - don't rat on me to the NFT board plz)
            let launcher_coin = sim.new_coin(SINGLETON_LAUNCHER_PUZZLE_HASH.into(), 1);
            let launcher = Launcher::new(launcher_coin.parent_coin_info, 1);
            let (_, did) = launcher.create_simple_did(ctx, &user_puzzle)?;

            // name is "aa" + "a" * i + "{i}"
            let handle = if i == 0 {
                "aa0".to_string()
            } else {
                "aa".to_string() + &"a".repeat(i).to_string() + &i.to_string()
            };
            let handle_hash: Bytes32 = handle.tree_hash().into();

            // create precommit coin
            if i % 2 == 1 {
                base_price = test_price_schedule[i / 2];
            };
            let reg_amount = XchandlesFactorPricingPuzzleArgs::get_price(base_price, &handle, 1);

            let handle_owner_launcher_id = did.info.launcher_id;
            let handle_resolved_launcher_id = Bytes32::from([u8::MAX - i as u8; 32]);
            let secret = Bytes32::default();

            let value = XchandlesPrecommitValue::for_normal_registration(
                payment_cat.asset_id.tree_hash(),
                XchandlesFactorPricingPuzzleArgs::curry_tree_hash(base_price),
                XchandlesFactorPricingSolution {
                    handle: handle.clone(),
                    num_years: 1,
                }
                .tree_hash(),
                secret,
                handle.clone(),
                100,
                handle_owner_launcher_id,
                handle_resolved_launcher_id,
            );

            let refund_puzzle = ctx.alloc(&1)?;
            let refund_puzzle_hash = ctx.tree_hash(refund_puzzle);
            let precommit_coin = PrecommitCoin::new(
                ctx,
                payment_cat.coin.coin_id(),
                payment_cat.child_lineage_proof(),
                payment_cat.asset_id,
                SingletonStruct::new(registry.info.launcher_id)
                    .tree_hash()
                    .into(),
                xchandles_constants.relative_block_height,
                xchandles_constants.precommit_payout_puzzle_hash,
                refund_puzzle_hash.into(),
                value,
                reg_amount,
            )?;

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                Conditions::new()
                    .create_coin(precommit_coin.inner_puzzle_hash, reg_amount, None)
                    .create_coin(minter_puzzle_hash, payment_cat_amount - reg_amount, None),
            )?;
            Cat::spend_all(
                ctx,
                &[CatSpend {
                    cat: payment_cat,
                    inner_spend: payment_cat_inner_spend,
                    extra_delta: 0,
                }],
            )?;

            payment_cat_amount -= reg_amount;
            payment_cat = payment_cat.wrapped_child(minter_puzzle_hash, payment_cat_amount);

            sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;
            // call the 'register' action on CNS
            slots.sort_unstable_by(|a, b| a.info.value.unwrap().cmp(&b.info.value.unwrap()));

            let slot_value_to_insert = XchandlesSlotValue::new(
                handle_hash,
                Bytes32::default(),
                Bytes32::default(),
                0,
                Bytes32::default(),
                Bytes32::default(),
            );

            let mut left_slot: Option<Slot<XchandlesSlotValue>> = None;
            let mut right_slot: Option<Slot<XchandlesSlotValue>> = None;
            for slot in slots.iter() {
                let slot_value = slot.info.value.unwrap();

                if slot_value < slot_value_to_insert {
                    // slot belongs to the left
                    if left_slot.is_none() || slot_value > left_slot.unwrap().info.value.unwrap() {
                        left_slot = Some(*slot);
                    }
                } else {
                    // slot belongs to the right
                    if right_slot.is_none() || slot_value < right_slot.unwrap().info.value.unwrap()
                    {
                        right_slot = Some(*slot);
                    }
                }
            }

            let (left_slot, right_slot) = (left_slot.unwrap(), right_slot.unwrap());

            // update price
            if i % 2 == 1 {
                let new_price = test_price_schedule[i / 2];
                let new_price_puzzle_hash: Bytes32 =
                    XchandlesFactorPricingPuzzleArgs::curry_tree_hash(new_price).into();
                assert_ne!(
                    new_price_puzzle_hash,
                    registry.info.state.pricing_puzzle_hash
                );

                let (
                    new_price_singleton_coin,
                    new_price_singleton_proof,
                    delegated_state_action_solution,
                ) = spend_price_singleton(
                    ctx,
                    price_singleton_coin,
                    price_singleton_proof,
                    price_singleton_puzzle,
                    XchandlesRegistryState::from(
                        payment_cat.asset_id.tree_hash().into(),
                        new_price,
                    ),
                    registry.coin.puzzle_hash,
                )?;

                price_singleton_coin = new_price_singleton_coin;
                price_singleton_proof = new_price_singleton_proof;

                let update_action =
                    XchandlesRegistryAction::UpdateState(delegated_state_action_solution);

                let registry_constants = registry.info.constants;
                let registry_coin = registry.coin;
                let spend = registry.spend(ctx, vec![update_action])?;
                ctx.spend(registry_coin, spend)?;

                sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

                let registry_puzzle = Puzzle::parse(&ctx.allocator, spend.puzzle);
                if let Some(new_registry) = XchandlesRegistry::from_parent_spend(
                    &mut ctx.allocator,
                    registry_coin,
                    registry_puzzle,
                    spend.solution,
                    registry_constants,
                )? {
                    registry = new_registry;
                } else {
                    panic!("Couldn't get registry after price was updated");
                };
            };

            let (secure_cond, new_registry, new_slots) =
                registry.register_handle(ctx, left_slot, right_slot, precommit_coin, base_price)?;

            let funds_puzzle = clvm_quote!(secure_cond.clone()).to_clvm(&mut ctx.allocator)?;
            let funds_coin = sim.new_coin(ctx.tree_hash(funds_puzzle).into(), 1);

            let funds_program = ctx.serialize(&funds_puzzle)?;
            let solution_program = ctx.serialize(&NodePtr::NIL)?;
            ctx.insert(CoinSpend::new(funds_coin, funds_program, solution_program));

            sim.pass_time(100); // registration start was at timestamp 100
            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != left_slot && *s != right_slot);

            let oracle_slot = new_slots[1];
            slots.extend(new_slots);

            registry = new_registry;

            // test on-chain oracle for current handle
            let (oracle_conds, new_registry, new_slots) = registry.oracle(ctx, oracle_slot)?;

            let user_coin = sim.new_coin(user_puzzle_hash, 0);
            StandardLayer::new(user_pk).spend(ctx, user_coin, oracle_conds)?;

            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != oracle_slot);
            slots.extend(new_slots.clone());

            registry = new_registry;

            // test on-chain extend mechanism for current handle
            let extension_years: u64 = i as u64 + 1;
            let extension_slot = new_slots[0];
            let pay_for_extension: u64 =
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &handle, extension_years);

            let (notarized_payment, extend_conds, new_registry, new_slots) = registry.extend(
                ctx,
                handle,
                extension_slot,
                payment_cat.asset_id,
                base_price,
                extension_years,
            )?;

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                extend_conds
                    .create_coin(
                        SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                        pay_for_extension,
                        None,
                    )
                    .create_coin(
                        minter_puzzle_hash,
                        payment_cat_amount - pay_for_extension,
                        None,
                    ),
            )?;

            let cat_offer_inner_spend = Spend::new(
                ctx.settlement_payments_puzzle()?,
                clvm_list!(notarized_payment).to_clvm(&mut ctx.allocator)?,
            );

            Cat::spend_all(
                ctx,
                &[
                    CatSpend {
                        cat: payment_cat,
                        inner_spend: payment_cat_inner_spend,
                        extra_delta: 0,
                    },
                    CatSpend {
                        cat: payment_cat.wrapped_child(
                            SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                            pay_for_extension,
                        ),
                        inner_spend: cat_offer_inner_spend,
                        extra_delta: 0,
                    },
                ],
            )?;

            payment_cat_amount -= pay_for_extension;
            payment_cat = payment_cat.wrapped_child(minter_puzzle_hash, payment_cat_amount);

            sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;

            slots.retain(|s| *s != extension_slot);
            slots.extend(new_slots.clone());

            registry = new_registry;

            // test on-chain mechanism for handle updates
            let new_owner_launcher_id = Bytes32::new([4 + i as u8; 32]);
            let new_resolved_launcher_id = Bytes32::new([u8::MAX - i as u8 - 1; 32]);
            let update_slot = new_slots[0];

            let (update_conds, new_registry, new_slots) = registry.update(
                ctx,
                update_slot,
                new_owner_launcher_id,
                new_resolved_launcher_id,
                did.info.inner_puzzle_hash().into(),
            )?;

            let _new_did = did.update(ctx, &user_puzzle, update_conds)?;

            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != update_slot);
            slots.extend(new_slots.clone());

            registry = new_registry;
        }

        assert_eq!(
            registry.info.state.pricing_puzzle_hash,
            // iterations 1, 3, 5 updated the price
            XchandlesFactorPricingPuzzleArgs::curry_tree_hash(test_price_schedule[2]).into(),
        );

        // expire one of the slots
        let handle_to_expire = "aa0".to_string();
        let handle_hash: Bytes32 = handle_to_expire.tree_hash().into();
        let initial_slot = slots
            .iter()
            .find(|s| s.info.value.unwrap().handle_hash == handle_hash)
            .unwrap();

        // precommit coin needed
        let refund_puzzle = ctx.alloc(&1)?;
        let refund_puzzle_hash = ctx.tree_hash(refund_puzzle);
        let expiration = initial_slot.info.value.unwrap().expiration;
        let buy_time = expiration + 27 * 24 * 60 * 60; // last day of auction; 0 < premium < 1 CAT
        let value = XchandlesPrecommitValue::for_normal_registration(
            payment_cat.asset_id.tree_hash(),
            XchandlesExponentialPremiumRenewPuzzleArgs::curry_tree_hash(base_price, 1000),
            XchandlesExponentialPremiumRenewPuzzleSolution {
                buy_time,
                expiration,
                pricing_program_solution: XchandlesFactorPricingSolution {
                    handle: handle_to_expire.clone(),
                    num_years: 1,
                },
            }
            .tree_hash(),
            Bytes32::default(),
            handle_to_expire.clone(),
            buy_time,
            Bytes32::from([42; 32]),
            Bytes32::from([69; 32]),
        );

        let pricing_puzzle =
            XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(ctx, base_price, 1000)?;
        let reg_amount =
            pricing_puzzle.get_price(ctx, handle_to_expire, expiration, buy_time, 1)? as u64;

        let precommit_coin = PrecommitCoin::<XchandlesPrecommitValue>::new(
            ctx,
            payment_cat.coin.coin_id(),
            payment_cat.child_lineage_proof(),
            payment_cat.asset_id,
            SingletonStruct::new(registry.info.launcher_id)
                .tree_hash()
                .into(),
            xchandles_constants.relative_block_height,
            xchandles_constants.precommit_payout_puzzle_hash,
            refund_puzzle_hash.into(),
            value,
            reg_amount,
        )?;
        assert!(reg_amount <= payment_cat_amount);

        let payment_cat_inner_spend = minter_p2.spend_with_conditions(
            ctx,
            Conditions::new()
                .create_coin(precommit_coin.inner_puzzle_hash, reg_amount, None)
                .create_coin(minter_puzzle_hash, payment_cat_amount - reg_amount, None),
        )?;
        Cat::spend_all(
            ctx,
            &[CatSpend {
                cat: payment_cat,
                inner_spend: payment_cat_inner_spend,
                extra_delta: 0,
            }],
        )?;

        payment_cat =
            payment_cat.wrapped_child(minter_puzzle_hash, payment_cat.coin.amount - reg_amount);

        sim.set_next_timestamp(buy_time)?;
        sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;

        let (expire_conds, new_registry, _new_slots) =
            registry.expire_handle(ctx, *initial_slot, 1, base_price, precommit_coin)?;

        // assert expire conds
        let conds_puzzle = clvm_quote!(expire_conds).to_clvm(&mut ctx.allocator)?;
        let conds_coin = sim.new_coin(ctx.tree_hash(conds_puzzle).into(), 1);

        let conds_program = ctx.serialize(&conds_puzzle)?;
        let conds_solution_program = ctx.serialize(&NodePtr::NIL)?;
        ctx.insert(CoinSpend::new(
            conds_coin,
            conds_program,
            conds_solution_program,
        ));

        sim.spend_coins(ctx.take(), &[user_sk.clone()])?;
        registry = new_registry;

        // Test refunds
        let unregistered_handle = "yak7".to_string();

        for use_factor_pricing in [true, false] {
            let pricing_puzzle = if use_factor_pricing {
                XchandlesFactorPricingPuzzleArgs::get_puzzle(ctx, base_price)?
            } else {
                XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
                    ctx, base_price, 1000,
                )?
                .get_puzzle(ctx)?
            };
            let pricing_solution = if use_factor_pricing {
                XchandlesFactorPricingSolution {
                    handle: unregistered_handle.clone(),
                    num_years: 1,
                }
                .to_clvm(&mut ctx.allocator)?
            } else {
                XchandlesExponentialPremiumRenewPuzzleSolution {
                    buy_time: 28 * 24 * 60 * 60 + 1, // premium should be 0
                    expiration: 0,
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        handle: unregistered_handle.clone(),
                        num_years: 1,
                    },
                }
                .to_clvm(&mut ctx.allocator)?
            };

            let expected_price =
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &unregistered_handle, 1);
            let other_pricing_puzzle = if use_factor_pricing {
                XchandlesFactorPricingPuzzleArgs::get_puzzle(ctx, base_price + 1)?
            } else {
                XchandlesExponentialPremiumRenewPuzzleArgs::from_scale_factor(
                    ctx,
                    base_price + 1,
                    1000,
                )?
                .get_puzzle(ctx)?
            };
            let other_expected_price = XchandlesFactorPricingPuzzleArgs::get_price(
                base_price + 1,
                &unregistered_handle,
                1,
            );
            assert_ne!(other_expected_price, expected_price);

            let existing_handle = if use_factor_pricing {
                "aaa1".to_string()
            } else {
                "aaaa2".to_string()
            };
            let existing_slot = *slots
                .iter()
                .find(|s| s.info.value.unwrap().handle_hash == existing_handle.tree_hash().into())
                .unwrap();
            let existing_handle_pricing_solution = if use_factor_pricing {
                XchandlesFactorPricingSolution {
                    handle: existing_handle.clone(),
                    num_years: 1,
                }
                .to_clvm(&mut ctx.allocator)?
            } else {
                XchandlesExponentialPremiumRenewPuzzleSolution {
                    buy_time: existing_slot.info.value.unwrap().expiration + 28 * 24 * 60 * 60 + 1, // premium should be 0
                    expiration: existing_slot.info.value.unwrap().expiration,
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        handle: existing_handle.clone(),
                        num_years: 1,
                    },
                }
                .to_clvm(&mut ctx.allocator)?
            };
            let existing_handle_expected_price =
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &existing_handle, 1);

            // a - the CAT maker puzzle has changed
            let alternative_payment_cat_amount = 10_000_000;
            let (minter2_sk, minter2_pk, minter2_puzzle_hash, minter2_coin) =
                sim.new_p2(alternative_payment_cat_amount)?;
            let minter_p2_2 = StandardLayer::new(minter2_pk);

            let (issue_cat, mut alternative_payment_cat) = Cat::single_issuance_eve(
                ctx,
                minter2_coin.coin_id(),
                alternative_payment_cat_amount,
                Conditions::new().create_coin(
                    minter2_puzzle_hash,
                    alternative_payment_cat_amount,
                    None,
                ),
            )?;
            minter_p2_2.spend(ctx, minter2_coin, issue_cat)?;

            alternative_payment_cat = alternative_payment_cat
                .wrapped_child(minter2_puzzle_hash, alternative_payment_cat_amount);
            sim.spend_coins(ctx.take(), &[minter2_sk.clone()])?;

            registry = test_refund_for_xchandles(
                ctx,
                &mut sim,
                unregistered_handle.clone(),
                pricing_puzzle,
                pricing_solution,
                None,
                alternative_payment_cat,
                expected_price,
                registry,
                minter_p2_2,
                minter2_puzzle_hash,
                &minter2_sk,
                &user_sk,
            )?
            .0;

            // b - the amount is wrong
            (registry, payment_cat) = test_refund_for_xchandles(
                ctx,
                &mut sim,
                unregistered_handle.clone(),
                pricing_puzzle,
                pricing_solution,
                None,
                payment_cat,
                expected_price + 1,
                registry,
                minter_p2,
                minter_puzzle_hash,
                &minter_sk,
                &user_sk,
            )?;

            // c - the pricing puzzle has changed
            (registry, payment_cat) = test_refund_for_xchandles(
                ctx,
                &mut sim,
                unregistered_handle.clone(),
                other_pricing_puzzle,
                pricing_solution,
                None,
                payment_cat,
                other_expected_price,
                registry,
                minter_p2,
                minter_puzzle_hash,
                &minter_sk,
                &user_sk,
            )?;

            // d - the handle has already been registered
            (registry, payment_cat) = test_refund_for_xchandles(
                ctx,
                &mut sim,
                existing_handle.clone(), // already registered handle
                pricing_puzzle,
                existing_handle_pricing_solution,
                Some(existing_slot),
                payment_cat,
                existing_handle_expected_price,
                registry,
                minter_p2,
                minter_puzzle_hash,
                &minter_sk,
                &user_sk,
            )?;
        }

        Ok(())
    }

    #[test]
    fn test_nft_with_any_metadata_updater() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        let (sk, pk, p2_puzzle_hash, coin) = sim.new_p2(1)?;
        let p2 = StandardLayer::new(pk);

        let nft_launcher = Launcher::new(coin.coin_id(), 1);

        let royalty_puzzle_hash = Bytes32::from([7; 32]);
        let (create_nft, nft) = nft_launcher.mint_nft(
            ctx,
            NftMint::<CatNftMetadata> {
                metadata: cat_nft_metadata_for_testing(),
                metadata_updater_puzzle_hash: ANY_METADATA_UPDATER_HASH.into(),
                royalty_puzzle_hash,
                royalty_ten_thousandths: 100,
                p2_puzzle_hash,
                owner: None,
            },
        )?;
        p2.spend(ctx, coin, create_nft)?;

        // actually try to run updater
        let new_metadata = CatNftMetadata {
            code: "XXX".to_string(),
            name: "Test Name".to_string(),
            description: "Test desc".to_string(),
            precision: 4,
            image_uris: vec!["img URI".to_string()],
            image_hash: Bytes32::from([31; 32]),
            metadata_uris: vec!["meta URI".to_string()],
            metadata_hash: Bytes32::from([8; 32]),
            license_uris: vec!["license URI".to_string()],
            license_hash: Bytes32::from([9; 32]),
        };

        let metadata_update = Spend {
            puzzle: ctx.any_metadata_updater()?,
            solution: new_metadata.to_clvm(&mut ctx.allocator)?,
        };

        let new_nft: Nft<CatNftMetadata> = nft.transfer_with_metadata(
            ctx,
            &p2,
            p2_puzzle_hash,
            metadata_update,
            Conditions::new(),
        )?;

        assert_eq!(new_nft.info.metadata, new_metadata);
        sim.spend_coins(ctx.take(), &[sk])?;

        Ok(())
    }

    // Spends the validator singleton
    fn spend_validator_singleton(
        ctx: &mut SpendContext,
        test_singleton_coin: Coin,
        test_singleton_proof: Proof,
        test_singleton_puzzle: NodePtr,
        test_singleton_output_conditions: Conditions<NodePtr>,
    ) -> Result<(Coin, Proof), DriverError> {
        let test_singleton_inner_puzzle = ctx.alloc(&1)?;
        let test_singleton_inner_puzzle_hash = ctx.tree_hash(test_singleton_inner_puzzle);

        let test_singleton_inner_solution = test_singleton_output_conditions
            .create_coin(test_singleton_inner_puzzle_hash.into(), 1, None)
            .to_clvm(&mut ctx.allocator)?;
        let test_singleton_solution = SingletonSolution {
            lineage_proof: test_singleton_proof,
            amount: 1,
            inner_solution: test_singleton_inner_solution,
        }
        .to_clvm(&mut ctx.allocator)?;

        let test_singleton_spend = Spend::new(test_singleton_puzzle, test_singleton_solution);
        ctx.spend(test_singleton_coin, test_singleton_spend)?;

        // compute validator singleton info for next spend
        let next_test_singleton_proof = Proof::Lineage(LineageProof {
            parent_parent_coin_info: test_singleton_coin.parent_coin_info,
            parent_inner_puzzle_hash: test_singleton_inner_puzzle_hash.into(),
            parent_amount: test_singleton_coin.amount,
        });
        let next_test_singleton_coin = Coin::new(
            test_singleton_coin.coin_id(),
            test_singleton_coin.puzzle_hash,
            1,
        );

        Ok((next_test_singleton_coin, next_test_singleton_proof))
    }

    #[test]
    fn test_dig_reward_distributor() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        // Launch token CAT
        let mut cat_amount = 10_000_000_000;
        let (cat_minter_sk, cat_minter_pk, cat_minter_puzzle_hash, cat_minter_coin) =
            sim.new_p2(cat_amount)?;
        let cat_minter_p2 = StandardLayer::new(cat_minter_pk);

        let (issue_cat, mut source_cat) = Cat::single_issuance_eve(
            ctx,
            cat_minter_coin.coin_id(),
            cat_amount,
            Conditions::new().create_coin(cat_minter_puzzle_hash, cat_amount, None),
        )?;
        cat_minter_p2.spend(ctx, cat_minter_coin, issue_cat)?;

        source_cat = source_cat.wrapped_child(cat_minter_puzzle_hash, cat_amount);
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;

        // Launch validator singleton
        let (
            validator_launcher_id,
            mut validator_coin,
            mut validator_singleton_proof,
            validator_singleton_inner_puzzle,
            validator_singleton_inner_puzzle_hash,
            validator_singleton_puzzle,
        ) = launch_test_singleton(ctx, &mut sim)?;

        // setup config
        let constants = DigRewardDistributorConstants {
            validator_launcher_id,
            validator_payout_puzzle_hash: Bytes32::new([1; 32]),
            epoch_seconds: 1000,
            removal_max_seconds_offset: 30,
            payout_threshold: 10_000,
            validator_fee_bps: 420,     // 4.2% fee
            withdrawal_share_bps: 9000, // 90% of the amount deposited will be returned
            reserve_asset_id: source_cat.asset_id,
            reserve_inner_puzzle_hash: Bytes32::default(), // will be overwritten
            reserve_full_puzzle_hash: Bytes32::default(),  // will be overwritten
        };

        // Create source offer
        let [launcher_sk, mirror1_sk]: [SecretKey; 2] = test_secret_keys(2)?.try_into().unwrap();

        let launcher_pk = launcher_sk.public_key();
        let launcher_puzzle_hash = StandardArgs::curry_tree_hash(launcher_pk).into();

        let mirror1_pk = mirror1_sk.public_key();
        // let mirror1_puzzle = StandardLayer::new(mirror1_pk);
        let mirror1_puzzle_hash: Bytes32 = StandardArgs::curry_tree_hash(mirror1_pk).into();

        let offer_amount = 1;
        let offer_src_coin = sim.new_coin(launcher_puzzle_hash, offer_amount);
        let offer_spend = StandardLayer::new(launcher_pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(
                SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                offer_amount,
                None,
            ),
        )?;

        let puzzle_reveal = ctx.serialize(&offer_spend.puzzle)?;
        let solution = ctx.serialize(&offer_spend.solution)?;
        let offer = Offer::new(SpendBundle {
            coin_spends: vec![CoinSpend::new(offer_src_coin, puzzle_reveal, solution)],
            aggregated_signature: sign_standard_transaction(
                ctx,
                offer_src_coin,
                offer_spend,
                &launcher_sk,
                &TESTNET11_CONSTANTS,
            )?,
        });

        // Launch the DIG reward distributor
        let first_epoch_start = 1234;
        let (_, security_sk, mut registry, first_epoch_slot) = launch_dig_reward_distributor(
            ctx,
            offer,
            first_epoch_start,
            constants,
            &TESTNET11_CONSTANTS,
        )?;

        // launch reserve
        let reserve = Reserve::new(
            source_cat.coin.coin_id(),
            source_cat.child_lineage_proof(),
            source_cat.asset_id,
            SingletonStruct::new(registry.info.launcher_id)
                .tree_hash()
                .into(),
            0,
            0,
        );

        let new_source_cat =
            source_cat.wrapped_child(cat_minter_puzzle_hash, source_cat.coin.amount);

        let cat_minter_inner_puzzle = clvm_quote!(Conditions::new()
            .create_coin(reserve.inner_puzzle_hash, 0, None)
            .create_coin(
                new_source_cat.p2_puzzle_hash,
                new_source_cat.coin.amount,
                None
            ))
        .to_clvm(&mut ctx.allocator)?;
        let source_cat_inner_spend = cat_minter_p2.delegated_inner_spend(
            ctx,
            Spend {
                puzzle: cat_minter_inner_puzzle,
                solution: NodePtr::NIL,
            },
        )?;
        source_cat.spend(
            ctx,
            SingleCatSpend {
                prev_coin_id: source_cat.coin.coin_id(),
                next_coin_proof: CoinProof {
                    parent_coin_info: source_cat.coin.parent_coin_info,
                    inner_puzzle_hash: cat_minter_puzzle_hash,
                    amount: source_cat.coin.amount,
                },
                prev_subtotal: 0,
                extra_delta: 0,
                inner_spend: source_cat_inner_spend,
            },
        )?;

        sim.spend_coins(
            ctx.take(),
            &[
                launcher_sk.clone(),
                security_sk.clone(),
                cat_minter_sk.clone(),
            ],
        )?;
        source_cat = new_source_cat;

        // add the 1st mirror before reward epoch ('first epoch') begins
        let (validator_conditions, mut registry, mut reserve, mirror1_slot) = registry.add_mirror(
            ctx,
            reserve,
            mirror1_puzzle_hash,
            1,
            validator_singleton_inner_puzzle_hash,
        )?;

        (validator_coin, validator_singleton_proof) = spend_validator_singleton(
            ctx,
            validator_coin,
            validator_singleton_proof,
            validator_singleton_puzzle,
            validator_conditions,
        )?;
        sim.spend_coins(ctx.take(), &[])?;

        // commit incentives for first epoch
        let rewards_to_add = constants.epoch_seconds;
        let registry_info = registry.info;
        let (
            secure_conditions,
            new_registry,
            new_reserve,
            registry_solution,
            first_epoch_commitment_slot,
            mut incentive_slots,
        ) = registry.commit_incentives(
            ctx,
            reserve.coin.parent_coin_info,
            first_epoch_slot,
            first_epoch_start,
            cat_minter_puzzle_hash,
            rewards_to_add,
        )?;

        // spend reserve and source cat together so deltas add up
        let reserve_delegated_puzzle = reserve.delegated_puzzle_for_finalizer_controller(
            ctx,
            registry_info.state,
            reserve.coin.amount + rewards_to_add,
            registry_solution,
        )?;

        let reserve_cat_spend = CatSpend::new(
            reserve.to_cat(),
            reserve.inner_spend(
                ctx,
                registry_info.inner_puzzle_hash().into(),
                reserve_delegated_puzzle,
                NodePtr::NIL,
            )?,
        );
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter_puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        let cat_spends = [reserve_cat_spend, source_cat_spend];
        Cat::spend_all(ctx, &cat_spends)?;

        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        reserve = new_reserve;
        registry = new_registry;
        source_cat = source_cat.wrapped_child(
            cat_minter_puzzle_hash,
            source_cat.coin.amount - rewards_to_add,
        );
        assert!(sim
            .coin_state(first_epoch_commitment_slot.coin.coin_id())
            .is_some());
        for incentive_slot in incentive_slots.iter() {
            assert!(sim.coin_state(incentive_slot.coin.coin_id()).is_some());
        }

        // commit incentives for fifth epoch
        let fifth_epoch_start = first_epoch_start + constants.epoch_seconds * 4;
        let rewards_to_add = constants.epoch_seconds * 10;
        let registry_info = registry.info;
        let (
            secure_conditions,
            new_registry,
            new_reserve,
            registry_solution,
            fifth_epoch_commitment_slot,
            new_incentive_slots,
        ) = registry.commit_incentives(
            ctx,
            reserve.coin.parent_coin_info,
            *incentive_slots.last().unwrap(),
            fifth_epoch_start,
            cat_minter_puzzle_hash,
            rewards_to_add,
        )?;

        let new_value_keys = new_incentive_slots
            .iter()
            .map(|s| s.info.value.unwrap().epoch_start)
            .collect::<Vec<_>>();
        incentive_slots.retain(|s| !new_value_keys.contains(&s.info.value.unwrap().epoch_start));
        incentive_slots.extend(new_incentive_slots);

        // spend reserve and source cat together so deltas add up
        let reserve_delegated_puzzle = reserve.delegated_puzzle_for_finalizer_controller(
            ctx,
            registry_info.state,
            reserve.coin.amount + rewards_to_add,
            registry_solution,
        )?;

        let reserve_cat_spend = CatSpend::new(
            reserve.to_cat(),
            reserve.inner_spend(
                ctx,
                registry_info.inner_puzzle_hash().into(),
                reserve_delegated_puzzle,
                NodePtr::NIL,
            )?,
        );
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter_puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        let cat_spends = [reserve_cat_spend, source_cat_spend];
        Cat::spend_all(ctx, &cat_spends)?;

        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        reserve = new_reserve;
        registry = new_registry;
        source_cat = source_cat.wrapped_child(
            cat_minter_puzzle_hash,
            source_cat.coin.amount - rewards_to_add,
        );
        assert!(sim
            .coin_state(fifth_epoch_commitment_slot.coin.coin_id())
            .is_some());
        for incentive_slot in incentive_slots.iter() {
            assert!(sim.coin_state(incentive_slot.coin.coin_id()).is_some());
        }

        // 2nd commit incentives for fifth epoch
        let rewards_to_add = constants.epoch_seconds * 2;
        let registry_info = registry.info;
        let (
            secure_conditions,
            new_registry,
            new_reserve,
            registry_solution,
            fifth_epoch_commitment_slot2,
            new_incentive_slots,
        ) = registry.commit_incentives(
            ctx,
            reserve.coin.parent_coin_info,
            *incentive_slots
                .iter()
                .find(|s| s.info.value.unwrap().epoch_start == fifth_epoch_start)
                .unwrap(),
            fifth_epoch_start,
            cat_minter_puzzle_hash,
            rewards_to_add,
        )?;

        let new_value_keys = new_incentive_slots
            .iter()
            .map(|s| s.info.value.unwrap().epoch_start)
            .collect::<Vec<_>>();
        incentive_slots.retain(|s| !new_value_keys.contains(&s.info.value.unwrap().epoch_start));
        incentive_slots.extend(new_incentive_slots);

        // spend reserve and source cat together so deltas add up
        let reserve_delegated_puzzle = reserve.delegated_puzzle_for_finalizer_controller(
            ctx,
            registry_info.state,
            reserve.coin.amount + rewards_to_add,
            registry_solution,
        )?;

        let reserve_cat_spend = CatSpend::new(
            reserve.to_cat(),
            reserve.inner_spend(
                ctx,
                registry_info.inner_puzzle_hash().into(),
                reserve_delegated_puzzle,
                NodePtr::NIL,
            )?,
        );
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter_puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        let cat_spends = [reserve_cat_spend, source_cat_spend];
        Cat::spend_all(ctx, &cat_spends)?;

        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        reserve = new_reserve;
        registry = new_registry;
        source_cat = source_cat.wrapped_child(
            cat_minter_puzzle_hash,
            source_cat.coin.amount - rewards_to_add,
        );
        assert!(sim
            .coin_state(fifth_epoch_commitment_slot2.coin.coin_id())
            .is_some());
        for incentive_slot in incentive_slots.iter() {
            assert!(sim.coin_state(incentive_slot.coin.coin_id()).is_some());
        }
        assert!(sim
            .coin_state(reserve.coin.coin_id())
            .unwrap()
            .spent_height
            .is_none());

        // withdraw the 1st incentives for epoch 5
        let reserve_cat = reserve.to_cat();
        let (
            withdraw_incentives_conditions,
            new_registry,
            new_reserve,
            withdrawn_amount,
            new_reward_slot,
        ) = registry.withdraw_incentives(
            ctx,
            reserve,
            fifth_epoch_commitment_slot,
            *incentive_slots
                .iter()
                .find(|s| s.info.value.unwrap().epoch_start == fifth_epoch_start)
                .unwrap(),
        )?;

        let payout_coin_id = reserve_cat
            .wrapped_child(
                cat_minter_puzzle_hash, // fifth_epoch_commitment_slot.info.value.unwrap().clawback_ph,
                withdrawn_amount,
            )
            .coin
            .coin_id();

        let claimer_coin = sim.new_coin(cat_minter_puzzle_hash, 0);
        cat_minter_p2.spend(ctx, claimer_coin, withdraw_incentives_conditions)?;

        sim.set_next_timestamp(first_epoch_start)?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        assert!(sim.coin_state(payout_coin_id).is_some());
        reserve = new_reserve;
        registry = new_registry;
        assert!(sim
            .coin_state(fifth_epoch_commitment_slot.coin.coin_id())
            .unwrap()
            .spent_height
            .is_some());
        assert!(sim
            .coin_state(new_reward_slot.coin.coin_id())
            .unwrap()
            .spent_height
            .is_none());
        incentive_slots.retain(|s| {
            s.info.value.unwrap().epoch_start != new_reward_slot.info.value.unwrap().epoch_start
        });
        incentive_slots.push(new_reward_slot);

        // start first epoch
        let reserve_cat = reserve.to_cat();
        let first_epoch_incentives_slot = *incentive_slots
            .iter()
            .find(|s| s.info.value.unwrap().epoch_start == first_epoch_start)
            .unwrap();
        let (new_epoch_conditions, new_registry, new_reserve, validator_fee, new_reward_slot) =
            registry.new_epoch(
                ctx,
                reserve,
                first_epoch_incentives_slot,
                first_epoch_incentives_slot.info.value.unwrap().rewards,
            )?;
        let payout_coin_id = reserve_cat
            .wrapped_child(constants.validator_payout_puzzle_hash, validator_fee)
            .coin
            .coin_id();

        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        assert!(sim.coin_state(payout_coin_id).is_some());
        reserve = new_reserve;
        registry = new_registry;
        assert!(sim
            .coin_state(first_epoch_incentives_slot.coin.coin_id())
            .unwrap()
            .spent_height
            .is_some());
        assert!(sim
            .coin_state(new_reward_slot.coin.coin_id())
            .unwrap()
            .spent_height
            .is_none());
        incentive_slots.retain(|s| {
            s.info.value.unwrap().epoch_start != new_reward_slot.info.value.unwrap().epoch_start
        });
        incentive_slots.push(new_reward_slot);

        Ok(())
    }
}

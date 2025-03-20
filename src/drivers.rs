use bip39::Mnemonic;
use chia::{
    bls::{sign, PublicKey, SecretKey, Signature},
    clvm_utils::ToTreeHash,
    consensus::consensus_constants::ConsensusConstants,
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{
        cat::{CatArgs, CatSolution, CAT_PUZZLE_HASH},
        offer::{
            Memos, NotarizedPayment, Payment, SettlementPaymentsSolution,
            SETTLEMENT_PAYMENTS_PUZZLE_HASH,
        },
        singleton::{SingletonArgs, SingletonSolution, SingletonStruct},
        standard::{StandardArgs, StandardSolution},
        EveProof, LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    announcement_id, AggSig, AggSigConstants, AggSigKind, Cat, CatSpend, Condition, Conditions,
    CurriedPuzzle, DriverError, Launcher, Layer, Offer, RequiredBlsSignature, Spend, SpendContext,
    StandardLayer,
};
use clvm_traits::{clvm_quote, clvm_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState,
    CatalogSlotValue, DefaultCatMakerArgs, DigRewardDistributor, DigRewardDistributorConstants,
    DigRewardDistributorInfo, DigRewardDistributorState, DigRewardSlotValue, DigSlotNonce,
    P2DelegatedBySingletonLayerArgs, Reserve, RoundRewardInfo, RoundTimeInfo, Slot, SlotInfo,
    SlotProof, XchandlesConstants, XchandlesRegistry, XchandlesRegistryInfo,
    XchandlesRegistryState, XchandlesSlotValue, SLOT32_MAX_VALUE, SLOT32_MIN_VALUE,
};

pub struct SecuredOneSidedOffer {
    pub coin_spends: Vec<CoinSpend>,
    pub aggregated_signature: Signature,
    pub security_coin: Coin,
    pub security_base_conditions: Conditions<NodePtr>,
    pub created_cat: Option<Cat>,
}

fn custom_err<T>(e: T) -> DriverError
where
    T: ToString,
{
    DriverError::Custom(e.to_string())
}

pub fn new_sk() -> Result<SecretKey, DriverError> {
    // we need the security coin puzzle hash to spend the offer coin after finding it
    let mut entropy = [0u8; 32];
    getrandom::getrandom(&mut entropy).map_err(custom_err)?;
    let mnemonic = Mnemonic::from_entropy(&entropy).map_err(custom_err)?;
    let seed = mnemonic.to_seed("");
    let sk = SecretKey::from_seed(&seed);
    Ok(sk)
}

pub fn parse_one_sided_offer(
    ctx: &mut SpendContext,
    offer: Offer,
    security_public_key: PublicKey,
    cat_destination_puzzle_hash: Option<Bytes32>,
) -> Result<SecuredOneSidedOffer, DriverError> {
    let offer = offer.parse(&mut ctx.allocator).map_err(custom_err)?;

    if !offer.requested_payments.is_empty() {
        return Err(DriverError::Custom(
            "Launch offer should not have any requested payments".to_string(),
        ));
    }

    let security_coin_puzzle_hash: Bytes32 =
        StandardArgs::curry_tree_hash(security_public_key).into();

    // returned spends will also spend the offer coin (creating the security coin)
    let mut coin_spends = Vec::with_capacity(offer.coin_spends.len() + 1);
    let mut security_coin_parent_id: Option<Bytes32> = None;
    let mut security_coin_amount = 0;

    let mut base_conditions = Conditions::new();
    let mut created_cat: Option<Cat> = None;

    for coin_spend in offer.coin_spends {
        let puzzle_ptr = coin_spend.puzzle_reveal.to_clvm(&mut ctx.allocator)?;
        let solution_ptr = coin_spend.solution.to_clvm(&mut ctx.allocator)?;

        let curried_puzzle = CurriedPuzzle::parse(&ctx.allocator, puzzle_ptr);
        if let Some(curried_puzzle) = curried_puzzle {
            if curried_puzzle.mod_hash == CAT_PUZZLE_HASH {
                let cat_args = CatArgs::<NodePtr>::from_clvm(&ctx.allocator, curried_puzzle.args)?;
                let cat_solution = CatSolution::<NodePtr>::from_clvm(&ctx.allocator, solution_ptr)?;

                let inner_output =
                    ctx.run(cat_args.inner_puzzle, cat_solution.inner_puzzle_solution)?;
                let inner_output =
                    Vec::<Condition<NodePtr>>::from_clvm(&ctx.allocator, inner_output)?;

                if let Some(cc) = inner_output
                    .into_iter()
                    .filter_map(|cond| {
                        let Condition::CreateCoin(cc) = cond else {
                            return None;
                        };

                        Some(cc)
                    })
                    .find(|cc| cc.puzzle_hash == SETTLEMENT_PAYMENTS_PUZZLE_HASH.into())
                {
                    let Some(cat_destination_puzzle_hash) = cat_destination_puzzle_hash else {
                        return Err(DriverError::Custom(
                            "CAT destination puzzle hash not provided but offered CAT found"
                                .to_string(),
                        ));
                    };

                    // we found a CAT creating an offered CAT - spend it to create
                    // the offer CAT, which then creates a 0-amount coin with
                    // puzzle hash = cat_destination_puzzle_hash; also return the cat
                    // by returning it to its original puzzle hash

                    // and assert the announcement to make sure it's not going
                    // somewhere else
                    let offer_cat_full_puzzle_hash = CatArgs::curry_tree_hash(
                        cat_args.asset_id,
                        SETTLEMENT_PAYMENTS_PUZZLE_HASH,
                    );
                    let offer_coin_cat = Coin::new(
                        coin_spend.coin.coin_id(),
                        offer_cat_full_puzzle_hash.into(),
                        cc.amount,
                    );
                    let refund_puzzle_hash = ctx.tree_hash(cat_args.inner_puzzle).into(); // funds will be returned to refund_puzzle_hash (inner puzzle hash)
                    let offer_cat = Cat::new(
                        offer_coin_cat,
                        Some(LineageProof {
                            parent_parent_coin_info: coin_spend.coin.parent_coin_info,
                            parent_inner_puzzle_hash: refund_puzzle_hash,
                            parent_amount: coin_spend.coin.amount,
                        }),
                        cat_args.asset_id,
                        SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                    );

                    // offers don't allow amount 0 coins, so we create an 1-amount coin,
                    //   which creates a 0-amount coin and spend all the CATs together
                    let interim_inner_puzzle = clvm_quote!(Conditions::new().create_coin(
                        cat_destination_puzzle_hash,
                        0,
                        Some(chia_wallet_sdk::Memos::new(
                            vec![cat_destination_puzzle_hash].to_clvm(&mut ctx.allocator)?
                        ))
                    ))
                    .to_clvm(&mut ctx.allocator)?;
                    let interim_inner_ph: Bytes32 = ctx.tree_hash(interim_inner_puzzle).into();

                    let notarized_payment = NotarizedPayment {
                        nonce: offer_coin_cat.coin_id(),
                        payments: vec![
                            Payment {
                                puzzle_hash: refund_puzzle_hash,
                                amount: cc.amount,
                                memos: Some(Memos(vec![refund_puzzle_hash.into()])),
                            },
                            Payment {
                                puzzle_hash: interim_inner_ph,
                                amount: 1,
                                memos: None,
                            },
                        ],
                    };
                    let offer_cat_inner_solution = SettlementPaymentsSolution {
                        notarized_payments: vec![notarized_payment.clone()],
                    }
                    .to_clvm(&mut ctx.allocator)?;

                    let offer_cat_spend = CatSpend::new(
                        offer_cat,
                        Spend::new(ctx.settlement_payments_puzzle()?, offer_cat_inner_solution),
                    );

                    let interim_cat = offer_cat.wrapped_child(interim_inner_ph, 1);
                    let interim_cat_spend =
                        CatSpend::new(interim_cat, Spend::new(interim_inner_puzzle, NodePtr::NIL));

                    let orig_spends = ctx.take();

                    Cat::spend_all(ctx, &[offer_cat_spend, interim_cat_spend])?;
                    coin_spends.extend(ctx.take());

                    for og_spend in orig_spends {
                        ctx.insert(og_spend);
                    }

                    created_cat = Some(interim_cat.wrapped_child(cat_destination_puzzle_hash, 0));
                    let announcement_msg: Bytes32 = notarized_payment.tree_hash().into();
                    base_conditions = base_conditions.assert_puzzle_announcement(announcement_id(
                        offer_cat.coin.puzzle_hash,
                        announcement_msg,
                    ));
                }
            }
        }

        if security_coin_parent_id.is_none() {
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

    if cat_destination_puzzle_hash.is_some() && created_cat.is_none() {
        return Err(DriverError::Custom(
            "Could not find CAT offered in one-sided offer".to_string(),
        ));
    }

    Ok(SecuredOneSidedOffer {
        coin_spends,
        aggregated_signature: offer.aggregated_signature,
        security_coin,
        security_base_conditions: base_conditions,
        created_cat,
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
fn spend_eve_coin_and_create_registry<S, M>(
    ctx: &mut SpendContext,
    launcher: Launcher,
    target_inner_puzzle_hash: Bytes32,
    left_slot_value: S,
    right_slot_value: S,
    memos_after_hint: M,
) -> Result<(Conditions, Coin, Proof, [Slot<S>; 2]), DriverError>
where
    S: Copy + ToTreeHash,
    M: ToClvm<Allocator>,
{
    let launcher_coin = launcher.coin();
    let launcher_id = launcher_coin.coin_id();

    let left_slot_info = SlotInfo::from_value(launcher_id, 0, left_slot_value);
    let left_slot_puzzle_hash = Slot::<S>::puzzle_hash(&left_slot_info);

    let right_slot_info = SlotInfo::from_value(launcher_id, 0, right_slot_value);
    let right_slot_puzzle_hash = Slot::<S>::puzzle_hash(&right_slot_info);

    let slot_hint: Bytes32 = Slot::<()>::first_curry_hash(launcher_id, 0).into();
    let slot_memos = ctx.hint(slot_hint)?;
    let launcher_id_ptr = ctx.alloc(&launcher_id)?;
    let memos_ptr = ctx.alloc(&memos_after_hint)?;
    let launcher_memos = ctx.memos(&clvm_tuple!(launcher_id_ptr, memos_ptr))?;
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
pub fn launch_catalog_registry<V>(
    ctx: &mut SpendContext,
    offer: Offer,
    initial_registration_price: u64,
    // (registry launcher id, security coin, additional_args) -> (additional conditions, registry constants, initial_registration_asset_id)
    get_additional_info: fn(
        ctx: &mut SpendContext,
        Bytes32,
        Coin,
        V,
    ) -> Result<
        (Conditions<NodePtr>, CatalogRegistryConstants, Bytes32),
        DriverError,
    >,
    consensus_constants: &ConsensusConstants,
    additional_args: V,
) -> Result<
    (
        Signature,
        SecretKey,
        CatalogRegistry,
        [Slot<CatalogSlotValue>; 2],
        Coin, // security coin
    ),
    DriverError,
> {
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(ctx, offer, security_coin_sk.public_key(), None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = offer.security_base_conditions;

    // Create CATalog registry launcher
    let registry_launcher = Launcher::new(security_coin_id, 1);
    let registry_launcher_coin = registry_launcher.coin();
    let registry_launcher_id = registry_launcher_coin.coin_id();

    let (additional_security_coin_conditions, catalog_constants, initial_registration_asset_id) =
        get_additional_info(
            ctx,
            registry_launcher_id,
            offer.security_coin,
            additional_args,
        )?;

    let initial_state = CatalogRegistryState {
        registration_price: initial_registration_price,
        cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(
            initial_registration_asset_id.tree_hash().into(),
        )
        .into(),
    };
    let catalog_registry_info =
        CatalogRegistryInfo::new(registry_launcher_id, initial_state, catalog_constants);
    let catalog_inner_puzzle_hash = catalog_registry_info.clone().inner_puzzle_hash();

    let (new_security_coin_conditions, new_catalog_registry_coin, catalog_proof, slots) =
        spend_eve_coin_and_create_registry(
            ctx,
            registry_launcher,
            catalog_inner_puzzle_hash.into(),
            CatalogSlotValue::left_end(SLOT32_MAX_VALUE.into()),
            CatalogSlotValue::right_end(SLOT32_MIN_VALUE.into()),
            clvm_tuple!(
                initial_registration_asset_id,
                clvm_tuple!(initial_state, ())
            ),
        )?;

    let catalog_registry = CatalogRegistry::new(
        new_catalog_registry_coin,
        catalog_proof,
        catalog_registry_info,
    );

    // this creates the CATalog registry & secures the spend
    security_coin_conditions = security_coin_conditions
        .extend(new_security_coin_conditions)
        .extend(additional_security_coin_conditions);

    // Spend security coin
    let security_coin_sig = spend_security_coin(
        ctx,
        offer.security_coin,
        security_coin_conditions,
        &security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        offer.aggregated_signature + &security_coin_sig,
        security_coin_sk,
        catalog_registry,
        slots,
        offer.security_coin,
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
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(ctx, offer, security_coin_sk.public_key(), None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = offer.security_base_conditions;

    // Create registry coin launcher
    let registry_launcher = Launcher::new(security_coin_id, 1);
    let registry_launcher_coin = registry_launcher.coin();
    let registry_launcher_id = registry_launcher_coin.coin_id();

    // Spend intermediary coin and create registry
    let initial_state = XchandlesRegistryState::from(
        initial_registration_asset_id.tree_hash().into(),
        initial_base_registration_price,
    );
    let target_xchandles_info =
        XchandlesRegistryInfo::new(registry_launcher_id, initial_state, xchandles_constants);
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
            clvm_tuple!(
                initial_registration_asset_id,
                clvm_tuple!(initial_state, ())
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
        &security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        offer.aggregated_signature + &security_coin_sig,
        security_coin_sk,
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
    // use a 'trick' to find the launcher id so we can determine reserve id, then call the function again
    //  with the right CAT destination ph
    let security_coin_sk = new_sk()?;
    let mock_offer = parse_one_sided_offer(
        ctx,
        offer.clone(),
        security_coin_sk.public_key(),
        Some(Bytes32::default()),
    )?;
    let launcher = Launcher::new(mock_offer.security_coin.coin_id(), 1);
    let launcher_coin = launcher.coin();
    let launcher_id = launcher_coin.coin_id();

    let controller_singleton_struct_hash: Bytes32 =
        SingletonStruct::new(launcher_id).tree_hash().into();
    let reserve_inner_ph =
        P2DelegatedBySingletonLayerArgs::curry_tree_hash(controller_singleton_struct_hash, 0);

    let offer = parse_one_sided_offer(
        ctx,
        offer,
        security_coin_sk.public_key(),
        Some(reserve_inner_ph.into()),
    )?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let mut security_coin_conditions = offer.security_base_conditions;

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
        next_epoch_initialized: false,
        rewards: 0,
    };
    let slot_info = SlotInfo::<DigRewardSlotValue>::from_value(
        launcher_id,
        DigSlotNonce::REWARD.to_u64(),
        slot_value,
    );
    let slot_puzzle_hash = Slot::<DigRewardSlotValue>::puzzle_hash(&slot_info);

    let slot_hint: Bytes32 =
        Slot::<()>::first_curry_hash(launcher_id, DigSlotNonce::REWARD.to_u64()).into();
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

    let (launch_conditions, eve_coin) =
        launcher
            .with_singleton_amount(1)
            .spend(ctx, eve_singleton_inner_puzzle_hash.into(), ())?;
    security_coin_conditions = security_coin_conditions.extend(launch_conditions);

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

    // create reserve and registry
    let Some(reserve_cat) = offer.created_cat else {
        return Err(DriverError::Custom(
            "Offer does not contain reserve".to_string(),
        ));
    };
    let reserve = Reserve::new(
        reserve_cat.coin.parent_coin_info,
        reserve_cat.lineage_proof.unwrap(),
        reserve_cat.asset_id,
        controller_singleton_struct_hash,
        0,
        reserve_cat.coin.amount,
    );
    let registry = DigRewardDistributor::new(new_registry_coin, new_proof, target_info, reserve);

    // Spend security coin
    let security_coin_sig = spend_security_coin(
        ctx,
        offer.security_coin,
        security_coin_conditions,
        &security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        offer.aggregated_signature + &security_coin_sig,
        security_coin_sk,
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
        test_secret_keys, Cat, CatSpend, Nft, NftMint, Simulator, SingleCatSpend,
        SpendWithConditions, TESTNET11_CONSTANTS,
    };
    use clvm_traits::clvm_list;
    use clvmr::Allocator;
    use hex_literal::hex;

    use crate::{
        print_spend_bundle_to_file, CatNftMetadata, CatalogPrecommitValue, CatalogRefundAction,
        CatalogRegisterAction, CatalogSlotValue, DelegatedStateAction,
        DelegatedStateActionSolution, DigAddIncentivesAction, DigAddMirrorAction,
        DigCommitIncentivesAction, DigInitiatePayoutAction, DigNewEpochAction,
        DigRemoveMirrorAction, DigRewardDistributorConstants, DigSyncAction,
        DigWithdrawIncentivesAction, PrecommitCoin, Slot, SpendContextExt, XchandlesExpireAction,
        XchandlesExponentialPremiumRenewPuzzleArgs, XchandlesExponentialPremiumRenewPuzzleSolution,
        XchandlesExtendAction, XchandlesFactorPricingPuzzleArgs, XchandlesFactorPricingSolution,
        XchandlesOracleAction, XchandlesPrecommitValue, XchandlesRefundAction,
        XchandlesRegisterAction, XchandlesUpdateAction, ANY_METADATA_UPDATER_HASH,
    };

    use super::*;

    fn cat_nft_metadata_for_testing() -> CatNftMetadata {
        CatNftMetadata {
            ticker: "TDBX".to_string(),
            name: "Testnet dexie bucks".to_string(),
            description: "    Testnet version of dexie bucks".to_string(),
            precision: 4,
            image_uris: vec!["https://icons-testnet.dexie.space/d82dd03f8a9ad2f84353cd953c4de6b21dbaaf7de3ba3f4ddd9abe31ecba80ad.webp".to_string()],
            image_hash: Bytes32::from(
                hex!("c84607c0e4cb4a878cc34ba913c90504ed0aac0f4484c2078529b9e42387da99")
            ),
            metadata_uris: vec!["https://icons-testnet.dexie.space/test.json".to_string()],
            metadata_hash: Some(Bytes32::from([2; 32])),
            license_uris: vec!["https://icons-testnet.dexie.space/license.pdf".to_string()],
            license_hash: Some(Bytes32::from([3; 32])),
        }
    }

    // ensures conditions are met
    fn ensure_conditions_met(
        ctx: &mut SpendContext,
        sim: &mut Simulator,
        conditions: Conditions<NodePtr>,
        amount_to_mint: u64,
    ) -> Result<(), DriverError> {
        let checker_puzzle_ptr = clvm_quote!(conditions).to_clvm(&mut ctx.allocator)?;
        let checker_coin = sim.new_coin(ctx.tree_hash(checker_puzzle_ptr).into(), amount_to_mint);
        ctx.spend(checker_coin, Spend::new(checker_puzzle_ptr, NodePtr::NIL))?;

        Ok(())
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
    ) -> Result<(Coin, Proof, DelegatedStateActionSolution<NodePtr>), DriverError>
    where
        S: ToTreeHash + ToClvm<Allocator>,
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
                new_state: new_state.to_clvm(&mut ctx.allocator)?,
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

        let mut catalog = catalog;
        let secure_cond = catalog.new_action::<CatalogRefundAction>().spend(
            ctx,
            &mut catalog,
            tail_hash.into(),
            if let Some(found_slot) = slot {
                found_slot.info.value.unwrap().neighbors.tree_hash().into()
            } else {
                Bytes32::default()
            },
            precommit_coin,
            slot.cloned(),
        )?;

        let new_catalog = catalog.finish_spend(ctx)?;

        ensure_conditions_met(ctx, sim, secure_cond, 0)?;

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
        let (_, security_sk, mut catalog, slots, _security_coin) = launch_catalog_registry(
            ctx,
            offer,
            initial_registration_price,
            |_ctx, _launcher_id, _coin, (catalog_constants, initial_registration_asset_id)| {
                Ok((
                    Conditions::new(),
                    catalog_constants,
                    initial_registration_asset_id,
                ))
            },
            &TESTNET11_CONSTANTS,
            (
                catalog_constants.with_price_singleton(price_singleton_launcher_id),
                payment_cat.asset_id,
            ),
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

                let new_state = CatalogRegistryState {
                    cat_maker_puzzle_hash: DefaultCatMakerArgs::curry_tree_hash(
                        payment_cat.asset_id.tree_hash().into(),
                    )
                    .into(),
                    registration_price: new_price,
                };

                let (
                    new_price_singleton_coin,
                    new_price_singleton_proof,
                    delegated_state_action_solution,
                ) = spend_price_singleton(
                    ctx,
                    price_singleton_coin,
                    price_singleton_proof,
                    price_singleton_puzzle,
                    new_state,
                    catalog.coin.puzzle_hash,
                )?;

                price_singleton_coin = new_price_singleton_coin;
                price_singleton_proof = new_price_singleton_proof;

                let (_conds, action_spend) = catalog.new_action::<DelegatedStateAction>().spend(
                    ctx,
                    catalog.coin,
                    new_state,
                    delegated_state_action_solution.other_singleton_inner_puzzle_hash,
                )?;

                catalog.insert(action_spend);
                catalog = catalog.finish_spend(ctx)?;
                sim.spend_coins(ctx.take(), &[user_sk.clone()])?;
            };

            let (secure_cond, new_slots) = catalog.new_action::<CatalogRegisterAction>().spend(
                ctx,
                &mut catalog,
                tail_hash.into(),
                left_slot,
                right_slot,
                precommit_coin,
                Spend {
                    puzzle: eve_nft_inner_puzzle,
                    solution: NodePtr::NIL,
                },
            )?;

            catalog = catalog.finish_spend(ctx)?;

            ensure_conditions_met(ctx, &mut sim, secure_cond.clone(), 1)?;

            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != left_slot && *s != right_slot);
            slots.extend(new_slots);
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
            if let Some(existing_slot) = slot {
                existing_slot.info.value.unwrap().expiration + 28 * 24 * 60 * 60 + 1
            } else {
                0
            },
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

        let mut registry = registry;
        let (secure_cond, _new_slot_maybe) = registry.new_action::<XchandlesRefundAction>().spend(
            ctx,
            &mut registry,
            precommit_coin,
            pricing_puzzle,
            pricing_solution,
            slot,
        )?;
        let new_registry = registry.finish_spend(ctx)?;

        ensure_conditions_met(ctx, sim, secure_cond.clone(), 0)?;

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
                    current_expiration: 0,
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

                let (_conds, action_spend) = registry.new_action::<DelegatedStateAction>().spend(
                    ctx,
                    registry.coin,
                    delegated_state_action_solution.new_state,
                    delegated_state_action_solution.other_singleton_inner_puzzle_hash,
                )?;

                registry.insert(action_spend);
                registry = registry.finish_spend(ctx)?;
                sim.spend_coins(ctx.take(), &[user_sk.clone()])?;
            };

            let (secure_cond, new_slots) = registry.new_action::<XchandlesRegisterAction>().spend(
                ctx,
                &mut registry,
                left_slot,
                right_slot,
                precommit_coin,
                base_price,
            )?;

            ensure_conditions_met(ctx, &mut sim, secure_cond.clone(), 1)?;

            registry = registry.finish_spend(ctx)?;
            sim.pass_time(100); // registration start was at timestamp 100
            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != left_slot && *s != right_slot);

            let oracle_slot = new_slots[1];
            slots.extend(new_slots);

            // test on-chain oracle for current handle
            let (oracle_conds, new_slot) = registry.new_action::<XchandlesOracleAction>().spend(
                ctx,
                &mut registry,
                oracle_slot,
            )?;

            let user_coin = sim.new_coin(user_puzzle_hash, 0);
            StandardLayer::new(user_pk).spend(ctx, user_coin, oracle_conds)?;

            slots.retain(|s| *s != oracle_slot);
            slots.push(new_slot);

            // test on-chain extend mechanism for current handle
            let extension_years: u64 = i as u64 + 1;
            let extension_slot = new_slot;
            let pay_for_extension: u64 =
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &handle, extension_years);

            let (notarized_payment, extend_conds, new_slot) =
                registry.new_action::<XchandlesExtendAction>().spend(
                    ctx,
                    &mut registry,
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

            slots.retain(|s| *s != extension_slot);
            slots.push(new_slot);

            // test on-chain mechanism for handle updates
            let new_owner_launcher_id = Bytes32::new([4 + i as u8; 32]);
            let new_resolved_launcher_id = Bytes32::new([u8::MAX - i as u8 - 1; 32]);
            let update_slot = new_slot;

            let (update_conds, new_slot) = registry.new_action::<XchandlesUpdateAction>().spend(
                ctx,
                &mut registry,
                update_slot,
                new_owner_launcher_id,
                new_resolved_launcher_id,
                did.info.inner_puzzle_hash().into(),
            )?;

            let _new_did = did.update(ctx, &user_puzzle, update_conds)?;

            registry = registry.finish_spend(ctx)?;
            sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;

            slots.retain(|s| *s != update_slot);
            slots.push(new_slot);
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
                pricing_program_solution: XchandlesFactorPricingSolution {
                    current_expiration: expiration,
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

        let (expire_conds, _new_slot) = registry.new_action::<XchandlesExpireAction>().spend(
            ctx,
            &mut registry,
            *initial_slot,
            1,
            base_price,
            precommit_coin,
        )?;

        // assert expire conds
        ensure_conditions_met(ctx, &mut sim, expire_conds, 1)?;
        registry = registry.finish_spend(ctx)?;
        sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

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
                    current_expiration: 0,
                    handle: unregistered_handle.clone(),
                    num_years: 1,
                }
                .to_clvm(&mut ctx.allocator)?
            } else {
                XchandlesExponentialPremiumRenewPuzzleSolution {
                    buy_time: 28 * 24 * 60 * 60 + 1, // premium should be 0
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        current_expiration: 0,
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
                    current_expiration: existing_slot.info.value.unwrap().expiration,
                    handle: existing_handle.clone(),
                    num_years: 1,
                }
                .to_clvm(&mut ctx.allocator)?
            } else {
                XchandlesExponentialPremiumRenewPuzzleSolution {
                    buy_time: existing_slot.info.value.unwrap().expiration + 28 * 24 * 60 * 60 + 1, // premium should be 0
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        current_expiration: existing_slot.info.value.unwrap().expiration,
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
            ticker: "XXX".to_string(),
            name: "Test Name".to_string(),
            description: "Test desc".to_string(),
            precision: 4,
            image_uris: vec!["img URI".to_string()],
            image_hash: Bytes32::from([31; 32]),
            metadata_uris: vec!["meta URI".to_string()],
            metadata_hash: Some(Bytes32::from([8; 32])),
            license_uris: vec!["license URI".to_string()],
            license_hash: Some(Bytes32::from([9; 32])),
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
        let cat_amount = 10_000_000_000;
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
            _validator_singleton_inner_puzzle,
            validator_singleton_inner_puzzle_hash,
            validator_singleton_puzzle,
        ) = launch_test_singleton(ctx, &mut sim)?;

        // setup config
        let constants = DigRewardDistributorConstants {
            validator_launcher_id,
            validator_payout_puzzle_hash: Bytes32::new([1; 32]),
            epoch_seconds: 1000,
            max_seconds_offset: 300,
            payout_threshold: 42,
            validator_fee_bps: 420,     // 4.2% fee
            withdrawal_share_bps: 9000, // 90% of the amount deposited will be returned
            reserve_asset_id: source_cat.asset_id,
            reserve_inner_puzzle_hash: Bytes32::default(), // will be overwritten
            reserve_full_puzzle_hash: Bytes32::default(),  // will be overwritten
        };

        // Create source offer
        let [launcher_sk, mirror1_sk, mirror2_sk]: [SecretKey; 3] =
            test_secret_keys(3)?.try_into().unwrap();

        let launcher_pk = launcher_sk.public_key();
        let launcher_puzzle_hash = StandardArgs::curry_tree_hash(launcher_pk).into();

        let mirror1_pk = mirror1_sk.public_key();
        let mirror1_puzzle_hash: Bytes32 = StandardArgs::curry_tree_hash(mirror1_pk).into();

        let mirror2_pk = mirror2_sk.public_key();
        let mirror2_puzzle_hash: Bytes32 = StandardArgs::curry_tree_hash(mirror2_pk).into();

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

        let cat_minter_inner_puzzle = clvm_quote!(Conditions::new().create_coin(
            SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
            source_cat.coin.amount,
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
        let spends = ctx.take();
        let cat_offer_spend = spends
            .iter()
            .find(|s| s.coin.coin_id() == source_cat.coin.coin_id())
            .unwrap()
            .clone();
        for spend in spends {
            if spend.coin.coin_id() != source_cat.coin.coin_id() {
                ctx.insert(spend);
            }
        }

        let offer = Offer::new(SpendBundle {
            coin_spends: vec![
                CoinSpend::new(offer_src_coin, puzzle_reveal, solution),
                cat_offer_spend,
            ],
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

        // launch reserve
        let (_, security_sk, mut registry, first_epoch_slot) = launch_dig_reward_distributor(
            ctx,
            offer,
            first_epoch_start,
            constants,
            &TESTNET11_CONSTANTS,
        )?;
        let spends = ctx.take();
        print_spend_bundle_to_file(spends.clone(), Signature::default(), "sb.debug");
        sim.spend_coins(
            spends,
            &[
                launcher_sk.clone(),
                security_sk.clone(),
                cat_minter_sk.clone(),
            ],
        )?;
        source_cat = source_cat
            .wrapped_child(
                SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                source_cat.coin.amount,
            )
            .wrapped_child(cat_minter_puzzle_hash, source_cat.coin.amount);
        assert!(sim.coin_state(source_cat.coin.coin_id()).is_some());

        // add the 1st mirror before reward epoch ('first epoch') begins
        let (validator_conditions, mirror1_slot) =
            registry.new_action::<DigAddMirrorAction>().spend(
                ctx,
                &mut registry,
                mirror1_puzzle_hash,
                1,
                validator_singleton_inner_puzzle_hash,
            )?;
        registry = registry.finish_spend(ctx, vec![])?;

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
        let (secure_conditions, first_epoch_commitment_slot, mut incentive_slots) =
            registry.new_action::<DigCommitIncentivesAction>().spend(
                ctx,
                &mut registry,
                first_epoch_slot,
                first_epoch_start,
                cat_minter_puzzle_hash,
                rewards_to_add,
            )?;

        // spend reserve and source cat together so deltas add up
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

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
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
        let (secure_conditions, fifth_epoch_commitment_slot, new_incentive_slots) =
            registry.new_action::<DigCommitIncentivesAction>().spend(
                ctx,
                &mut registry,
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

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
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
        let (secure_conditions, fifth_epoch_commitment_slot2, new_incentive_slots) =
            registry.new_action::<DigCommitIncentivesAction>().spend(
                ctx,
                &mut registry,
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

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;

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
            .coin_state(registry.reserve.coin.coin_id())
            .unwrap()
            .spent_height
            .is_none());

        // withdraw the 1st incentives for epoch 5
        let (withdraw_incentives_conditions, new_reward_slot, withdrawn_amount) =
            registry.new_action::<DigWithdrawIncentivesAction>().spend(
                ctx,
                &mut registry,
                fifth_epoch_commitment_slot,
                *incentive_slots
                    .iter()
                    .find(|s| s.info.value.unwrap().epoch_start == fifth_epoch_start)
                    .unwrap(),
            )?;

        let payout_coin_id = registry
            .reserve
            .to_cat()
            .wrapped_child(
                cat_minter_puzzle_hash, // fifth_epoch_commitment_slot.info.value.unwrap().clawback_ph,
                withdrawn_amount,
            )
            .coin
            .coin_id();

        let claimer_coin = sim.new_coin(cat_minter_puzzle_hash, 0);
        cat_minter_p2.spend(ctx, claimer_coin, withdraw_incentives_conditions)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.set_next_timestamp(first_epoch_start)?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        assert!(sim.coin_state(payout_coin_id).is_some());
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
        let reserve_cat = registry.reserve.to_cat();
        let first_epoch_incentives_slot = *incentive_slots
            .iter()
            .find(|s| s.info.value.unwrap().epoch_start == first_epoch_start)
            .unwrap();
        let (new_epoch_conditions, new_reward_slot, validator_fee) =
            registry.new_action::<DigNewEpochAction>().spend(
                ctx,
                &mut registry,
                first_epoch_incentives_slot,
                first_epoch_incentives_slot.info.value.unwrap().rewards,
            )?;
        let payout_coin_id = reserve_cat
            .wrapped_child(constants.validator_payout_puzzle_hash, validator_fee)
            .coin
            .coin_id();

        ensure_conditions_met(ctx, &mut sim, new_epoch_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.pass_time(100);
        sim.spend_coins(ctx.take(), &[])?;
        assert!(sim.coin_state(payout_coin_id).is_some());
        assert_eq!(registry.info.state.active_shares, 1);
        assert_eq!(registry.info.state.total_reserves, 4000 - validator_fee);
        assert_eq!(registry.info.state.round_reward_info.cumulative_payout, 0);
        assert_eq!(
            registry.info.state.round_reward_info.remaining_rewards,
            first_epoch_incentives_slot.info.value.unwrap().rewards - validator_fee
        );
        assert_eq!(
            registry.info.state.round_time_info.last_update,
            first_epoch_start
        );
        assert_eq!(
            registry.info.state.round_time_info.epoch_end,
            first_epoch_start + constants.epoch_seconds
        );
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

        // sync to 10%
        let initial_reward_info = registry.info.state.round_reward_info;
        let sync_conditions = registry.new_action::<DigSyncAction>().spend(
            ctx,
            &mut registry,
            first_epoch_start + 100,
        )?;
        ensure_conditions_met(ctx, &mut sim, sync_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.pass_time(400);
        sim.spend_coins(ctx.take(), &[])?;
        assert!(registry.info.state.round_time_info.last_update == first_epoch_start + 100);

        let cumulative_payout_delta = initial_reward_info.remaining_rewards / 10;
        assert!(
            registry.info.state.round_reward_info.remaining_rewards
                == initial_reward_info.remaining_rewards - cumulative_payout_delta
        );
        assert!(
            registry.info.state.round_reward_info.cumulative_payout
                == initial_reward_info.cumulative_payout + cumulative_payout_delta
        );

        // sync to 50% (so + 40%)
        let initial_reward_info = registry.info.state.round_reward_info;
        let sync_conditions = registry.new_action::<DigSyncAction>().spend(
            ctx,
            &mut registry,
            first_epoch_start + 500,
        )?;
        ensure_conditions_met(ctx, &mut sim, sync_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.spend_coins(ctx.take(), &[])?;
        assert!(registry.info.state.round_time_info.last_update == first_epoch_start + 500);

        let cumulative_payout_delta = initial_reward_info.remaining_rewards * 400 / 900;
        assert!(
            registry.info.state.round_reward_info.remaining_rewards
                == initial_reward_info.remaining_rewards - cumulative_payout_delta
        );
        assert!(
            registry.info.state.round_reward_info.cumulative_payout
                == initial_reward_info.cumulative_payout + cumulative_payout_delta
        );

        // add incentives
        let initial_reward_info = registry.info.state.round_reward_info;
        let incentives_amount = initial_reward_info.remaining_rewards;
        let registry_info = registry.info;

        let add_incentives_conditions = registry.new_action::<DigAddIncentivesAction>().spend(
            ctx,
            &mut registry,
            incentives_amount,
        )?;

        // spend reserve and source cat together so deltas add up
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                add_incentives_conditions.create_coin(
                    cat_minter_puzzle_hash,
                    source_cat.coin.amount - incentives_amount,
                    None,
                ),
            )?,
        );

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        assert_eq!(
            registry.info.state.round_time_info.last_update,
            first_epoch_start + 500
        );
        assert_eq!(
            registry.info.state.round_reward_info.cumulative_payout,
            registry_info.state.round_reward_info.cumulative_payout
        );
        assert_eq!(
            registry.info.state.round_reward_info.remaining_rewards,
            registry_info.state.round_reward_info.remaining_rewards
                + (incentives_amount - incentives_amount * constants.validator_fee_bps / 10000)
        );
        source_cat = source_cat.wrapped_child(
            cat_minter_puzzle_hash,
            source_cat.coin.amount - incentives_amount,
        );

        // add mirror2
        let (validator_conditions, mirror2_slot) =
            registry.new_action::<DigAddMirrorAction>().spend(
                ctx,
                &mut registry,
                mirror2_puzzle_hash,
                2,
                validator_singleton_inner_puzzle_hash,
            )?;
        (validator_coin, validator_singleton_proof) = spend_validator_singleton(
            ctx,
            validator_coin,
            validator_singleton_proof,
            validator_singleton_puzzle,
            validator_conditions,
        )?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.pass_time(250);
        sim.spend_coins(ctx.take(), &[])?;
        assert_eq!(registry.info.state.active_shares, 3);

        // sync to 75% (so + 25%)
        let initial_reward_info = registry.info.state.round_reward_info;
        let sync_conditions = registry.new_action::<DigSyncAction>().spend(
            ctx,
            &mut registry,
            first_epoch_start + 750,
        )?;
        ensure_conditions_met(ctx, &mut sim, sync_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.spend_coins(ctx.take(), &[])?;
        assert!(registry.info.state.round_time_info.last_update == first_epoch_start + 750);

        let cumulative_payout_delta = initial_reward_info.remaining_rewards * 250 / (3 * 500);
        assert!(
            registry.info.state.round_reward_info.remaining_rewards
                == initial_reward_info.remaining_rewards - cumulative_payout_delta * 3
        );
        assert!(
            registry.info.state.round_reward_info.cumulative_payout
                == initial_reward_info.cumulative_payout + cumulative_payout_delta
        );

        // remove mirror2
        let reserve_cat = registry.reserve.to_cat();
        let (remove_mirror_validator_conditions, mirror2_payout_amount) =
            registry.new_action::<DigRemoveMirrorAction>().spend(
                ctx,
                &mut registry,
                mirror2_slot,
                validator_singleton_inner_puzzle_hash,
            )?;

        let (_validator_coin, _validator_singleton_proof) = spend_validator_singleton(
            ctx,
            validator_coin,
            validator_singleton_proof,
            validator_singleton_puzzle,
            remove_mirror_validator_conditions,
        )?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.spend_coins(ctx.take(), &[])?;
        let payout_coin_id = reserve_cat
            .wrapped_child(mirror2_puzzle_hash, mirror2_payout_amount)
            .coin
            .coin_id();

        assert!(registry.info.state.active_shares == 1);
        assert!(sim.coin_state(payout_coin_id).is_some());
        assert!(sim
            .coin_state(mirror2_slot.coin.coin_id())
            .unwrap()
            .spent_height
            .is_some());

        for epoch in 1..7 {
            let update_time = registry.info.state.round_time_info.epoch_end;
            let sync_conditions =
                registry
                    .new_action::<DigSyncAction>()
                    .spend(ctx, &mut registry, update_time)?;

            let reward_slot = *incentive_slots
                .iter()
                .find(|s| {
                    s.info.value.unwrap().epoch_start
                        == first_epoch_start
                            + if epoch <= 4 { epoch } else { 4 } * constants.epoch_seconds
                })
                .unwrap();
            let (new_epoch_conditions, new_reward_slot, _validator_fee) =
                registry.new_action::<DigNewEpochAction>().spend(
                    ctx,
                    &mut registry,
                    reward_slot,
                    if epoch <= 4 {
                        reward_slot.info.value.unwrap().rewards
                    } else {
                        0
                    },
                )?;
            incentive_slots.retain(|s| {
                s.info.value.unwrap().epoch_start != new_reward_slot.info.value.unwrap().epoch_start
            });
            incentive_slots.push(new_reward_slot);

            ensure_conditions_met(
                ctx,
                &mut sim,
                sync_conditions.extend(new_epoch_conditions),
                0,
            )?;

            registry = registry.finish_spend(ctx, vec![])?;
            sim.set_next_timestamp(update_time)?;
            sim.spend_coins(ctx.take(), &[])?;
        }

        // commit incentives for 10th epoch
        let tenth_epoch_start = first_epoch_start + constants.epoch_seconds * 9;
        let rewards_to_add = constants.epoch_seconds * 10;
        let (secure_conditions, tenth_epoch_commitment_slot, new_incentive_slots) =
            registry.new_action::<DigCommitIncentivesAction>().spend(
                ctx,
                &mut registry,
                *incentive_slots.last().unwrap(),
                tenth_epoch_start,
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

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        sim.spend_coins(ctx.take(), &[cat_minter_sk.clone()])?;
        let _source_cat = source_cat.wrapped_child(
            cat_minter_puzzle_hash,
            source_cat.coin.amount - rewards_to_add,
        );
        assert!(sim
            .coin_state(tenth_epoch_commitment_slot.coin.coin_id())
            .is_some());
        for incentive_slot in incentive_slots.iter() {
            assert!(sim.coin_state(incentive_slot.coin.coin_id()).is_some());
        }

        for epoch in 7..10 {
            let update_time = registry.info.state.round_time_info.epoch_end;
            let sync_conditions =
                registry
                    .new_action::<DigSyncAction>()
                    .spend(ctx, &mut registry, update_time)?;

            let reward_slot = *incentive_slots
                .iter()
                .find(|s| {
                    s.info.value.unwrap().epoch_start
                        == first_epoch_start + epoch * constants.epoch_seconds
                })
                .unwrap();
            let (new_epoch_conditions, new_reward_slot, _validator_fee) =
                registry.new_action::<DigNewEpochAction>().spend(
                    ctx,
                    &mut registry,
                    reward_slot,
                    reward_slot.info.value.unwrap().rewards,
                )?;
            incentive_slots.retain(|s| {
                s.info.value.unwrap().epoch_start != new_reward_slot.info.value.unwrap().epoch_start
            });
            incentive_slots.push(new_reward_slot);

            ensure_conditions_met(
                ctx,
                &mut sim,
                sync_conditions.extend(new_epoch_conditions),
                0,
            )?;

            registry = registry.finish_spend(ctx, vec![])?;
            sim.set_next_timestamp(update_time)?;
            sim.spend_coins(ctx.take(), &[])?;
        }

        let update_time = registry.info.state.round_time_info.epoch_end - 100;
        let sync_conditions =
            registry
                .new_action::<DigSyncAction>()
                .spend(ctx, &mut registry, update_time)?;

        // payout mirror
        let reserve_cat = registry.reserve.to_cat();
        let (payout_conditions, _mirror1_slot, withdrawal_amount) = registry
            .new_action::<DigInitiatePayoutAction>()
            .spend(ctx, &mut registry, mirror1_slot)?;

        ensure_conditions_met(ctx, &mut sim, payout_conditions.extend(sync_conditions), 0)?;

        let _registry = registry.finish_spend(ctx, vec![])?;
        sim.set_next_timestamp(update_time)?;
        sim.spend_coins(ctx.take(), &[])?;

        let payout_coin_id = reserve_cat
            .wrapped_child(mirror1_puzzle_hash, withdrawal_amount)
            .coin
            .coin_id();

        assert!(sim.coin_state(payout_coin_id).is_some());
        assert_eq!(sim.coin_state(payout_coin_id).unwrap().coin.amount, 12602);

        Ok(())
    }
}

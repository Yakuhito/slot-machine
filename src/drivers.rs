use bip39::Mnemonic;
use chia::{
    bls::{sign, PublicKey, SecretKey, Signature},
    clvm_utils::ToTreeHash,
    consensus::consensus_constants::ConsensusConstants,
    protocol::{Bytes32, Coin, CoinSpend},
    puzzles::{
        cat::{CatArgs, CatSolution},
        offer::{NotarizedPayment, Payment, SettlementPaymentsSolution},
        singleton::{SingletonArgs, SingletonSolution, SingletonStruct},
        standard::{StandardArgs, StandardSolution},
        CoinProof, EveProof, LineageProof, Proof,
    },
};
use chia_puzzles::{CAT_PUZZLE_HASH, SETTLEMENT_PAYMENT_HASH, SINGLETON_TOP_LAYER_V1_1_HASH};
use chia_wallet_sdk::{
    driver::{
        Cat, CatSpend, CurriedPuzzle, DriverError, HashedPtr, Launcher, Layer, Nft, Offer, Puzzle,
        SingleCatSpend, Spend, SpendContext, StandardLayer,
    },
    prelude::{AggSig, AggSigKind},
    signer::{AggSigConstants, RequiredBlsSignature},
    types::{announcement_id, puzzles::SettlementPayment, Condition, Conditions},
};
use clvm_traits::{clvm_list, clvm_quote, clvm_tuple, FromClvm, ToClvm};
use clvmr::{Allocator, NodePtr};

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState,
    CatalogSlotValue, DefaultCatMakerArgs, P2DelegatedBySingletonLayerArgs, Reserve,
    RewardDistributor, RewardDistributorConstants, RewardDistributorInfo,
    RewardDistributorRewardSlotValue, RewardDistributorSlotNonce, RewardDistributorState, Slot,
    SlotInfo, SlotProof, XchandlesConstants, XchandlesRegistry, XchandlesRegistryInfo,
    XchandlesRegistryState, XchandlesSlotValue,
};

pub struct SecuredOneSidedOffer {
    pub coin_spends: Vec<CoinSpend>,
    pub aggregated_signature: Signature,
    pub security_coin: Coin,
    pub security_base_conditions: Conditions<NodePtr>,
    pub created_cat: Option<Cat>,
    pub created_nft: Option<Nft<HashedPtr>>,
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
    getrandom::fill(&mut entropy).map_err(custom_err)?;
    let mnemonic = Mnemonic::from_entropy(&entropy).map_err(custom_err)?;
    let seed = mnemonic.to_seed("");
    let sk = SecretKey::from_seed(&seed);
    Ok(sk)
}

pub fn parse_one_sided_offer(
    ctx: &mut SpendContext,
    offer: Offer,
    security_public_key: PublicKey,
    cat_notarized_payment: Option<NotarizedPayment>,
    nft_notatized_payment: Option<NotarizedPayment>,
) -> Result<SecuredOneSidedOffer, DriverError> {
    let offer = offer.parse(ctx).map_err(custom_err)?;

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
    let mut created_nft: Option<Nft<HashedPtr>> = None;

    for coin_spend in offer.coin_spends {
        let puzzle_ptr = coin_spend.puzzle_reveal.to_clvm(ctx)?;
        let solution_ptr = coin_spend.solution.to_clvm(ctx)?;

        let curried_puzzle = CurriedPuzzle::parse(ctx, puzzle_ptr);

        if let Some(curried_puzzle) = curried_puzzle {
            if curried_puzzle.mod_hash == CAT_PUZZLE_HASH.into() {
                let cat_args = CatArgs::<NodePtr>::from_clvm(ctx, curried_puzzle.args)?;
                let cat_solution = CatSolution::<NodePtr>::from_clvm(ctx, solution_ptr)?;

                let inner_output =
                    ctx.run(cat_args.inner_puzzle, cat_solution.inner_puzzle_solution)?;
                let inner_output = Vec::<Condition<NodePtr>>::from_clvm(ctx, inner_output)?;

                if let Some(cc) = inner_output
                    .into_iter()
                    .filter_map(|cond| {
                        let Condition::CreateCoin(cc) = cond else {
                            return None;
                        };

                        Some(cc)
                    })
                    .find(|cc| cc.puzzle_hash == SETTLEMENT_PAYMENT_HASH.into())
                {
                    let Some(cat_notarized_payment) = cat_notarized_payment.clone() else {
                        return Err(DriverError::Custom(
                            "CAT payment not requested but offered CAT found".to_string(),
                        ));
                    };

                    // we found a CAT creating an offered CAT - spend it to create
                    // the offer CAT, which then creates a 0-amount coin with
                    // puzzle hash = cat_destination_puzzle_hash; also return the cat
                    // by returning it to its original puzzle hash

                    // and assert the announcement to make sure it's not going
                    // somewhere else
                    let offer_cat_full_puzzle_hash =
                        CatArgs::curry_tree_hash(cat_args.asset_id, SETTLEMENT_PAYMENT_HASH.into());
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
                        SETTLEMENT_PAYMENT_HASH.into(),
                    );

                    // offers don't allow amount 0 coins, so we create an 1-amount coin,
                    //   which creates a 0-amount coin and spend all the CATs together
                    if cat_notarized_payment.payments[0].amount == 0 {
                        let memos = Some(chia_wallet_sdk::prelude::Memos::new(
                            ctx.alloc(&cat_notarized_payment.payments[0].memos)?,
                        ));
                        let interim_inner_puzzle = ctx.alloc(&clvm_quote!(Conditions::new()
                            .create_coin(
                                cat_notarized_payment.payments[0].puzzle_hash,
                                0,
                                memos,
                            )))?;
                        let interim_inner_ph: Bytes32 = ctx.tree_hash(interim_inner_puzzle).into();

                        let notarized_payment = NotarizedPayment {
                            nonce: offer_coin_cat.coin_id(),
                            payments: vec![
                                Payment::with_memos(
                                    refund_puzzle_hash,
                                    cc.amount,
                                    vec![refund_puzzle_hash.into()],
                                ),
                                Payment::new(interim_inner_ph, 1),
                            ],
                        };
                        let offer_cat_inner_solution = ctx.alloc(&SettlementPaymentsSolution {
                            notarized_payments: vec![notarized_payment.clone()],
                        })?;

                        let offer_cat_spend = CatSpend::new(
                            offer_cat,
                            Spend::new(
                                ctx.alloc_mod::<SettlementPayment>()?,
                                offer_cat_inner_solution,
                            ),
                        );

                        let interim_cat = offer_cat.wrapped_child(interim_inner_ph, 1);
                        let interim_cat_spend = CatSpend::new(
                            interim_cat,
                            Spend::new(interim_inner_puzzle, NodePtr::NIL),
                        );

                        let orig_spends = ctx.take();

                        Cat::spend_all(ctx, &[offer_cat_spend, interim_cat_spend])?;
                        coin_spends.extend(ctx.take());

                        for og_spend in orig_spends {
                            ctx.insert(og_spend);
                        }

                        created_cat = Some(
                            interim_cat
                                .wrapped_child(cat_notarized_payment.payments[0].puzzle_hash, 0),
                        );
                        let announcement_msg: Bytes32 = notarized_payment.tree_hash().into();
                        base_conditions = base_conditions.assert_puzzle_announcement(
                            announcement_id(offer_cat.coin.puzzle_hash, announcement_msg),
                        );
                    } else {
                        let offer_cat_inner_solution = ctx.alloc(&SettlementPaymentsSolution {
                            notarized_payments: vec![cat_notarized_payment.clone()],
                        })?;

                        let offer_cat_inner_puzzle = ctx.alloc_mod::<SettlementPayment>()?;
                        offer_cat.spend(
                            ctx,
                            SingleCatSpend {
                                prev_coin_id: offer_cat.coin.coin_id(),
                                next_coin_proof: CoinProof {
                                    parent_coin_info: offer_cat.coin.parent_coin_info,
                                    inner_puzzle_hash: offer_cat.p2_puzzle_hash,
                                    amount: cc.amount,
                                },
                                prev_subtotal: 0,
                                extra_delta: 0,
                                inner_spend: Spend::new(
                                    offer_cat_inner_puzzle,
                                    offer_cat_inner_solution,
                                ),
                            },
                        )?;

                        created_cat = Some(offer_cat.wrapped_child(
                            cat_notarized_payment.payments[0].puzzle_hash,
                            cc.amount,
                        ));
                        let announcement_msg: Bytes32 = cat_notarized_payment.tree_hash().into();
                        base_conditions = base_conditions.assert_puzzle_announcement(
                            announcement_id(offer_cat.coin.puzzle_hash, announcement_msg),
                        );
                    }
                }
            } else if curried_puzzle.mod_hash == SINGLETON_TOP_LAYER_V1_1_HASH.into() {
                let Some(nft_notatized_payment) = nft_notatized_payment.clone() else {
                    return Err(DriverError::Custom(
                        "NFT payment not requested but offered NFT found".to_string(),
                    ));
                };

                let parent_puzzle = Puzzle::from_clvm(ctx, puzzle_ptr)?;
                let Some(nft) = Nft::<HashedPtr>::parse_child(
                    ctx,
                    coin_spend.coin,
                    parent_puzzle,
                    solution_ptr,
                )?
                else {
                    return Err(DriverError::Custom("Could not parse child NFT".to_string()));
                };

                let inner_puzzle = ctx.alloc_mod::<SettlementPayment>()?;
                let inner_solution = ctx.alloc(&SettlementPaymentsSolution {
                    notarized_payments: vec![nft_notatized_payment.clone()],
                })?;

                nft.spend(ctx, Spend::new(inner_puzzle, inner_solution))?;

                created_nft = Some(nft.wrapped_child::<HashedPtr>(
                    nft_notatized_payment.payments[0].puzzle_hash,
                    nft.info.current_owner,
                    nft.info.metadata,
                ));
                let announcement_msg: Bytes32 = nft_notatized_payment.tree_hash().into();
                base_conditions = base_conditions.assert_puzzle_announcement(announcement_id(
                    nft.coin.puzzle_hash,
                    announcement_msg,
                ));
            }
        }

        if security_coin_parent_id.is_none() {
            let res = ctx.run(puzzle_ptr, solution_ptr)?;
            let res = Vec::<Condition<NodePtr>>::from_clvm(ctx, res)?;

            if let Some(cc) = res
                .into_iter()
                .filter_map(|cond| {
                    let Condition::CreateCoin(cc) = cond else {
                        return None;
                    };

                    Some(cc)
                })
                .find(|cc| cc.puzzle_hash == SETTLEMENT_PAYMENT_HASH.into())
            {
                let offer_coin = Coin::new(
                    coin_spend.coin.coin_id(),
                    SETTLEMENT_PAYMENT_HASH.into(),
                    cc.amount,
                );
                let offer_coin_id = offer_coin.coin_id();

                security_coin_parent_id = Some(offer_coin_id);
                security_coin_amount = cc.amount;

                let settlement_payments_puzzle = ctx.alloc_mod::<SettlementPayment>()?;
                let offer_coin_puzzle = ctx.serialize(&settlement_payments_puzzle)?;

                let offer_coin_solution = SettlementPaymentsSolution {
                    notarized_payments: vec![NotarizedPayment {
                        nonce: offer_coin_id,
                        payments: vec![Payment::new(
                            security_coin_puzzle_hash,
                            security_coin_amount,
                        )],
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

    if cat_notarized_payment.is_some() && created_cat.is_none() {
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
        created_nft,
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

    let quoted_conditions_ptr = ctx.alloc(&clvm_quote!(conditions))?;
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
    let output = Vec::<Condition<NodePtr>>::from_clvm(ctx, output)?;
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

pub fn eve_singleton_inner_puzzle<S>(
    ctx: &mut SpendContext,
    launcher_id: Bytes32,
    left_slot_value: S,
    right_slot_value: S,
    memos_after_hint: NodePtr,
    target_inner_puzzle_hash: Bytes32,
) -> Result<NodePtr, DriverError>
where
    S: Copy + ToTreeHash,
{
    let left_slot_info = SlotInfo::from_value(launcher_id, 0, left_slot_value);
    let left_slot_puzzle_hash = Slot::<S>::puzzle_hash(&left_slot_info);

    let right_slot_info = SlotInfo::from_value(launcher_id, 0, right_slot_value);
    let right_slot_puzzle_hash = Slot::<S>::puzzle_hash(&right_slot_info);

    let slot_hint: Bytes32 = Slot::<()>::first_curry_hash(launcher_id, 0).into();
    let slot_memos = ctx.hint(slot_hint)?;
    let launcher_id_ptr = ctx.alloc(&launcher_id)?;
    let launcher_memos = ctx.memos(&clvm_tuple!(launcher_id_ptr, memos_after_hint))?;

    clvm_quote!(Conditions::new()
        .create_coin(left_slot_puzzle_hash.into(), 0, Some(slot_memos))
        .create_coin(right_slot_puzzle_hash.into(), 0, Some(slot_memos))
        .create_coin(target_inner_puzzle_hash, 1, Some(launcher_memos)))
    .to_clvm(ctx)
    .map_err(DriverError::ToClvm)
}

// Spends the eve signleton, whose only job is to create the
//   slot 'premine' (leftmost and rightmost slots) and
//   transition to the actual registry puzzle
#[allow(clippy::type_complexity)]
fn spend_eve_coin_and_create_registry<S, M, KV>(
    ctx: &mut SpendContext,
    launcher: Launcher,
    target_inner_puzzle_hash: Bytes32,
    left_slot_value: S,
    right_slot_value: S,
    memos_after_hint: M,
    launcher_kv_list: KV,
) -> Result<(Conditions, Coin, Proof, [Slot<S>; 2]), DriverError>
where
    S: Copy + ToTreeHash,
    M: ToClvm<Allocator>,
    KV: ToClvm<Allocator>,
{
    let launcher_coin = launcher.coin();
    let launcher_id = launcher_coin.coin_id();

    let memos_after_hint = ctx.alloc(&memos_after_hint)?;
    let eve_singleton_inner_puzzle = eve_singleton_inner_puzzle(
        ctx,
        launcher_id,
        left_slot_value,
        right_slot_value,
        memos_after_hint,
        target_inner_puzzle_hash,
    )?;

    let eve_singleton_inner_puzzle_hash = ctx.tree_hash(eve_singleton_inner_puzzle);
    let eve_singleton_proof = Proof::Eve(EveProof {
        parent_parent_coin_info: launcher_coin.parent_coin_info,
        parent_amount: launcher_coin.amount,
    });

    let (security_coin_conditions, eve_coin) = launcher.with_singleton_amount(1).spend(
        ctx,
        eve_singleton_inner_puzzle_hash.into(),
        launcher_kv_list,
    )?;

    let eve_coin_solution = SingletonSolution {
        lineage_proof: eve_singleton_proof,
        amount: 1,
        inner_solution: NodePtr::NIL,
    }
    .to_clvm(ctx)?;

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
    let left_slot = Slot::new(
        slot_proof,
        SlotInfo::from_value(launcher_id, 0, left_slot_value),
    );
    let right_slot = Slot::new(
        slot_proof,
        SlotInfo::from_value(launcher_id, 0, right_slot_value),
    );

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
    let offer = parse_one_sided_offer(ctx, offer, security_coin_sk.public_key(), None, None)?;
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
    let catalog_registry_info = CatalogRegistryInfo::new(
        initial_state,
        catalog_constants.with_launcher_id(registry_launcher_id),
    );
    let catalog_inner_puzzle_hash = catalog_registry_info.clone().inner_puzzle_hash();

    let (new_security_coin_conditions, new_catalog_registry_coin, catalog_proof, slots) =
        spend_eve_coin_and_create_registry(
            ctx,
            registry_launcher,
            catalog_inner_puzzle_hash.into(),
            CatalogSlotValue::initial_left_end(),
            CatalogSlotValue::initial_right_end(),
            clvm_tuple!(
                initial_registration_asset_id,
                clvm_tuple!(initial_state, ())
            ),
            (),
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

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn launch_xchandles_registry<V>(
    ctx: &mut SpendContext,
    offer: Offer,
    initial_base_registration_price: u64,
    // (registry launcher id, security coin, additional_args) -> (additional conditions, registry constants, initial_registration_asset_id)
    get_additional_info: fn(
        ctx: &mut SpendContext,
        Bytes32,
        Coin,
        V,
    ) -> Result<
        (Conditions<NodePtr>, XchandlesConstants, Bytes32),
        DriverError,
    >,
    consensus_constants: &ConsensusConstants,
    additional_args: V,
) -> Result<
    (
        Signature,
        SecretKey,
        XchandlesRegistry,
        [Slot<XchandlesSlotValue>; 2],
        Coin, // security coin
    ),
    DriverError,
> {
    let security_coin_sk = new_sk()?;
    let offer = parse_one_sided_offer(ctx, offer, security_coin_sk.public_key(), None, None)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = offer.security_base_conditions;

    // Create registry coin launcher
    let registry_launcher = Launcher::new(security_coin_id, 1);
    let registry_launcher_coin = registry_launcher.coin();
    let registry_launcher_id = registry_launcher_coin.coin_id();

    let (additional_security_coin_conditions, xchandles_constants, initial_registration_asset_id) =
        get_additional_info(
            ctx,
            registry_launcher_id,
            offer.security_coin,
            additional_args,
        )?;

    // Spend intermediary coin and create registry
    let initial_state = XchandlesRegistryState::from(
        initial_registration_asset_id.tree_hash().into(),
        initial_base_registration_price,
    );
    let target_xchandles_info = XchandlesRegistryInfo::new(
        initial_state,
        xchandles_constants.with_launcher_id(registry_launcher_id),
    );

    let target_xchandles_inner_puzzle_hash = target_xchandles_info.clone().inner_puzzle_hash();
    let (new_security_coin_conditions, new_xchandles_coin, xchandles_proof, slots) =
        spend_eve_coin_and_create_registry(
            ctx,
            registry_launcher,
            target_xchandles_inner_puzzle_hash.into(),
            XchandlesSlotValue::initial_left_end(),
            XchandlesSlotValue::initial_right_end(),
            (),
            clvm_list!(
                initial_registration_asset_id,
                initial_base_registration_price,
                initial_state,
                target_xchandles_info.constants
            ),
        )?;

    // this creates the launcher & secures the spend
    security_coin_conditions = security_coin_conditions
        .extend(new_security_coin_conditions)
        .extend(additional_security_coin_conditions);

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
        offer.security_coin,
    ))
}

#[allow(clippy::type_complexity)]
pub fn launch_dig_reward_distributor(
    ctx: &mut SpendContext,
    offer: Offer,
    first_epoch_start: u64,
    cat_refund_puzzle_hash: Bytes32,
    constants: RewardDistributorConstants,
    consensus_constants: &ConsensusConstants,
    comment: &str,
) -> Result<
    (
        Signature,
        SecretKey,
        RewardDistributor,
        Slot<RewardDistributorRewardSlotValue>,
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
        Some(NotarizedPayment {
            nonce: constants.launcher_id,
            payments: vec![Payment::with_memos(
                cat_refund_puzzle_hash,
                0,
                vec![cat_refund_puzzle_hash.into()],
            )],
        }),
        None,
    )?;
    let dig_reward_distributor_hint: Bytes32 = "DIG Reward Distributor v1".tree_hash().into();
    let launcher_memos = chia_wallet_sdk::prelude::Memos::new(
        ctx.alloc(&(dig_reward_distributor_hint, (comment, ())))?,
    );
    let launcher = Launcher::with_memos(mock_offer.security_coin.coin_id(), 1, launcher_memos);
    let launcher_coin = launcher.coin();
    let launcher_id = launcher_coin.coin_id();

    let controller_singleton_struct_hash: Bytes32 =
        SingletonStruct::new(launcher_id).tree_hash().into();
    let reserve_inner_ph: Bytes32 =
        P2DelegatedBySingletonLayerArgs::curry_tree_hash(controller_singleton_struct_hash, 0)
            .into();

    let offer = parse_one_sided_offer(
        ctx,
        offer,
        security_coin_sk.public_key(),
        Some(NotarizedPayment {
            nonce: constants.launcher_id,
            payments: vec![Payment::with_memos(
                reserve_inner_ph,
                0,
                vec![reserve_inner_ph.into()],
            )],
        }),
        None,
    )?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let mut security_coin_conditions = offer.security_base_conditions;

    // Spend intermediary coin and create registry
    let target_info = RewardDistributorInfo::new(
        RewardDistributorState::initial(first_epoch_start),
        constants.with_launcher_id(launcher_id),
    );

    let target_inner_puzzle_hash = target_info.clone().inner_puzzle_hash();

    let slot_value = RewardDistributorRewardSlotValue {
        epoch_start: first_epoch_start,
        next_epoch_initialized: false,
        rewards: 0,
    };
    let slot_info = SlotInfo::<RewardDistributorRewardSlotValue>::from_value(
        launcher_id,
        RewardDistributorSlotNonce::REWARD.to_u64(),
        slot_value,
    );
    let slot_puzzle_hash = Slot::<RewardDistributorRewardSlotValue>::puzzle_hash(&slot_info);

    let slot_hint: Bytes32 =
        Slot::<()>::first_curry_hash(launcher_id, RewardDistributorSlotNonce::REWARD.to_u64())
            .into();
    let slot_memos = ctx.hint(slot_hint)?;
    let launcher_memos = ctx.hint(launcher_id)?;
    let eve_singleton_inner_puzzle = clvm_quote!(Conditions::new()
        .create_coin(slot_puzzle_hash.into(), 0, Some(slot_memos))
        .create_coin(target_inner_puzzle_hash.into(), 1, Some(launcher_memos)))
    .to_clvm(ctx)?;

    let eve_singleton_inner_puzzle_hash = ctx.tree_hash(eve_singleton_inner_puzzle);
    let eve_singleton_proof = Proof::Eve(EveProof {
        parent_parent_coin_info: launcher_coin.parent_coin_info,
        parent_amount: launcher_coin.amount,
    });

    let (launch_conditions, eve_coin) = launcher.with_singleton_amount(1).spend(
        ctx,
        eve_singleton_inner_puzzle_hash.into(),
        (first_epoch_start, target_info.constants),
    )?;
    security_coin_conditions = security_coin_conditions.extend(launch_conditions);

    let eve_coin_solution = SingletonSolution {
        lineage_proof: eve_singleton_proof,
        amount: 1,
        inner_solution: NodePtr::NIL,
    }
    .to_clvm(ctx)?;

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
    let registry = RewardDistributor::new(new_registry_coin, new_proof, target_info, reserve);

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
            singleton::{SingletonSolution, SingletonStruct},
            CoinProof,
        },
    };
    use chia_puzzles::SINGLETON_LAUNCHER_HASH;
    use chia_wallet_sdk::{
        driver::{Nft, NftMint, SpendWithConditions},
        test::Simulator,
        types::TESTNET11_CONSTANTS,
    };
    use clvm_traits::clvm_list;
    use clvmr::Allocator;
    use hex_literal::hex;

    use crate::{
        benchmarker::tests::Benchmark, print_spend_bundle_to_file, CatNftMetadata,
        CatalogPrecommitValue, CatalogRefundAction, CatalogRegisterAction, CatalogSlotValue,
        DelegatedStateAction, DelegatedStateActionSolution, IntermediaryCoinProof,
        NftLauncherProof, PrecommitCoin, RewardDistributorAddEntryAction,
        RewardDistributorAddIncentivesAction, RewardDistributorCommitIncentivesAction,
        RewardDistributorInitiatePayoutAction, RewardDistributorNewEpochAction,
        RewardDistributorRemoveEntryAction, RewardDistributorStakeAction,
        RewardDistributorSyncAction, RewardDistributorType,
        RewardDistributorWithdrawIncentivesAction, Slot, SpendContextExt, XchandlesExpireAction,
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
        let checker_puzzle_ptr = clvm_quote!(conditions).to_clvm(ctx)?;
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
        let test_singleton_launcher_coin = sim.new_coin(SINGLETON_LAUNCHER_HASH.into(), 1);
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
            .send_message(18, message.into(), vec![ctx.alloc(&receiver_puzzle_hash)?])
            .create_coin(price_singleton_inner_puzzle_hash.into(), 1, None);

        let price_singleton_inner_solution = price_singleton_inner_solution.to_clvm(ctx)?;
        let price_singleton_solution = SingletonSolution {
            lineage_proof: price_singleton_proof,
            amount: 1,
            inner_solution: price_singleton_inner_solution,
        }
        .to_clvm(ctx)?;

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
                new_state: new_state.to_clvm(ctx)?,
                other_singleton_inner_puzzle_hash: price_singleton_inner_puzzle_hash.into(),
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn test_refund_for_catalog(
        ctx: &mut SpendContext,
        sim: &mut Simulator,
        benchmark: &mut Benchmark,
        benchmark_label: &str,
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
            SingletonStruct::new(catalog.info.constants.launcher_id)
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

        // sim.spend_coins(ctx.take(), sks)?;
        benchmark.add_spends(ctx, sim, "create_precommit", sks)?;

        let slot = slots
            .iter()
            .find(|s| s.info.value.asset_id == tail_hash.into());

        let mut catalog = catalog;
        let secure_cond = catalog.new_action::<CatalogRefundAction>().spend(
            ctx,
            &mut catalog,
            tail_hash.into(),
            if let Some(found_slot) = slot {
                found_slot.info.value.neighbors.tree_hash().into()
            } else {
                Bytes32::default()
            },
            precommit_coin,
            slot.cloned(),
        )?;

        let new_catalog = catalog.finish_spend(ctx)?;

        ensure_conditions_met(ctx, sim, secure_cond, 0)?;

        // sim.spend_coins(ctx.take(), sks)?;
        benchmark.add_spends(ctx, sim, benchmark_label, sks)?;

        Ok((new_catalog, new_payment_cat))
    }

    #[test]
    fn test_catalog() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();
        let mut benchmark = Benchmark::new("CATalog".to_string());

        // setup config

        let initial_registration_price = 2000;
        let test_price_schedule = [1000, 500, 250];

        let catalog_constants = CatalogRegistryConstants {
            launcher_id: Bytes32::from([1; 32]),
            royalty_address: Bytes32::from([7; 32]),
            royalty_ten_thousandths: 100,
            precommit_payout_puzzle_hash: Bytes32::from([8; 32]),
            relative_block_height: 1,
            price_singleton_launcher_id: Bytes32::from(hex!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
        };

        // Create source offer
        let user_bls = sim.bls(0);

        let offer_amount = 1;
        let launcher_bls = sim.bls(offer_amount);

        let offer_src_coin = launcher_bls.coin;
        let offer_spend = StandardLayer::new(launcher_bls.pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(SETTLEMENT_PAYMENT_HASH.into(), offer_amount, None),
        )?;

        let puzzle_reveal = ctx.serialize(&offer_spend.puzzle)?;
        let solution = ctx.serialize(&offer_spend.solution)?;
        let offer = Offer::new(SpendBundle {
            coin_spends: vec![CoinSpend::new(offer_src_coin, puzzle_reveal, solution)],
            aggregated_signature: sign_standard_transaction(
                ctx,
                offer_src_coin,
                offer_spend,
                &launcher_bls.sk,
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
        let minter_bls = sim.bls(payment_cat_amount);
        let minter_p2 = StandardLayer::new(minter_bls.pk);

        let (issue_cat, mut payment_cat) = Cat::single_issuance_eve(
            ctx,
            minter_bls.coin.coin_id(),
            payment_cat_amount,
            Conditions::new().create_coin(minter_bls.puzzle_hash, payment_cat_amount, None),
        )?;
        minter_p2.spend(ctx, minter_bls.coin, issue_cat)?;

        payment_cat = payment_cat.wrapped_child(minter_bls.puzzle_hash, payment_cat_amount);
        sim.spend_coins(ctx.take(), &[minter_bls.sk.clone()])?;

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

        // sim.spend_coins(ctx.take(), &[launcher_bls.sk, security_sk])?;
        benchmark.add_spends(ctx, &mut sim, "launch", &[launcher_bls.sk, security_sk])?;

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
            let user_coin = sim.new_coin(user_bls.puzzle_hash, reg_amount);
            // pretty much a random TAIL - we're not actually launching it
            tail = ctx.curry(GenesisByCoinIdTailArgs::new(user_coin.coin_id()))?;
            let tail_hash = ctx.tree_hash(tail);

            let eve_nft_inner_puzzle = clvm_quote!(Conditions::new().create_coin(
                Bytes32::new([4 + i as u8; 32]),
                1,
                None
            ))
            .to_clvm(ctx)?;
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
                SingletonStruct::new(catalog.info.constants.launcher_id)
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
                        minter_bls.puzzle_hash,
                        payment_cat_amount - reg_amount,
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

            payment_cat_amount -= reg_amount;
            payment_cat = payment_cat.wrapped_child(minter_bls.puzzle_hash, payment_cat_amount);

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone(), minter_bls.sk.clone()])?;
            benchmark.add_spends(
                ctx,
                &mut sim,
                "create_precommit",
                &[user_bls.sk.clone(), minter_bls.sk.clone()],
            )?;

            // call the 'register' action on CATalog
            slots.sort_unstable_by(|a, b| a.info.value.cmp(&b.info.value));

            let slot_value_to_insert =
                CatalogSlotValue::new(tail_hash.into(), Bytes32::default(), Bytes32::default());

            let mut left_slot: Option<Slot<CatalogSlotValue>> = None;
            let mut right_slot: Option<Slot<CatalogSlotValue>> = None;
            for slot in slots.iter() {
                let slot_value = slot.info.value;

                if slot_value < slot_value_to_insert {
                    // slot belongs to the left
                    if left_slot.is_none() || slot_value > left_slot.unwrap().info.value {
                        left_slot = Some(*slot);
                    }
                } else {
                    // slot belongs to the right
                    if right_slot.is_none() || slot_value < right_slot.unwrap().info.value {
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
                // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
                benchmark.add_spends(ctx, &mut sim, "update_price", &[user_bls.sk.clone()])?;
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

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
            benchmark.add_spends(ctx, &mut sim, "register", &[user_bls.sk.clone()])?;

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
            &mut benchmark,
            "refund_amount_wrong",
            catalog.info.state.registration_price + 1,
            payment_cat,
            None,
            catalog,
            &catalog_constants,
            &slots,
            user_bls.puzzle_hash,
            minter_p2,
            minter_bls.puzzle_hash,
            &[user_bls.sk.clone(), minter_bls.sk.clone()],
        )?;

        // a - the CAT maker puzzle has changed
        // i.e., use different payment CAT
        let alternative_payment_cat_amount = 10_000_000;
        let minter2_bls = sim.bls(alternative_payment_cat_amount);
        let minter_p2_2 = StandardLayer::new(minter2_bls.pk);

        let (issue_cat, mut alternative_payment_cat) = Cat::single_issuance_eve(
            ctx,
            minter2_bls.coin.coin_id(),
            alternative_payment_cat_amount,
            Conditions::new().create_coin(
                minter2_bls.puzzle_hash,
                alternative_payment_cat_amount,
                None,
            ),
        )?;
        minter_p2_2.spend(ctx, minter2_bls.coin, issue_cat)?;

        alternative_payment_cat = alternative_payment_cat
            .wrapped_child(minter2_bls.puzzle_hash, alternative_payment_cat_amount);
        sim.spend_coins(ctx.take(), &[minter2_bls.sk.clone()])?;

        let (catalog, _alternative_payment_cat) = test_refund_for_catalog(
            ctx,
            &mut sim,
            &mut benchmark,
            "refund_cat_changed",
            catalog.info.state.registration_price,
            alternative_payment_cat,
            None,
            catalog,
            &catalog_constants,
            &slots,
            user_bls.puzzle_hash,
            minter_p2_2,
            minter2_bls.puzzle_hash,
            &[user_bls.sk.clone(), minter2_bls.sk.clone()],
        )?;

        // c - the tail hash has already been registered
        let (_catalog, _payment_cat) = test_refund_for_catalog(
            ctx,
            &mut sim,
            &mut benchmark,
            "refund_cat_already_registered",
            catalog.info.state.registration_price,
            payment_cat,
            Some(tail),
            catalog,
            &catalog_constants,
            &slots,
            user_bls.puzzle_hash,
            minter_p2,
            minter_bls.puzzle_hash,
            &[user_bls.sk.clone(), minter_bls.sk.clone()],
        )?;

        benchmark.print_summary(Some("catalog.costs"));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn test_refund_for_xchandles(
        ctx: &mut SpendContext,
        sim: &mut Simulator,
        benchmark: &mut Benchmark,
        benchmark_label: &str,
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
                existing_slot.info.value.expiration + 28 * 24 * 60 * 60 + 1
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
            SingletonStruct::new(registry.info.constants.launcher_id)
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

        // sim.spend_coins(ctx.take(), &[user_sk.clone(), minter_sk.clone()])?;
        benchmark.add_spends(
            ctx,
            sim,
            "create_precommit",
            &[user_sk.clone(), minter_sk.clone()],
        )?;

        let mut registry = registry;
        let used_slot_value_hash = slot.map(|s| s.info.value_hash);
        let (secure_cond, _new_slot_maybe) = registry.new_action::<XchandlesRefundAction>().spend(
            ctx,
            &mut registry,
            precommit_coin,
            pricing_puzzle,
            pricing_solution,
            slot,
        )?;
        assert_eq!(
            used_slot_value_hash,
            XchandlesRefundAction::get_spent_slot_value_hash_from_solution(
                ctx,
                registry.pending_items.actions[registry.pending_items.actions.len() - 1].solution
            )?
        );

        let new_registry = registry.finish_spend(ctx)?;

        ensure_conditions_met(ctx, sim, secure_cond.clone(), 0)?;

        // sim.spend_coins(ctx.take(), &[user_sk.clone()])?;
        benchmark.add_spends(ctx, sim, benchmark_label, &[user_sk.clone()])?;

        Ok((new_registry, new_payment_cat))
    }

    #[test]
    fn test_xchandles() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();
        let mut benchmark = Benchmark::new("XCHandles".to_string());
        // setup config
        let initial_registration_price = 2000;
        let test_price_schedule = [1000, 500, 250];

        let xchandles_constants = XchandlesConstants {
            launcher_id: Bytes32::from([1; 32]),
            precommit_payout_puzzle_hash: Bytes32::from([8; 32]),
            relative_block_height: 1,
            price_singleton_launcher_id: Bytes32::from(hex!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
        };

        // Create source offer
        let user_bls = sim.bls(0);
        let user_p2 = StandardLayer::new(user_bls.pk);

        let offer_amount = 1;
        let launcher_bls = sim.bls(offer_amount);
        let offer_spend = StandardLayer::new(launcher_bls.pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(SETTLEMENT_PAYMENT_HASH.into(), offer_amount, None),
        )?;

        let puzzle_reveal = ctx.serialize(&offer_spend.puzzle)?;
        let solution = ctx.serialize(&offer_spend.solution)?;
        let offer = Offer::new(SpendBundle {
            coin_spends: vec![CoinSpend::new(launcher_bls.coin, puzzle_reveal, solution)],
            aggregated_signature: sign_standard_transaction(
                ctx,
                launcher_bls.coin,
                offer_spend,
                &launcher_bls.sk,
                &TESTNET11_CONSTANTS,
            )?,
        });

        // Launch CAT
        let mut payment_cat_amount = 10_000_000;
        let minter_bls = sim.bls(payment_cat_amount);
        let minter_p2 = StandardLayer::new(minter_bls.pk);

        let (issue_cat, mut payment_cat) = Cat::single_issuance_eve(
            ctx,
            minter_bls.coin.coin_id(),
            payment_cat_amount,
            Conditions::new().create_coin(minter_bls.puzzle_hash, payment_cat_amount, None),
        )?;
        minter_p2.spend(ctx, minter_bls.coin, issue_cat)?;

        payment_cat = payment_cat.wrapped_child(minter_bls.puzzle_hash, payment_cat_amount);

        sim.spend_coins(ctx.take(), &[minter_bls.sk.clone()])?;

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
        let (_, security_sk, mut registry, slots_returned_by_launch, _security_coin) =
            launch_xchandles_registry(
                ctx,
                offer,
                initial_registration_price,
                |_ctx, _launcher_id, _coin, (xchandles_constants, payment_cat_asset_id)| {
                    Ok((Conditions::new(), xchandles_constants, payment_cat_asset_id))
                },
                &TESTNET11_CONSTANTS,
                (
                    xchandles_constants.with_price_singleton(price_singleton_launcher_id),
                    payment_cat.asset_id,
                ),
            )?;

        // Check XCHandlesRegistry::from_launcher_solution
        let spends = ctx.take();
        let mut initial_slots = None;
        for spend in spends {
            if spend.coin.puzzle_hash == SINGLETON_LAUNCHER_HASH.into() {
                let launcher_solution = ctx.alloc(&spend.solution)?;

                if let Some((registry, slots, initial_registration_asset_id, initial_base_price)) =
                    XchandlesRegistry::from_launcher_solution(ctx, spend.coin, launcher_solution)?
                {
                    initial_slots = Some(slots);
                    assert_eq!(initial_registration_asset_id, payment_cat.asset_id);
                    assert_eq!(
                        registry.info.constants,
                        xchandles_constants
                            .with_price_singleton(price_singleton_launcher_id)
                            .with_launcher_id(spend.coin.coin_id())
                    );
                    assert_eq!(initial_registration_price, initial_base_price);
                };
            }

            ctx.insert(spend);
        }

        // This will fail if we didn't find (or were not able to parse) the XCHandles launcher
        assert!(initial_slots.is_some());

        // sim.spend_coins(ctx.take(), &[launcher_bls.sk, security_sk])?;
        benchmark.add_spends(ctx, &mut sim, "launch", &[launcher_bls.sk, security_sk])?;

        let slots = initial_slots.unwrap();
        assert!(sim.coin_state(slots[0].coin.coin_id()).is_some());
        assert!(sim.coin_state(slots[1].coin.coin_id()).is_some());
        assert_eq!(slots, slots_returned_by_launch);

        // Register 7 handles

        let mut base_price = initial_registration_price;

        let mut slots: Vec<Slot<XchandlesSlotValue>> = slots.into();
        for i in 0..7 {
            // mint controller singleton (it's a DID, not an NFT - don't rat on me to the NFT board plz)
            let launcher_coin = sim.new_coin(SINGLETON_LAUNCHER_HASH.into(), 1);
            let launcher = Launcher::new(launcher_coin.parent_coin_info, 1);
            let (_, did) = launcher.create_simple_did(ctx, &user_p2)?;

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
                SingletonStruct::new(registry.info.constants.launcher_id)
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
                    .create_coin(
                        minter_bls.puzzle_hash,
                        payment_cat_amount - reg_amount,
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

            payment_cat_amount -= reg_amount;
            payment_cat = payment_cat.wrapped_child(minter_bls.puzzle_hash, payment_cat_amount);

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone(), minter_bls.sk.clone()])?;
            benchmark.add_spends(
                ctx,
                &mut sim,
                "create_precommit",
                &[user_bls.sk.clone(), minter_bls.sk.clone()],
            )?;

            // call the 'register' action on the registry
            slots.sort_unstable_by(|a, b| a.info.value.cmp(&b.info.value));

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
                let slot_value = slot.info.value;

                if slot_value < slot_value_to_insert {
                    // slot belongs to the left
                    if left_slot.is_none() || slot_value > left_slot.unwrap().info.value {
                        left_slot = Some(*slot);
                    }
                } else {
                    // slot belongs to the right
                    if right_slot.is_none() || slot_value < right_slot.unwrap().info.value {
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
                // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
                benchmark.add_spends(ctx, &mut sim, "update_price", &[user_bls.sk.clone()])?;
            };

            let spent_value_hashes = [left_slot.info.value_hash, right_slot.info.value_hash];
            let (secure_cond, new_slots) = registry.new_action::<XchandlesRegisterAction>().spend(
                ctx,
                &mut registry,
                left_slot,
                right_slot,
                precommit_coin,
                base_price,
            )?;

            ensure_conditions_met(ctx, &mut sim, secure_cond.clone(), 1)?;

            assert_eq!(
                spent_value_hashes,
                XchandlesRegisterAction::get_spent_slot_value_hashes_from_solution(
                    ctx,
                    registry.pending_items.actions[registry.pending_items.actions.len() - 1]
                        .solution
                )?
            );
            registry = registry.finish_spend(ctx)?;
            sim.pass_time(100); // registration start was at timestamp 100

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
            benchmark.add_spends(ctx, &mut sim, "register", &[user_bls.sk.clone()])?;

            slots.retain(|s| *s != left_slot && *s != right_slot);

            let oracle_slot = new_slots[1];
            slots.extend(new_slots);

            // test on-chain oracle for current handle
            let spent_slot_value_hash = oracle_slot.info.value_hash;
            let (oracle_conds, new_slot) = registry.new_action::<XchandlesOracleAction>().spend(
                ctx,
                &mut registry,
                oracle_slot,
            )?;

            ensure_conditions_met(ctx, &mut sim, oracle_conds, 0)?;

            assert_eq!(
                spent_slot_value_hash,
                XchandlesOracleAction::get_spent_slot_value_hash_from_solution(
                    ctx,
                    registry.pending_items.actions[registry.pending_items.actions.len() - 1]
                        .solution
                )?
            );
            registry = registry.finish_spend(ctx)?;

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
            benchmark.add_spends(ctx, &mut sim, "oracle", &[user_bls.sk.clone()])?;

            slots.retain(|s| *s != oracle_slot);
            slots.push(new_slot);

            // test on-chain extend mechanism for current handle
            let extension_years: u64 = i as u64 + 1;
            let extension_slot = new_slot;
            let pay_for_extension: u64 =
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &handle, extension_years);

            let spent_slot_value_hash = extension_slot.info.value_hash;
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

            assert_eq!(
                spent_slot_value_hash,
                XchandlesExtendAction::get_spent_slot_value_hash_from_solution(
                    ctx,
                    registry.pending_items.actions[registry.pending_items.actions.len() - 1]
                        .solution
                )?
            );

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                extend_conds
                    .create_coin(SETTLEMENT_PAYMENT_HASH.into(), pay_for_extension, None)
                    .create_coin(
                        minter_bls.puzzle_hash,
                        payment_cat_amount - pay_for_extension,
                        None,
                    ),
            )?;

            let cat_offer_inner_spend = Spend::new(
                ctx.alloc_mod::<SettlementPayment>()?,
                ctx.alloc(&clvm_list!(notarized_payment))?,
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
                        cat: payment_cat
                            .wrapped_child(SETTLEMENT_PAYMENT_HASH.into(), pay_for_extension),
                        inner_spend: cat_offer_inner_spend,
                        extra_delta: 0,
                    },
                ],
            )?;

            payment_cat_amount -= pay_for_extension;
            payment_cat = payment_cat.wrapped_child(minter_bls.puzzle_hash, payment_cat_amount);

            registry = registry.finish_spend(ctx)?;

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone(), minter_bls.sk.clone()])?;
            benchmark.add_spends(
                ctx,
                &mut sim,
                "extend",
                &[user_bls.sk.clone(), minter_bls.sk.clone()],
            )?;

            slots.retain(|s| *s != extension_slot);
            slots.push(new_slot);

            // test on-chain mechanism for handle updates
            let new_owner_launcher_id = Bytes32::new([4 + i as u8; 32]);
            let new_resolved_launcher_id = Bytes32::new([u8::MAX - i as u8 - 1; 32]);
            let update_slot = new_slot;
            let update_slot_value_hash = new_slot.info.value_hash;

            let (update_conds, new_slot) = registry.new_action::<XchandlesUpdateAction>().spend(
                ctx,
                &mut registry,
                update_slot,
                new_owner_launcher_id,
                new_resolved_launcher_id,
                did.info.inner_puzzle_hash().into(),
            )?;

            let _new_did = did.update(ctx, &user_p2, update_conds)?;

            assert_eq!(
                update_slot_value_hash,
                XchandlesUpdateAction::get_spent_slot_value_hash_from_solution(
                    ctx,
                    registry.pending_items.actions[registry.pending_items.actions.len() - 1]
                        .solution
                )?
            );
            registry = registry.finish_spend(ctx)?;

            // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
            benchmark.add_spends(ctx, &mut sim, "update", &[user_bls.sk.clone()])?;

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
            .find(|s| s.info.value.handle_hash == handle_hash)
            .unwrap();

        // precommit coin needed
        let refund_puzzle = ctx.alloc(&1)?;
        let refund_puzzle_hash = ctx.tree_hash(refund_puzzle);
        let expiration = initial_slot.info.value.expiration;
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
            SingletonStruct::new(registry.info.constants.launcher_id)
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
                .create_coin(
                    minter_bls.puzzle_hash,
                    payment_cat_amount - reg_amount,
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

        payment_cat =
            payment_cat.wrapped_child(minter_bls.puzzle_hash, payment_cat.coin.amount - reg_amount);

        sim.set_next_timestamp(buy_time)?;
        // sim.spend_coins(ctx.take(), &[user_bls.sk.clone(), minter_bls.sk.clone()])?;
        benchmark.add_spends(
            ctx,
            &mut sim,
            "create_precommit",
            &[user_bls.sk.clone(), minter_bls.sk.clone()],
        )?;

        let spent_slot_value_hash = initial_slot.info.value_hash;
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

        assert_eq!(
            spent_slot_value_hash,
            XchandlesExpireAction::get_spent_slot_value_hash_from_solution(
                ctx,
                registry.pending_items.actions[registry.pending_items.actions.len() - 1].solution
            )?
        );
        registry = registry.finish_spend(ctx)?;

        // sim.spend_coins(ctx.take(), &[user_bls.sk.clone()])?;
        benchmark.add_spends(ctx, &mut sim, "expire", &[user_bls.sk.clone()])?;

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
                ctx.alloc(&XchandlesFactorPricingSolution {
                    current_expiration: 0,
                    handle: unregistered_handle.clone(),
                    num_years: 1,
                })?
            } else {
                ctx.alloc(&XchandlesExponentialPremiumRenewPuzzleSolution {
                    buy_time: 28 * 24 * 60 * 60 + 1, // premium should be 0
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        current_expiration: 0,
                        handle: unregistered_handle.clone(),
                        num_years: 1,
                    },
                })?
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
                .find(|s| s.info.value.handle_hash == existing_handle.tree_hash().into())
                .unwrap();
            let existing_handle_pricing_solution = if use_factor_pricing {
                ctx.alloc(&XchandlesFactorPricingSolution {
                    current_expiration: existing_slot.info.value.expiration,
                    handle: existing_handle.clone(),
                    num_years: 1,
                })?
            } else {
                ctx.alloc(&XchandlesExponentialPremiumRenewPuzzleSolution {
                    buy_time: existing_slot.info.value.expiration + 28 * 24 * 60 * 60 + 1, // premium should be 0
                    pricing_program_solution: XchandlesFactorPricingSolution {
                        current_expiration: existing_slot.info.value.expiration,
                        handle: existing_handle.clone(),
                        num_years: 1,
                    },
                })?
            };
            let existing_handle_expected_price =
                XchandlesFactorPricingPuzzleArgs::get_price(base_price, &existing_handle, 1);

            // a - the CAT maker puzzle has changed
            let alternative_payment_cat_amount = 10_000_000;
            let minter2 = sim.bls(alternative_payment_cat_amount);
            let minter_p2_2 = StandardLayer::new(minter2.pk);

            let (issue_cat, mut alternative_payment_cat) = Cat::single_issuance_eve(
                ctx,
                minter2.coin.coin_id(),
                alternative_payment_cat_amount,
                Conditions::new().create_coin(
                    minter2.puzzle_hash,
                    alternative_payment_cat_amount,
                    None,
                ),
            )?;
            minter_p2_2.spend(ctx, minter2.coin, issue_cat)?;

            alternative_payment_cat = alternative_payment_cat
                .wrapped_child(minter2.puzzle_hash, alternative_payment_cat_amount);
            sim.spend_coins(ctx.take(), &[minter2.sk.clone()])?;

            registry = test_refund_for_xchandles(
                ctx,
                &mut sim,
                &mut benchmark,
                "refund_cat_wrong",
                unregistered_handle.clone(),
                pricing_puzzle,
                pricing_solution,
                None,
                alternative_payment_cat,
                expected_price,
                registry,
                minter_p2_2,
                minter2.puzzle_hash,
                &minter2.sk,
                &user_bls.sk,
            )?
            .0;

            // b - the amount is wrong
            (registry, payment_cat) = test_refund_for_xchandles(
                ctx,
                &mut sim,
                &mut benchmark,
                "refund_amount_wrong",
                unregistered_handle.clone(),
                pricing_puzzle,
                pricing_solution,
                None,
                payment_cat,
                expected_price + 1,
                registry,
                minter_p2,
                minter_bls.puzzle_hash,
                &minter_bls.sk,
                &user_bls.sk,
            )?;

            // c - the pricing puzzle has changed
            (registry, payment_cat) = test_refund_for_xchandles(
                ctx,
                &mut sim,
                &mut benchmark,
                "refund_pricing_wrong",
                unregistered_handle.clone(),
                other_pricing_puzzle,
                pricing_solution,
                None,
                payment_cat,
                other_expected_price,
                registry,
                minter_p2,
                minter_bls.puzzle_hash,
                &minter_bls.sk,
                &user_bls.sk,
            )?;

            // d - the handle has already been registered
            (registry, payment_cat) = test_refund_for_xchandles(
                ctx,
                &mut sim,
                &mut benchmark,
                "refund_handle_already_registered",
                existing_handle.clone(), // already registered handle
                pricing_puzzle,
                existing_handle_pricing_solution,
                Some(existing_slot),
                payment_cat,
                existing_handle_expected_price,
                registry,
                minter_p2,
                minter_bls.puzzle_hash,
                &minter_bls.sk,
                &user_bls.sk,
            )?;
        }

        benchmark.print_summary(Some("xchandles.costs"));

        Ok(())
    }

    #[test]
    fn test_nft_with_any_metadata_updater() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        let bls = sim.bls(1);
        let p2 = StandardLayer::new(bls.pk);

        let nft_launcher = Launcher::new(bls.coin.coin_id(), 1);

        let royalty_puzzle_hash = Bytes32::from([7; 32]);
        let (create_nft, nft) = nft_launcher.mint_nft(
            ctx,
            NftMint::<CatNftMetadata> {
                metadata: cat_nft_metadata_for_testing(),
                metadata_updater_puzzle_hash: ANY_METADATA_UPDATER_HASH.into(),
                royalty_puzzle_hash,
                royalty_ten_thousandths: 100,
                p2_puzzle_hash: bls.puzzle_hash,
                owner: None,
            },
        )?;
        p2.spend(ctx, bls.coin, create_nft)?;

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
            solution: ctx.alloc(&new_metadata)?,
        };

        let new_nft: Nft<CatNftMetadata> = nft.transfer_with_metadata(
            ctx,
            &p2,
            bls.puzzle_hash,
            metadata_update,
            Conditions::new(),
        )?;

        assert_eq!(new_nft.info.metadata, new_metadata);
        sim.spend_coins(ctx.take(), &[bls.sk])?;
        Ok(())
    }

    // Spends the manager singleton
    fn spend_manager_singleton(
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
            .to_clvm(ctx)?;
        let test_singleton_solution = ctx.alloc(&SingletonSolution {
            lineage_proof: test_singleton_proof,
            amount: 1,
            inner_solution: test_singleton_inner_solution,
        })?;

        let test_singleton_spend = Spend::new(test_singleton_puzzle, test_singleton_solution);
        ctx.spend(test_singleton_coin, test_singleton_spend)?;

        // compute manager singleton info for next spend
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
    fn test_managed_reward_distributor() -> anyhow::Result<()> {
        test_reward_distributor(RewardDistributorType::Manager)
    }

    #[test]
    fn test_nft_reward_distributor() -> anyhow::Result<()> {
        test_reward_distributor(RewardDistributorType::Nft)
    }

    fn test_reward_distributor(manager_type: RewardDistributorType) -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();
        let mut benchmark = Benchmark::new(format!(
            "Reward Distributor ({})",
            match manager_type {
                RewardDistributorType::Manager => "Manager",
                RewardDistributorType::Nft => "NFT",
            }
        ));

        // Launch token CAT
        let cat_amount = 10_000_000_000;
        let cat_minter = sim.bls(cat_amount);
        let cat_minter_p2 = StandardLayer::new(cat_minter.pk);

        let (issue_cat, mut source_cat) = Cat::single_issuance_eve(
            ctx,
            cat_minter.coin.coin_id(),
            cat_amount,
            Conditions::new().create_coin(cat_minter.puzzle_hash, cat_amount, None),
        )?;
        cat_minter_p2.spend(ctx, cat_minter.coin, issue_cat)?;

        source_cat = source_cat.wrapped_child(cat_minter.puzzle_hash, cat_amount);
        sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;

        // Launch manager singleton
        let (
            manager_or_did_launcher_id,
            mut manager_or_did_coin,
            mut manager_or_did_singleton_proof,
            _manager_or_didsingleton_inner_puzzle,
            manager_or_did_singleton_inner_puzzle_hash,
            manager_or_did_singleton_puzzle,
        ) = launch_test_singleton(ctx, &mut sim)?;

        // setup config
        let constants = RewardDistributorConstants::without_launcher_id(
            manager_type,
            manager_or_did_launcher_id,
            Bytes32::new([1; 32]),
            1000,
            300,
            42,
            420,  // 4.2% fee
            9000, // 90% of the amount deposited will be returned
            source_cat.asset_id,
        );

        // Create source offer
        let entry1_bls = sim.bls(0);
        let entry2_bls = sim.bls(0);

        let offer_amount = 1;
        let launcher_bls = sim.bls(offer_amount);
        let offer_spend = StandardLayer::new(launcher_bls.pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(SETTLEMENT_PAYMENT_HASH.into(), offer_amount, None),
        )?;

        let puzzle_reveal = ctx.serialize(&offer_spend.puzzle)?;
        let solution = ctx.serialize(&offer_spend.solution)?;

        let cat_minter_inner_puzzle = clvm_quote!(Conditions::new().create_coin(
            SETTLEMENT_PAYMENT_HASH.into(),
            source_cat.coin.amount,
            None
        ))
        .to_clvm(ctx)?;
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
                    inner_puzzle_hash: cat_minter.puzzle_hash,
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
                CoinSpend::new(launcher_bls.coin, puzzle_reveal, solution),
                cat_offer_spend,
            ],
            aggregated_signature: sign_standard_transaction(
                ctx,
                launcher_bls.coin,
                offer_spend,
                &launcher_bls.sk,
                &TESTNET11_CONSTANTS,
            )?,
        });

        // Launch the reward distributor
        let first_epoch_start = 1234;
        let (_, security_sk, mut registry, first_epoch_slot) = launch_dig_reward_distributor(
            ctx,
            offer,
            first_epoch_start,
            Bytes32::default(),
            constants,
            &TESTNET11_CONSTANTS,
            "yak yak yak",
        )?;

        // sim.spend_coins(
        //     ctx.take(),
        //     &[
        //         launcher_bls.sk.clone(),
        //         security_sk.clone(),
        //         cat_minter.sk.clone(),
        //     ],
        // )?;
        benchmark.add_spends(
            ctx,
            &mut sim,
            "launch",
            &[
                launcher_bls.sk.clone(),
                security_sk.clone(),
                cat_minter.sk.clone(),
            ],
        )?;

        source_cat = source_cat
            .wrapped_child(SETTLEMENT_PAYMENT_HASH.into(), source_cat.coin.amount)
            .wrapped_child(cat_minter.puzzle_hash, source_cat.coin.amount);
        assert!(sim.coin_state(source_cat.coin.coin_id()).is_some());

        let nft_bls = sim.bls(1);

        // add the 1st entry/NFT before reward epoch ('first epoch') begins
        let (entry1_slot, _nft1) = if manager_type == RewardDistributorType::Manager {
            let (manager_conditions, entry1_slot) = registry
                .new_action::<RewardDistributorAddEntryAction>()
                .spend(
                    ctx,
                    &mut registry,
                    entry1_bls.puzzle_hash,
                    1,
                    manager_or_did_singleton_inner_puzzle_hash,
                )?;
            registry = registry.finish_spend(ctx, vec![])?;

            (manager_or_did_coin, manager_or_did_singleton_proof) = spend_manager_singleton(
                ctx,
                manager_or_did_coin,
                manager_or_did_singleton_proof,
                manager_or_did_singleton_puzzle,
                manager_conditions,
            )?;

            // sim.spend_coins(ctx.take(), &[])?;
            benchmark.add_spends(ctx, &mut sim, "add_entry", &[])?;

            (entry1_slot, None)
        } else {
            let nft_launcher =
                Launcher::new(manager_or_did_coin.coin_id(), 0).with_singleton_amount(1);
            let nft_launcher_coin = nft_launcher.coin();
            let meta = ctx.alloc(&"nft1")?;
            let meta = HashedPtr::from_ptr(ctx, meta);

            let (conds, nft) = nft_launcher.mint_nft(
                ctx,
                NftMint::new(meta, nft_bls.puzzle_hash, 10, None)
                    .with_royalty_puzzle_hash(Bytes32::from([1; 32]))
                    .with_custom_metadata_updater(ANY_METADATA_UPDATER_HASH.into()),
            )?;

            (manager_or_did_coin, manager_or_did_singleton_proof) = spend_manager_singleton(
                ctx,
                manager_or_did_coin,
                manager_or_did_singleton_proof,
                manager_or_did_singleton_puzzle,
                conds,
            )?;

            ensure_conditions_met(
                ctx,
                &mut sim,
                Conditions::new().assert_concurrent_spend(nft_launcher_coin.coin_id()),
                1,
            )?;

            benchmark.add_spends(ctx, &mut sim, "mint_nft", &[])?;

            let Proof::Lineage(did_proof) = manager_or_did_singleton_proof else {
                panic!("did_proof is not a lineage proof");
            };
            let nft_proof = NftLauncherProof {
                did_proof,
                intermediary_coin_proofs: vec![IntermediaryCoinProof {
                    full_puzzle_hash: nft_launcher_coin.puzzle_hash,
                    amount: nft_launcher_coin.amount,
                }],
            };

            let (sec_conds, notarized_payment, entry1_slot, locked_nft) = registry
                .new_action::<RewardDistributorStakeAction>()
                .spend(ctx, &mut registry, nft, nft_proof, nft_bls.puzzle_hash)?;
            registry = registry.finish_spend(ctx, vec![])?;

            ensure_conditions_met(ctx, &mut sim, sec_conds, 0)?;

            let offer_nft =
                nft.wrapped_child(SETTLEMENT_PAYMENT_HASH.into(), None, nft.info.metadata);

            let nft_inner_spend = Spend::new(
                ctx.alloc(&clvm_quote!(Conditions::new().create_coin(
                    SETTLEMENT_PAYMENT_HASH.into(),
                    1,
                    None
                )))?,
                NodePtr::NIL,
            );
            let nft_inner_spend =
                StandardLayer::new(nft_bls.pk).delegated_inner_spend(ctx, nft_inner_spend)?;
            nft.spend(ctx, nft_inner_spend)?;

            let nft_inner_spend = Spend::new(
                ctx.alloc_mod::<SettlementPayment>()?,
                ctx.alloc(&SettlementPaymentsSolution {
                    notarized_payments: vec![notarized_payment],
                })?,
            );
            offer_nft.spend(ctx, nft_inner_spend)?;

            // sim.spend_coins(spends, &[nft_bls.sk.clone()])?;
            benchmark.add_spends(ctx, &mut sim, "stake_nft", &[nft_bls.sk.clone()])?;

            (entry1_slot, Some(locked_nft))
        };
        println!("got here, yay!"); // todo: debug

        // commit incentives for first epoch
        let rewards_to_add = constants.epoch_seconds;
        let (secure_conditions, first_epoch_commitment_slot, mut incentive_slots) = registry
            .new_action::<RewardDistributorCommitIncentivesAction>()
            .spend(
                ctx,
                &mut registry,
                first_epoch_slot,
                first_epoch_start,
                cat_minter.puzzle_hash,
                rewards_to_add,
            )?;

        // spend reserve and source cat together so deltas add up
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter.puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        // sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;
        benchmark.add_spends(ctx, &mut sim, "commit_incentives", &[cat_minter.sk.clone()])?;
        source_cat = source_cat.wrapped_child(
            cat_minter.puzzle_hash,
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
        let (secure_conditions, fifth_epoch_commitment_slot, new_incentive_slots) = registry
            .new_action::<RewardDistributorCommitIncentivesAction>()
            .spend(
                ctx,
                &mut registry,
                *incentive_slots.last().unwrap(),
                fifth_epoch_start,
                cat_minter.puzzle_hash,
                rewards_to_add,
            )?;

        let new_value_keys = new_incentive_slots
            .iter()
            .map(|s| s.info.value.epoch_start)
            .collect::<Vec<_>>();
        incentive_slots.retain(|s| !new_value_keys.contains(&s.info.value.epoch_start));
        incentive_slots.extend(new_incentive_slots);

        // spend reserve and source cat together so deltas add up
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter.puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        // sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;
        benchmark.add_spends(ctx, &mut sim, "commit_incentives", &[cat_minter.sk.clone()])?;

        source_cat = source_cat.wrapped_child(
            cat_minter.puzzle_hash,
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
        let (secure_conditions, fifth_epoch_commitment_slot2, new_incentive_slots) = registry
            .new_action::<RewardDistributorCommitIncentivesAction>()
            .spend(
                ctx,
                &mut registry,
                *incentive_slots
                    .iter()
                    .find(|s| s.info.value.epoch_start == fifth_epoch_start)
                    .unwrap(),
                fifth_epoch_start,
                cat_minter.puzzle_hash,
                rewards_to_add,
            )?;

        let new_value_keys = new_incentive_slots
            .iter()
            .map(|s| s.info.value.epoch_start)
            .collect::<Vec<_>>();
        incentive_slots.retain(|s| !new_value_keys.contains(&s.info.value.epoch_start));
        incentive_slots.extend(new_incentive_slots);

        // spend reserve and source cat together so deltas add up
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter.puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        // sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;
        benchmark.add_spends(ctx, &mut sim, "commit_incentives", &[cat_minter.sk.clone()])?;

        source_cat = source_cat.wrapped_child(
            cat_minter.puzzle_hash,
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
        let (withdraw_incentives_conditions, new_reward_slot, withdrawn_amount) = registry
            .new_action::<RewardDistributorWithdrawIncentivesAction>()
            .spend(
                ctx,
                &mut registry,
                fifth_epoch_commitment_slot,
                *incentive_slots
                    .iter()
                    .find(|s| s.info.value.epoch_start == fifth_epoch_start)
                    .unwrap(),
            )?;

        let payout_coin_id = registry
            .reserve
            .to_cat()
            .wrapped_child(
                cat_minter.puzzle_hash, // fifth_epoch_commitment_slot.info.value.unwrap().clawback_ph,
                withdrawn_amount,
            )
            .coin
            .coin_id();

        let claimer_coin = sim.new_coin(cat_minter.puzzle_hash, 0);
        cat_minter_p2.spend(ctx, claimer_coin, withdraw_incentives_conditions)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.set_next_timestamp(first_epoch_start)?;
        // sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;
        benchmark.add_spends(
            ctx,
            &mut sim,
            "withdraw_incentives",
            &[cat_minter.sk.clone()],
        )?;

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
        incentive_slots
            .retain(|s| s.info.value.epoch_start != new_reward_slot.info.value.epoch_start);
        incentive_slots.push(new_reward_slot);

        // start first epoch
        let reserve_cat = registry.reserve.to_cat();
        let first_epoch_incentives_slot = *incentive_slots
            .iter()
            .find(|s| s.info.value.epoch_start == first_epoch_start)
            .unwrap();
        let (new_epoch_conditions, new_reward_slot, fee) = registry
            .new_action::<RewardDistributorNewEpochAction>()
            .spend(
                ctx,
                &mut registry,
                first_epoch_incentives_slot,
                // first_epoch_incentives_slot.info.value.rewards,
            )?;
        let payout_coin_id = reserve_cat
            .wrapped_child(constants.fee_payout_puzzle_hash, fee)
            .coin
            .coin_id();

        ensure_conditions_met(ctx, &mut sim, new_epoch_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.pass_time(100);
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "new_epoch", &[])?;

        assert!(sim.coin_state(payout_coin_id).is_some());
        assert_eq!(registry.info.state.active_shares, 1);
        assert_eq!(registry.info.state.total_reserves, 4000 - fee);
        assert_eq!(registry.info.state.round_reward_info.cumulative_payout, 0);
        assert_eq!(
            registry.info.state.round_reward_info.remaining_rewards,
            first_epoch_incentives_slot.info.value.rewards - fee
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
        incentive_slots
            .retain(|s| s.info.value.epoch_start != new_reward_slot.info.value.epoch_start);
        incentive_slots.push(new_reward_slot);

        // sync to 10%
        let initial_reward_info = registry.info.state.round_reward_info;
        let sync_conditions = registry.new_action::<RewardDistributorSyncAction>().spend(
            ctx,
            &mut registry,
            first_epoch_start + 100,
        )?;
        ensure_conditions_met(ctx, &mut sim, sync_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.pass_time(400);
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "sync", &[])?;

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
        let sync_conditions = registry.new_action::<RewardDistributorSyncAction>().spend(
            ctx,
            &mut registry,
            first_epoch_start + 500,
        )?;
        ensure_conditions_met(ctx, &mut sim, sync_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "sync", &[])?;
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

        let add_incentives_conditions = registry
            .new_action::<RewardDistributorAddIncentivesAction>()
            .spend(ctx, &mut registry, incentives_amount)?;

        // spend reserve and source cat together so deltas add up
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                add_incentives_conditions.create_coin(
                    cat_minter.puzzle_hash,
                    source_cat.coin.amount - incentives_amount,
                    None,
                ),
            )?,
        );

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        // sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;
        benchmark.add_spends(ctx, &mut sim, "add_incentives", &[cat_minter.sk.clone()])?;

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
                + (incentives_amount - incentives_amount * constants.fee_bps / 10000)
        );
        source_cat = source_cat.wrapped_child(
            cat_minter.puzzle_hash,
            source_cat.coin.amount - incentives_amount,
        );

        // add entry2
        let (manager_conditions, entry2_slot) = registry
            .new_action::<RewardDistributorAddEntryAction>()
            .spend(
                ctx,
                &mut registry,
                entry2_bls.puzzle_hash,
                2,
                manager_or_did_singleton_inner_puzzle_hash,
            )?;
        (manager_or_did_coin, manager_or_did_singleton_proof) = spend_manager_singleton(
            ctx,
            manager_or_did_coin,
            manager_or_did_singleton_proof,
            manager_or_did_singleton_puzzle,
            manager_conditions,
        )?;

        registry = registry.finish_spend(ctx, vec![])?;
        sim.pass_time(250);
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "add_entry", &[])?;
        assert_eq!(registry.info.state.active_shares, 3);

        // sync to 75% (so + 25%)
        let initial_reward_info = registry.info.state.round_reward_info;
        let sync_conditions = registry.new_action::<RewardDistributorSyncAction>().spend(
            ctx,
            &mut registry,
            first_epoch_start + 750,
        )?;
        ensure_conditions_met(ctx, &mut sim, sync_conditions, 0)?;

        registry = registry.finish_spend(ctx, vec![])?;
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "sync", &[])?;
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

        // remove entry2
        let reserve_cat = registry.reserve.to_cat();
        let (remove_entry_manager_conditions, entry2_payout_amount) = registry
            .new_action::<RewardDistributorRemoveEntryAction>()
            .spend(
                ctx,
                &mut registry,
                entry2_slot,
                manager_or_did_singleton_inner_puzzle_hash,
            )?;

        let (_manager_coin, _manager_singleton_proof) = spend_manager_singleton(
            ctx,
            manager_or_did_coin,
            manager_or_did_singleton_proof,
            manager_or_did_singleton_puzzle,
            remove_entry_manager_conditions,
        )?;

        registry = registry.finish_spend(ctx, vec![])?;
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "remove_entry", &[])?;
        let payout_coin_id = reserve_cat
            .wrapped_child(entry2_bls.puzzle_hash, entry2_payout_amount)
            .coin
            .coin_id();

        assert!(registry.info.state.active_shares == 1);
        assert!(sim.coin_state(payout_coin_id).is_some());
        assert!(sim
            .coin_state(entry2_slot.coin.coin_id())
            .unwrap()
            .spent_height
            .is_some());

        for epoch in 1..7 {
            let update_time = registry.info.state.round_time_info.epoch_end;
            let first_update_time =
                (registry.info.state.round_time_info.last_update + update_time) / 2;
            let sync_conditions1 = registry.new_action::<RewardDistributorSyncAction>().spend(
                ctx,
                &mut registry,
                first_update_time,
            )?;

            let sync_conditions2 = registry.new_action::<RewardDistributorSyncAction>().spend(
                ctx,
                &mut registry,
                update_time,
            )?;

            let reward_slot = *incentive_slots
                .iter()
                .find(|s| {
                    s.info.value.epoch_start
                        == first_epoch_start
                            + if epoch <= 4 { epoch } else { 4 } * constants.epoch_seconds
                })
                .unwrap();

            let (new_epoch_conditions, new_reward_slot, _manager_fee) = registry
                .new_action::<RewardDistributorNewEpochAction>()
                .spend(ctx, &mut registry, reward_slot)?;
            incentive_slots
                .retain(|s| s.info.value.epoch_start != new_reward_slot.info.value.epoch_start);
            incentive_slots.push(new_reward_slot);

            ensure_conditions_met(
                ctx,
                &mut sim,
                sync_conditions1
                    .extend(sync_conditions2)
                    .extend(new_epoch_conditions),
                0,
            )?;

            registry = registry.finish_spend(ctx, vec![])?;
            sim.set_next_timestamp(update_time)?;
            // sim.spend_coins(ctx.take(), &[])?;
            benchmark.add_spends(ctx, &mut sim, "sync_and_new_epoch", &[])?;
        }

        // commit incentives for 10th epoch
        let tenth_epoch_start = first_epoch_start + constants.epoch_seconds * 9;
        let rewards_to_add = constants.epoch_seconds * 10;
        let (secure_conditions, tenth_epoch_commitment_slot, new_incentive_slots) = registry
            .new_action::<RewardDistributorCommitIncentivesAction>()
            .spend(
                ctx,
                &mut registry,
                *incentive_slots.last().unwrap(),
                tenth_epoch_start,
                cat_minter.puzzle_hash,
                rewards_to_add,
            )?;

        let new_value_keys = new_incentive_slots
            .iter()
            .map(|s| s.info.value.epoch_start)
            .collect::<Vec<_>>();
        incentive_slots.retain(|s| !new_value_keys.contains(&s.info.value.epoch_start));
        incentive_slots.extend(new_incentive_slots);

        // spend reserve and source cat together so deltas add up
        let source_cat_spend = CatSpend::new(
            source_cat,
            cat_minter_p2.spend_with_conditions(
                ctx,
                secure_conditions.create_coin(
                    cat_minter.puzzle_hash,
                    source_cat.coin.amount - rewards_to_add,
                    None,
                ),
            )?,
        );

        registry = registry.finish_spend(ctx, vec![source_cat_spend])?;
        // sim.spend_coins(ctx.take(), &[cat_minter.sk.clone()])?;
        benchmark.add_spends(ctx, &mut sim, "commit_incentives", &[cat_minter.sk.clone()])?;
        let _source_cat = source_cat.wrapped_child(
            cat_minter.puzzle_hash,
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
            let sync_conditions = registry.new_action::<RewardDistributorSyncAction>().spend(
                ctx,
                &mut registry,
                update_time,
            )?;

            let reward_slot = *incentive_slots
                .iter()
                .find(|s| {
                    s.info.value.epoch_start == first_epoch_start + epoch * constants.epoch_seconds
                })
                .unwrap();
            let (new_epoch_conditions, new_reward_slot, _manager_fee) = registry
                .new_action::<RewardDistributorNewEpochAction>()
                .spend(
                    ctx,
                    &mut registry,
                    reward_slot,
                    // reward_slot.info.value.rewards,
                )?;
            incentive_slots
                .retain(|s| s.info.value.epoch_start != new_reward_slot.info.value.epoch_start);
            incentive_slots.push(new_reward_slot);

            ensure_conditions_met(
                ctx,
                &mut sim,
                sync_conditions.extend(new_epoch_conditions),
                0,
            )?;

            registry = registry.finish_spend(ctx, vec![])?;
            sim.set_next_timestamp(update_time)?;
            // sim.spend_coins(ctx.take(), &[])?;
            benchmark.add_spends(ctx, &mut sim, "sync", &[])?;
        }

        let update_time = registry.info.state.round_time_info.epoch_end - 100;
        let sync_conditions = registry.new_action::<RewardDistributorSyncAction>().spend(
            ctx,
            &mut registry,
            update_time,
        )?;

        // payout entry
        let reserve_cat = registry.reserve.to_cat();
        let (payout_conditions, _entry1_slot, withdrawal_amount) = registry
            .new_action::<RewardDistributorInitiatePayoutAction>()
            .spend(ctx, &mut registry, entry1_slot)?;

        ensure_conditions_met(ctx, &mut sim, payout_conditions.extend(sync_conditions), 0)?;

        let _registry = registry.finish_spend(ctx, vec![])?;
        sim.set_next_timestamp(update_time)?;
        // sim.spend_coins(ctx.take(), &[])?;
        benchmark.add_spends(ctx, &mut sim, "initiate_payout", &[])?;

        let payout_coin_id = reserve_cat
            .wrapped_child(entry1_bls.puzzle_hash, withdrawal_amount)
            .coin
            .coin_id();

        assert!(sim.coin_state(payout_coin_id).is_some());
        assert_eq!(sim.coin_state(payout_coin_id).unwrap().coin.amount, 12602);

        benchmark.print_summary(Some("dig.costs"));

        Ok(())
    }
}

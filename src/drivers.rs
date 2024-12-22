use bip39::Mnemonic;
use chia::{
    bls::{sign, SecretKey, Signature},
    clvm_utils::{CurriedProgram, ToTreeHash},
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
    Offer, RequiredSignature, Spend, SpendContext, StandardLayer,
};
use clvm_traits::{clvm_quote, FromClvm, ToClvm};
use clvmr::NodePtr;

use crate::{
    CatalogRegistry, CatalogRegistryConstants, CatalogRegistryInfo, CatalogRegistryState,
    CatalogSlotValue, Slot, SlotInfo, SlotProof, XchandlesConstants, XchandlesRegistry,
    XchandlesRegistryInfo, XchandlesRegistryState, XchandlesSlotValue, SLOT32_MAX_VALUE,
    SLOT32_MIN_VALUE,
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
                            memos: vec![],
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

    let required_signature = RequiredSignature::from_condition(
        &coin,
        AggSig::new(
            AggSigKind::Me,
            agg_sig_me.public_key,
            agg_sig_me.message.clone(),
        ),
        &AggSigConstants::new(consensus_constants.agg_sig_me_additional_data),
    );

    Ok(sign(sk, required_signature.final_message()))
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

    let left_slot_info = SlotInfo::from_value(launcher_id, left_slot_value);
    let left_slot_puzzle_hash = Slot::<S>::puzzle_hash(&left_slot_info);

    let right_slot_info = SlotInfo::from_value(launcher_id, right_slot_value);
    let right_slot_puzzle_hash = Slot::<S>::puzzle_hash(&right_slot_info);

    let slot_hint: Bytes32 = Slot::<()>::first_curry_hash(launcher_id).into();
    let eve_singleton_inner_puzzle = clvm_quote!(Conditions::new()
        .create_coin(left_slot_puzzle_hash.into(), 0, vec![slot_hint.into()])
        .create_coin(right_slot_puzzle_hash.into(), 0, vec![slot_hint.into()])
        .create_coin(target_inner_puzzle_hash, 1, vec![launcher_id.into()],))
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

    let eve_singleton_puzzle = CurriedProgram {
        program: ctx.singleton_top_layer()?,
        args: SingletonArgs::new(launcher_id, eve_singleton_inner_puzzle),
    }
    .to_clvm(&mut ctx.allocator)?;
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
            registration_asset_id_hash: initial_registration_asset_id.tree_hash().into(),
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
        XchandlesRegistryState {
            registration_base_price: initial_base_registration_price,
            registration_asset_id_hash: initial_registration_asset_id.tree_hash().into(),
        },
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
            ),
            XchandlesSlotValue::new(
                SLOT32_MAX_VALUE.into(),
                SLOT32_MIN_VALUE.into(),
                SLOT32_MAX_VALUE.into(),
                u64::MAX,
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

#[cfg(test)]
mod tests {
    use chia::{
        clvm_utils::CurriedProgram,
        protocol::SpendBundle,
        puzzles::{
            cat::GenesisByCoinIdTailArgs,
            singleton::{SingletonSolution, SINGLETON_LAUNCHER_PUZZLE_HASH},
        },
    };
    use chia_wallet_sdk::{
        test_secret_keys, Cat, CatSpend, Nft, NftMint, Simulator, SpendWithConditions,
        TESTNET11_CONSTANTS,
    };
    use clvm_traits::clvm_list;
    use hex_literal::hex;

    use crate::{
        print_spend_bundle_to_file, CatNftMetadata, CatalogPrecommitValue, CatalogRegistryAction,
        CatalogSlotValue, DelegatedStateActionSolution, PrecommitCoin, Slot, SpendContextExt,
        XchandlesPrecommitValue, XchandlesRegisterAction, XchandlesRegistryAction,
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
    ) -> Result<(Bytes32, Coin, Proof, NodePtr), DriverError> {
        let price_singleton_launcher_coin = sim.new_coin(SINGLETON_LAUNCHER_PUZZLE_HASH.into(), 1);
        let price_singleton_launcher =
            Launcher::new(price_singleton_launcher_coin.parent_coin_info, 1);

        let price_singleton_launcher_id = price_singleton_launcher.coin().coin_id();

        let price_singleton_inner_puzzle = ctx.alloc(&1)?;
        let price_singleton_inner_puzzle_hash = ctx.tree_hash(price_singleton_inner_puzzle);
        let (_, price_singleton_coin) =
            price_singleton_launcher.spend(ctx, price_singleton_inner_puzzle_hash.into(), ())?;

        let price_singleton_puzzle = CurriedProgram {
            program: ctx.singleton_top_layer()?,
            args: SingletonArgs::new(price_singleton_launcher_id, price_singleton_inner_puzzle),
        }
        .to_clvm(&mut ctx.allocator)?;
        let price_singleton_proof: Proof = Proof::Eve(EveProof {
            parent_parent_coin_info: price_singleton_launcher_coin.parent_coin_info,
            parent_amount: price_singleton_launcher_coin.amount,
        });

        Ok((
            price_singleton_launcher_id,
            price_singleton_coin,
            price_singleton_proof,
            price_singleton_puzzle,
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
            .create_coin(price_singleton_inner_puzzle_hash.into(), 1, vec![]);

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
                vec![],
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
            Conditions::new().create_coin(minter_puzzle_hash, payment_cat_amount, vec![]),
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

        let mut slots: Vec<Slot<CatalogSlotValue>> = slots.into();
        for i in 0..7 {
            // create precommit coin
            let reg_amount = if i % 2 == 1 {
                test_price_schedule[i / 2]
            } else {
                catalog.info.state.registration_price
            };
            let user_coin = sim.new_coin(user_puzzle_hash, reg_amount);
            let tail = CurriedProgram {
                program: ctx.genesis_by_coin_id_tail_puzzle()?,
                args: GenesisByCoinIdTailArgs::new(user_coin.coin_id()),
            }
            .to_clvm(&mut ctx.allocator)?; // pretty much a random TAIL - we're not actually launching it
            let tail_hash = ctx.tree_hash(tail);

            let eve_nft_inner_puzzle = clvm_quote!(Conditions::new().create_coin(
                Bytes32::new([4 + i as u8; 32]),
                1,
                vec![]
            ))
            .to_clvm(&mut ctx.allocator)?;
            let eve_nft_inner_puzzle_hash = ctx.tree_hash(eve_nft_inner_puzzle);

            let value = CatalogPrecommitValue {
                initial_inner_puzzle_hash: eve_nft_inner_puzzle_hash.into(),
                tail_reveal: tail,
            };

            let precommit_coin = PrecommitCoin::new(
                ctx,
                payment_cat.coin.coin_id(),
                payment_cat.child_lineage_proof(),
                payment_cat.asset_id,
                catalog.info.launcher_id,
                catalog_constants.relative_block_height,
                catalog_constants.precommit_payout_puzzle_hash,
                value,
                reg_amount,
            )?;

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                Conditions::new()
                    .create_coin(precommit_coin.inner_puzzle_hash, reg_amount, vec![])
                    .create_coin(minter_puzzle_hash, payment_cat_amount - reg_amount, vec![]),
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

            let spends = ctx.take();
            print_spend_bundle_to_file(spends.clone(), Signature::default(), "sb.debug");
            sim.spend_coins(spends, &[user_sk.clone(), minter_sk.clone()])?;

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

            let price_update = if i % 2 == 1 {
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
                        registration_asset_id_hash: payment_cat.asset_id.tree_hash().into(),
                        registration_price: new_price,
                    },
                    catalog.coin.puzzle_hash,
                )?;

                price_singleton_coin = new_price_singleton_coin;
                price_singleton_proof = new_price_singleton_proof;

                let update_action =
                    CatalogRegistryAction::UpdatePrice(delegated_state_action_solution);

                Some(update_action)
            } else {
                None
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
                price_update,
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

        Ok(())
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
                vec![],
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
            Conditions::new().create_coin(minter_puzzle_hash, payment_cat_amount, vec![]),
        )?;
        minter_p2.spend(ctx, minter_coin, issue_cat)?;

        payment_cat = payment_cat.wrapped_child(minter_puzzle_hash, payment_cat_amount);
        sim.spend_coins(ctx.take(), &[minter_sk.clone()])?;

        // Launch price singleton
        let (
            price_singleton_launcher_id,
            mut price_singleton_coin,
            mut price_singleton_proof,
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

        let mut slots: Vec<Slot<XchandlesSlotValue>> = slots.into();
        for i in 0..7 {
            println!("registering handle {}", i); // todo: debug
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
            let reg_amount = if i % 2 == 1 {
                test_price_schedule[i / 2]
            } else {
                registry.info.state.registration_base_price
            };
            let reg_amount =
                reg_amount * XchandlesRegisterAction::get_price_factor(&handle).unwrap_or(1);

            let handle_launcher_id = did.info.launcher_id;
            let secret = Bytes32::default();

            let value =
                XchandlesPrecommitValue::new(secret, handle.clone(), handle_launcher_id, 100);

            let precommit_coin = PrecommitCoin::new(
                ctx,
                payment_cat.coin.coin_id(),
                payment_cat.child_lineage_proof(),
                payment_cat.asset_id,
                registry.info.launcher_id,
                xchandles_constants.relative_block_height,
                xchandles_constants.precommit_payout_puzzle_hash,
                value,
                reg_amount,
            )?;

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                Conditions::new()
                    .create_coin(precommit_coin.inner_puzzle_hash, reg_amount, vec![])
                    .create_coin(minter_puzzle_hash, payment_cat_amount - reg_amount, vec![]),
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

            println!("before spend 0 {}", i); // todo: debug
            let spends = ctx.take();
            print_spend_bundle_to_file(spends.clone(), Signature::default(), "sb.debug");
            sim.spend_coins(spends, &[user_sk.clone(), minter_sk.clone()])?;
            println!("after spend 0 {}", i); // todo: debug

            // call the 'register' action on CNS
            slots.sort_unstable_by(|a, b| a.info.value.unwrap().cmp(&b.info.value.unwrap()));

            let slot_value_to_insert = XchandlesSlotValue::new(
                handle_hash,
                Bytes32::default(),
                Bytes32::default(),
                0,
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

            let price_update = if i % 2 == 1 {
                let new_price = test_price_schedule[i / 2];
                assert_ne!(new_price, registry.info.state.registration_base_price);

                let (
                    new_price_singleton_coin,
                    new_price_singleton_proof,
                    delegated_state_action_solution,
                ) = spend_price_singleton(
                    ctx,
                    price_singleton_coin,
                    price_singleton_proof,
                    price_singleton_puzzle,
                    XchandlesRegistryState {
                        registration_asset_id_hash: payment_cat.asset_id.tree_hash().into(),
                        registration_base_price: new_price,
                    },
                    registry.coin.puzzle_hash,
                )?;

                price_singleton_coin = new_price_singleton_coin;
                price_singleton_proof = new_price_singleton_proof;

                let update_action =
                    XchandlesRegistryAction::UpdatePrice(delegated_state_action_solution);

                Some(update_action)
            } else {
                None
            };

            let (secure_cond, new_registry, new_slots) = registry.register_handle(
                ctx,
                left_slot,
                right_slot,
                precommit_coin,
                price_update,
            )?;

            let funds_puzzle = clvm_quote!(secure_cond.clone()).to_clvm(&mut ctx.allocator)?;
            let funds_coin = sim.new_coin(ctx.tree_hash(funds_puzzle).into(), 1);

            let funds_program = ctx.serialize(&funds_puzzle)?;
            let solution_program = ctx.serialize(&NodePtr::NIL)?;
            ctx.insert(CoinSpend::new(funds_coin, funds_program, solution_program));

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
            let pay_for_extension: u64 = extension_years
                * registry.info.state.registration_base_price
                * XchandlesRegisterAction::get_price_factor(&handle).unwrap_or(1);

            let (notarized_payment, extend_conds, new_registry, new_slots) =
                registry.extend(ctx, handle, extension_slot, pay_for_extension)?;

            let payment_cat_inner_spend = minter_p2.spend_with_conditions(
                ctx,
                extend_conds
                    .create_coin(
                        SETTLEMENT_PAYMENTS_PUZZLE_HASH.into(),
                        pay_for_extension,
                        vec![],
                    )
                    .create_coin(
                        minter_puzzle_hash,
                        payment_cat_amount - pay_for_extension,
                        vec![],
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
            let new_launcher_id = Bytes32::new([4 + i as u8; 32]);
            let update_slot = new_slots[0];

            let (update_conds, new_registry, new_slots) = registry.update(
                ctx,
                update_slot,
                new_launcher_id,
                did.info.inner_puzzle_hash().into(),
            )?;

            let _new_did = did.update(ctx, &user_puzzle, update_conds)?;

            sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

            slots.retain(|s| *s != update_slot);
            slots.extend(new_slots.clone());

            registry = new_registry;
        }

        assert_eq!(
            registry.info.state.registration_base_price,
            test_price_schedule[2], // 1, 3, 5 updated the price
        );

        // expire one of the slots
        // simulator doesn't have concept of time I suppose
        let handle_to_expire = "aa0".to_string();
        let handle_hash: Bytes32 = handle_to_expire.tree_hash().into();
        let initial_slot = slots
            .iter()
            .find(|s| s.info.value.unwrap().handle_hash == handle_hash)
            .unwrap();
        let left_slot = slots
            .iter()
            .find(|s| {
                s.info.value.unwrap().handle_hash
                    == initial_slot.info.value.unwrap().neighbors.left_value
            })
            .unwrap();
        let right_slot = slots
            .iter()
            .find(|s| {
                s.info.value.unwrap().handle_hash
                    == initial_slot.info.value.unwrap().neighbors.right_value
            })
            .unwrap();

        let (_, _, _) = registry.expire_handle(ctx, *initial_slot, *left_slot, *right_slot)?;

        sim.spend_coins(ctx.take(), &[user_sk.clone()])?;

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
}

use bip39::Mnemonic;
use chia::{
    bls::{sign, SecretKey, Signature},
    consensus::consensus_constants::ConsensusConstants,
    protocol::{Bytes32, Coin, CoinSpend, SpendBundle},
    puzzles::{
        offer::{
            NotarizedPayment, Payment, SettlementPaymentsSolution, SETTLEMENT_PAYMENTS_PUZZLE_HASH,
        },
        singleton::SingletonArgs,
        standard::{StandardArgs, StandardSolution},
        EveProof, LineageProof, Proof,
    },
};
use chia_wallet_sdk::{
    AggSig, AggSigKind, Condition, Conditions, DriverError, Launcher, Layer, Offer,
    RequiredSignature, Spend, SpendContext, StandardLayer,
};
use clvm_traits::{clvm_quote, FromClvm, ToClvm};
use clvmr::NodePtr;

use crate::{
    AddCat, Catalog, CatalogConstants, CatalogInfo, CatalogPreroller, CatalogPrerollerInfo,
    CatalogState, PriceSchedule, PriceScheduler, PriceSchedulerInfo,
};

pub struct SecureOneSidedOffer {
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
) -> Result<SecureOneSidedOffer, DriverError> {
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

    Ok(SecureOneSidedOffer {
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
    sk: SecretKey,
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

    let output = ctx.run(puzzle_reveal_ptr, solution_ptr)?;
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
        &security_coin,
        AggSig::new(
            AggSigKind::Me,
            agg_sig_me.public_key,
            agg_sig_me.message.clone(),
        ),
        consensus_constants,
    );

    Ok(sign(&sk, required_signature.final_message()))
}

pub fn launch_catalog(
    offer: Offer,
    price_schedule: PriceSchedule,
    initial_registration_price: u64,
    cats_to_launch: Vec<AddCat>,
    catalog_constants: CatalogConstants,
    consensus_constants: &ConsensusConstants,
) -> Result<(SpendBundle, PriceScheduler, Catalog), DriverError> {
    let ctx = &mut SpendContext::new();

    let offer = parse_one_sided_offer(ctx, offer)?;
    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = Conditions::new();

    // Create preroll coin launcher
    let preroll_launcher = Launcher::new(security_coin_id, 0);
    let preroll_launcher_coin = preroll_launcher.coin();
    let catalog_launcher_id = preroll_launcher_coin.coin_id();

    // Launch price scheduler
    let price_scheduler_launcher = Launcher::new(security_coin_id, 2);
    let price_scheduler_launcher_coin = price_scheduler_launcher.coin();
    let price_scheduler_launcher_id = price_scheduler_launcher_coin.coin_id();
    security_coin_conditions = security_coin_conditions.create_coin(
        price_scheduler_launcher_coin.puzzle_hash,
        price_scheduler_launcher_coin.amount,
        vec![price_scheduler_launcher_id.into()],
    );

    let price_scheduler_0th_gen_info = PriceSchedulerInfo::new(
        price_scheduler_launcher_id,
        price_schedule.clone(),
        0,
        catalog_launcher_id,
    );

    let schedule_ptr = price_schedule.to_clvm(&mut ctx.allocator)?;
    let (conds, price_scheduler_0th_gen_coin) = price_scheduler_launcher.spend(
        ctx,
        price_scheduler_0th_gen_info.inner_puzzle_hash().into(),
        schedule_ptr,
    )?;

    // this creates the launcher & secures the spend
    security_coin_conditions = security_coin_conditions.extend(conds);

    let price_scheduler = PriceScheduler::new(
        price_scheduler_0th_gen_coin,
        Proof::Eve(EveProof {
            parent_parent_coin_info: price_scheduler_launcher_coin.parent_coin_info,
            parent_amount: price_scheduler_launcher_coin.amount,
        }),
        price_scheduler_0th_gen_info,
    );

    // Spend preroll coin launcher
    let target_catalog_info = CatalogInfo::new(
        catalog_launcher_id,
        CatalogState {
            registration_price: initial_registration_price,
        },
        catalog_constants,
    );
    let target_catalog_inner_puzzle_hash = target_catalog_info.clone().inner_puzzle_hash();
    let preroll_info = CatalogPrerollerInfo::new(
        catalog_launcher_id,
        cats_to_launch,
        target_catalog_inner_puzzle_hash.into(),
    );

    let preroll_coin_inner_ph = preroll_info
        .clone()
        .inner_puzzle_hash(ctx, Bytes32::default())?;
    let (conds, preroller_coin) = preroll_launcher.spend(ctx, preroll_coin_inner_ph.into(), ())?;

    // this creates the launcher & secures the spend
    security_coin_conditions = security_coin_conditions.extend(conds);

    let preroller = CatalogPreroller::new(
        preroller_coin,
        Proof::Eve(EveProof {
            parent_parent_coin_info: preroll_launcher_coin.parent_coin_info,
            parent_amount: preroll_launcher_coin.amount,
        }),
        preroll_info,
    );

    // Spend preroll coin until the Catalog is created
    let catalog_coin = Coin::new(
        preroller.coin.coin_id(),
        SingletonArgs::curry_tree_hash(catalog_launcher_id, target_catalog_inner_puzzle_hash)
            .into(),
        1,
    );
    let catalog = Catalog::new(
        catalog_coin,
        Proof::Lineage(LineageProof {
            parent_parent_coin_info: preroller.coin.parent_coin_info,
            parent_inner_puzzle_hash: preroll_coin_inner_ph.into(),
            parent_amount: 1,
        }),
        target_catalog_info,
    );

    preroller.spend(ctx)?;

    // Secure everything we've done with the preroll coin
    security_coin_conditions =
        security_coin_conditions.assert_concurrent_spend(catalog.coin.coin_id());

    // Spend security coin
    let security_coin_sig = spend_security_coin(
        ctx,
        offer.security_coin,
        security_coin_conditions,
        offer.security_coin_sk,
        consensus_constants,
    )?;

    // Finally, return the data
    Ok((
        SpendBundle::new(ctx.take(), offer.aggregated_signature + &security_coin_sig),
        price_scheduler,
        catalog,
    ))
}

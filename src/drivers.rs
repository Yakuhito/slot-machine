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
    CatalogSlotValue, CatalogState, PriceSchedule, PriceScheduler, PriceSchedulerInfo, Slot,
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
        consensus_constants,
    );

    Ok(sign(sk, required_signature.final_message()))
}

#[allow(clippy::type_complexity)]
pub fn launch_catalog(
    ctx: &mut SpendContext,
    offer: Offer,
    price_schedule: PriceSchedule,
    initial_registration_price: u64,
    cats_to_launch: Vec<AddCat>,
    catalog_constants: CatalogConstants,
    consensus_constants: &ConsensusConstants,
) -> Result<
    (
        Signature,
        SecretKey,
        PriceScheduler,
        Catalog,
        Vec<Slot<CatalogSlotValue>>,
    ),
    DriverError,
> {
    let offer = parse_one_sided_offer(ctx, offer)?;
    offer.coin_spends.into_iter().for_each(|cs| ctx.insert(cs));

    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = Conditions::new();

    // Create preroll coin launcher
    let preroll_launcher = Launcher::new(security_coin_id, 1);
    let preroll_launcher_coin = preroll_launcher.coin();
    let catalog_launcher_id = preroll_launcher_coin.coin_id();

    // Launch price scheduler
    let price_scheduler_launcher = Launcher::new(security_coin_id, 2);
    let price_scheduler_launcher_coin = price_scheduler_launcher.coin();
    let price_scheduler_launcher_id = price_scheduler_launcher_coin.coin_id();

    let price_scheduler_0th_gen_info = PriceSchedulerInfo::new(
        price_scheduler_launcher_id,
        price_schedule.clone(),
        0,
        catalog_launcher_id,
    );

    let schedule_ptr = price_schedule.to_clvm(&mut ctx.allocator)?;
    let (conds, price_scheduler_0th_gen_coin) =
        price_scheduler_launcher.with_singleton_amount(1).spend(
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
    let royalty_puzzle_hash = catalog_constants.royalty_address;
    let trade_price_percentage = catalog_constants.trade_price_percentage;

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
        royalty_puzzle_hash.tree_hash().into(),
        trade_price_percentage,
    );

    let preroll_coin_inner_ph = preroll_info.clone().inner_puzzle_hash(ctx)?;
    let (conds, preroller_coin) =
        preroll_launcher
            .with_singleton_amount(1)
            .spend(ctx, preroll_coin_inner_ph.into(), ())?;

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

    let slots = preroller.spend(ctx, royalty_puzzle_hash)?;

    // Secure everything we've done with the preroll coin
    security_coin_conditions =
        security_coin_conditions.assert_concurrent_spend(catalog.coin.parent_coin_info);

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
        price_scheduler,
        catalog,
        slots,
    ))
}

#[cfg(test)]
mod tests {
    use chia::{
        clvm_utils::CurriedProgram, protocol::SpendBundle, puzzles::cat::GenesisByCoinIdTailArgs,
    };
    use chia_wallet_sdk::{test_secret_keys, Simulator, SpendWithConditions, TESTNET11_CONSTANTS};
    use hex_literal::hex;

    use crate::{
        print_spend_bundle_to_file, AddCatInfo, CatNftMetadata, CatalogAction,
        CatalogActionSolution, CatalogPrecommitValue, CatalogRegisterAction,
        CatalogRegisterActionSolution, PrecommitCoin,
    };

    use super::*;

    #[test]
    fn test_catalog() -> anyhow::Result<()> {
        let ctx = &mut SpendContext::new();
        let mut sim = Simulator::new();

        // setup config

        let initial_registration_price = 2000;
        let test_price_schedule = vec![(1, 1000), (2, 500), (3, 250)];

        let catalog_constants = CatalogConstants {
            royalty_address: Bytes32::from([7; 32]),
            trade_price_percentage: 100,
            precommit_payout_puzzle_hash: Bytes32::from([8; 32]),
            relative_block_height: 1,
            price_singleton_launcher_id: Bytes32::from(hex!(
                "0000000000000000000000000000000000000000000000000000000000000000"
            )),
        };

        let premine_cat = AddCat {
            asset_id: Bytes32::from(hex!(
                "d82dd03f8a9ad2f84353cd953c4de6b21dbaaf7de3ba3f4ddd9abe31ecba80ad"
            )),
            info: Some(AddCatInfo {
                asset_id_left: Bytes32::from(hex!(
                    "8000000000000000000000000000000000000000000000000000000000000000"
                )),
                asset_id_right: Bytes32::from(hex!(
                    "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                )),
                owner_puzzle_hash: Bytes32::from([1; 32]),
                metadata: CatNftMetadata {
                    code: "TDBX".to_string(),
                    name: "Testnet dexie bucks".to_string(),
                    description: "    Testnet version of dexie bucks".to_string(),
                    image_uris: vec!["https://icons-testnet.dexie.space/d82dd03f8a9ad2f84353cd953c4de6b21dbaaf7de3ba3f4ddd9abe31ecba80ad.webp".to_string()],
                    image_hash: Bytes32::from(
                        hex!("c84607c0e4cb4a878cc34ba913c90504ed0aac0f4484c2078529b9e42387da99")
                    ),
                    metadata_uris: vec!["https://icons-testnet.dexie.space/test.json".to_string()],
                    metadata_hash: Bytes32::from([2; 32]),
                },
            }),
        };
        let cats_to_launch = vec![premine_cat];

        // Create source offer
        let [launcher_sk, user_sk]: [SecretKey; 2] = test_secret_keys(2)?.try_into().unwrap();

        let launcher_pk = launcher_sk.public_key();
        let launcher_puzzle_hash = StandardArgs::curry_tree_hash(launcher_pk).into();

        let user_pk = user_sk.public_key();
        let user_puzzle_hash = StandardArgs::curry_tree_hash(user_pk).into();

        let offer_amount = 2 + cats_to_launch.len() as u64;
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

        // Launch catalog & price singleton
        let (_, security_sk, _price_scheduler, catalog, slots) = launch_catalog(
            ctx,
            offer,
            test_price_schedule,
            initial_registration_price,
            cats_to_launch,
            catalog_constants.clone(),
            &TESTNET11_CONSTANTS,
        )?;

        sim.spend_coins(ctx.take(), &[launcher_sk, security_sk])?;

        // Register CAT

        // create precommit coin
        let user_coin = sim.new_coin(user_puzzle_hash, catalog.info.state.registration_price);
        let tail = CurriedProgram {
            program: ctx.genesis_by_coin_id_tail_puzzle()?,
            args: GenesisByCoinIdTailArgs::new(user_coin.coin_id()),
        }
        .to_clvm(&mut ctx.allocator)?; // pretty much a random TAIL - we're not actually launching it
        let tail_hash = ctx.tree_hash(tail);

        let value = CatalogPrecommitValue {
            initial_inner_puzzle_hash: Bytes32::new([10; 32]),
            tail_reveal: tail,
        };
        let precommit_coin = PrecommitCoin::new(
            ctx,
            user_coin.coin_id(),
            catalog.info.launcher_id,
            catalog_constants.relative_block_height,
            catalog_constants.precommit_payout_puzzle_hash,
            value,
            catalog.info.state.registration_price,
        )?;

        StandardLayer::new(user_pk).spend(
            ctx,
            user_coin,
            Conditions::new().create_coin(
                precommit_coin.coin.puzzle_hash,
                precommit_coin.coin.amount,
                vec![],
            ),
        )?;

        // call the 'register' action on CATalog
        let mut sorted_slot_vals = slots
            .clone()
            .into_iter()
            .map(|s| s.value.unwrap())
            .collect::<Vec<_>>();
        sorted_slot_vals.sort_unstable();

        let slot_value_to_insert =
            CatalogSlotValue::new(tail_hash.into(), Bytes32::default(), Bytes32::default());

        let left_slot_value = sorted_slot_vals
            .iter()
            .rev()
            .find(|&&x| x < slot_value_to_insert)
            .unwrap();
        let left_slot = slots.iter().find(|s| s.value.unwrap() == *left_slot_value);

        let right_slot_value = sorted_slot_vals
            .iter()
            .find(|&&x| x > slot_value_to_insert)
            .unwrap();
        let right_slot = slots.iter().find(|s| s.value.unwrap() == *right_slot_value);

        let register_action = CatalogAction::Register(CatalogRegisterAction {
            launcher_id: catalog.info.launcher_id,
            royalty_puzzle_hash_hash: catalog_constants.royalty_address.tree_hash().into(),
            trade_price_percentage: catalog_constants.trade_price_percentage,
            precommit_payout_puzzle_hash: catalog_constants.precommit_payout_puzzle_hash,
            relative_block_height: catalog_constants.relative_block_height,
        });
        let register_solution = CatalogActionSolution::Register(CatalogRegisterActionSolution {
            tail_hash: tail_hash.into(),
            initial_nft_owner_ph: value.initial_inner_puzzle_hash,
            left_tail_hash: left_slot_value.asset_id,
            left_left_tail_hash: left_slot_value.neighbors.left_asset_id,
            right_tail_hash: right_slot_value.asset_id,
            right_right_tail_hash: right_slot_value.neighbors.right_asset_id,
            my_id: catalog.coin.coin_id(),
        });

        catalog.spend(ctx, vec![register_action], vec![register_solution])?;

        let spends = ctx.take();
        print_spend_bundle_to_file(spends.clone(), Signature::default(), "sb.debug");
        sim.spend_coins(spends, &[user_sk])?;

        Ok(())
    }
}

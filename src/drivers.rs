use bip39::Mnemonic;
use chia::{
    bls::{SecretKey, Signature},
    protocol::{Bytes32, Coin, CoinSpend, SpendBundle},
    puzzles::{
        offer::{
            NotarizedPayment, Payment, SettlementPaymentsSolution, SETTLEMENT_PAYMENTS_PUZZLE_HASH,
        },
        standard::StandardArgs,
    },
};
use chia_wallet_sdk::{Condition, Conditions, DriverError, Launcher, Offer, SpendContext};
use clvm_traits::{FromClvm, ToClvm};
use clvmr::NodePtr;

use crate::PriceSchedule;

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

pub fn launch_catalog(offer: Offer, schedule: PriceSchedule) -> Result<SpendBundle, DriverError> {
    let ctx = &mut SpendContext::new();

    let offer = parse_one_sided_offer(ctx, offer)?;
    let security_coin_id = offer.security_coin.coin_id();

    let mut security_coin_conditions = Conditions::new();

    // Launch preroll coin
    let preroll_launcher = Launcher::new(security_coin_id, 0);
    let preroll_launcher_coin = preroll_launcher.coin();
    let preroll_launcher_id = preroll_launcher_coin.coin_id();
    security_coin_conditions = security_coin_conditions.create_coin(
        preroll_launcher_coin.puzzle_hash,
        preroll_launcher_coin.amount,
        vec![preroll_launcher_id.into()],
    );

    // Launch price oracle
    let price_oracle_launcher = Launcher::new(security_coin_id, 2);
    let price_oracle_launcher_coin = price_oracle_launcher.coin();
    let price_oracle_launcher_id = price_oracle_launcher_coin.coin_id();
    security_coin_conditions = security_coin_conditions.create_coin(
        price_oracle_launcher_coin.puzzle_hash,
        price_oracle_launcher_coin.amount,
        vec![price_oracle_launcher_id.into()],
    );

    // let price_oracle_0th_gen_inner_puzzle_hash = PriceScheduler::new(coin, proof, launcher_id, price_schedule, generation, other_singleton_launcher_id)
    // price_oracle_launcher.spend(
    //     ctx,
    //     singleton_inner_puzzle_hash,
    //     schedule.to_clvm(&mut ctx.allocator)?,
    // );

    // Spend preroll coin until the Catalog is created

    // Secure everything we've done with the preroll coin

    // Spend security coin

    // Finally, return the spend bundle

    todo!()
    // overview:
    //  - launch preroll coin, do not secure via announcement (see below)
    //  - launch price oracle and secure announcement (amount 2 so it doesn't conflict w/ catalog launch)
    //  - spend preroll coin until the Catalog is created
    //  - assert concurrent spend with the last spent preroll coin to secure the whole thing
    //  - spend security coin
}

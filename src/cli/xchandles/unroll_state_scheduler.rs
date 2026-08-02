use chia_protocol::SpendBundle;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, DelegatedStateAction, Offer,
        SpendContext, XchandlesExpirePricingPuzzle, XchandlesRegistryState,
    },
    types::{
        puzzles::{DefaultCatMakerArgs, XchandlesFactorPricingPuzzleArgs},
        Conditions, Mod, MAINNET_CONSTANTS, TESTNET11_CONSTANTS,
    },
};
use clvm_utils::ToTreeHash;

use crate::{
    assets_xch_only, confirm_pushed_transaction, get_coinset_client, get_last_onchain_timestamp,
    hex_string_to_bytes32, load_xchandles_state_schedule_csv, no_assets, parse_amount,
    quick_sync_xchandles, sync_multisig_singleton, sync_xchandles, yes_no_prompt, CliError, Db,
    MultisigSingleton, SageClient,
};

/// True when the latest confirmed transaction-block timestamp has reached activation.
fn scheduler_activation_reached(latest_onchain_timestamp: u64, required_timestamp: u64) -> bool {
    latest_onchain_timestamp >= required_timestamp
}

pub async fn xchandles_unroll_state_scheduler(
    launcher_id_str: String,
    testnet11: bool,
    local: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    let cli = get_coinset_client(testnet11);
    let mut db = Db::new(false).await?;
    let mut ctx = SpendContext::new();

    let mut registry = if local {
        sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?
    } else {
        quick_sync_xchandles(&cli, &mut db, &mut ctx, launcher_id).await?
    };

    let (MultisigSingleton::StateScheduler(state_scheduler), _) =
        sync_multisig_singleton::<XchandlesRegistryState>(
            &cli,
            &mut ctx,
            registry.info.constants.price_singleton_launcher_id,
            None,
        )
        .await?
    else {
        return Err(CliError::Custom(
            "Price singleton is not (or no longer) a state scheduler".to_string(),
        ));
    };

    let sage = SageClient::new()?;
    let fee = parse_amount(&fee_str, false)?;

    let (required_timestamp, new_state) =
        state_scheduler.info.state_schedule[state_scheduler.info.generation];

    let latest_onchain_timestamp = get_last_onchain_timestamp(&cli).await?;
    if !scheduler_activation_reached(latest_onchain_timestamp, required_timestamp) {
        return Err(CliError::Custom(format!(
            "Latest confirmed transaction-block timestamp is {}, but required timestamp for new state is {}",
            latest_onchain_timestamp, required_timestamp
        )));
    }

    println!(
        "Next state sets a pricing puzzle hash of {} and an expired handle pricing puzzle hash of {} with CAT maker puzzle hash={}",
        hex::encode(new_state.pricing_puzzle_hash),
        hex::encode(new_state.expired_handle_pricing_puzzle_hash),
        hex::encode(new_state.cat_maker_puzzle_hash)
    );

    let filename = if testnet11 {
        "xchandles_price_schedule_testnet11.csv"
    } else {
        "xchandles_price_schedule_mainnet.csv"
    };
    let schedule = load_xchandles_state_schedule_csv(filename)?;
    let mut found = false;
    for record in schedule.iter() {
        let cmph = DefaultCatMakerArgs::new(record.asset_id.tree_hash().into()).curry_tree_hash();
        let pph = XchandlesFactorPricingPuzzleArgs {
            base_price: record.registration_price,
            registration_period: record.registration_period,
        }
        .curry_tree_hash();
        let eph = XchandlesExpirePricingPuzzle::curry_tree_hash(
            record.registration_price,
            record.registration_period,
        );
        if cmph == new_state.cat_maker_puzzle_hash.into()
            && pph == new_state.pricing_puzzle_hash.into()
            && eph == new_state.expired_handle_pricing_puzzle_hash.into()
        {
            println!(
                "These hashes correspond to a base price of {} mojos of the CAT with asset_id={} (activation timestamp {})",
                record.registration_price,
                hex::encode(record.asset_id),
                record.timestamp
            );
            found = true;
        }
    }
    if !found {
        println!("Could *NOT* figure out what those hashes translate to.");
        println!("PROCEED WITH CAUTION.\n\n\n")
    }

    println!("An offer will be generated offering:");
    println!(" - 1 mojo");
    println!(" - {} XCH ({} mojos) as fee", fee_str, fee);
    yes_no_prompt("The state scheduler and the XCHandles registry have been synced. This is the last check - do you wish to continue?")?;

    // spend state scheduler & XCHandles registry

    // no need to include security conditions as we assert the state scheduler is spent
    // which means the right message is consumed
    let (_action_secure_conds, registry_action_spend) = registry
        .new_action::<DelegatedStateAction>()
        .spend::<XchandlesRegistryState>(
            &mut ctx,
            registry.coin,
            new_state,
            state_scheduler.info.inner_puzzle_hash().into(),
        )?;
    registry.insert_action_spend(&mut ctx, registry_action_spend)?;

    let registry_inner_ph = registry.info.inner_puzzle_hash();
    let (_new_registry, pending_sig) = registry.finish_spend(&mut ctx)?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let security_coin_conditions = Conditions::new()
        .assert_concurrent_spend(state_scheduler.coin.coin_id())
        .reserve_fee(1);

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        security_coin_conditions,
        &security_coin_sk,
        if testnet11 {
            &TESTNET11_CONSTANTS
        } else {
            &MAINNET_CONSTANTS
        },
    )?;

    state_scheduler.spend(&mut ctx, registry_inner_ph.into())?;

    let sb = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &pending_sig,
    ));

    println!("Submitting transaction...");
    let resp = cli.push_tx(sb).await?;

    if confirm_pushed_transaction(&cli, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_uses_onchain_timestamp_not_height() {
        assert!(!scheduler_activation_reached(0, 1));
        assert!(scheduler_activation_reached(1, 1));
        assert!(scheduler_activation_reached(2, 1));
        assert!(!scheduler_activation_reached(2, 3));
    }
}

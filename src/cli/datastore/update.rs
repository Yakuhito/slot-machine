use chia_protocol::SpendBundle;
use chia_puzzle_types::standard::StandardArgs;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, DataStore, DataStoreMetadata,
        Offer, SpendContext, SpendWithConditions, StandardLayer,
    },
    types::Conditions,
};

use crate::{
    assets_xch_only, build_root_hash, confirm_pushed_transaction, get_coinset_client,
    get_constants, hex_string_to_pubkey, hex_string_to_signature, load_and_dedupe_csv, no_assets,
    parse_amount, sync_datastore, validate_update_csvs, yes_no_prompt, CliError, SageClient,
};

use super::oracle_delegated_puzzles;

pub async fn datastore_update(
    launcher_id_str: String,
    old_csv_path: String,
    new_csv_path: String,
    label: Option<String>,
    description: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = crate::hex_string_to_bytes32(&launcher_id_str)?;
    let old_records = load_and_dedupe_csv(&old_csv_path)?;
    let new_records = load_and_dedupe_csv(&new_csv_path)?;
    validate_update_csvs(&old_records, &new_records)?;

    let fee = parse_amount(&fee_str, false)?;

    println!("Syncing datastore...");
    let client = get_coinset_client(testnet11);
    let mut ctx = SpendContext::new();
    let datastore =
        sync_datastore(&client, &mut ctx, launcher_id, &oracle_delegated_puzzles()).await?;

    let old_root_hash = build_root_hash(&old_records)?;
    if old_root_hash != datastore.info.metadata.root_hash {
        return Err(CliError::Custom(format!(
            "Old CSV merkle root ({}) does not match on-chain root_hash ({})",
            hex::encode(old_root_hash),
            hex::encode(datastore.info.metadata.root_hash),
        )));
    }

    let sage = SageClient::new()?;
    let derivation_resp = sage.get_derivations(false, 0, 1).await?;
    let owner_pk = hex_string_to_pubkey(&derivation_resp.derivations[0].public_key)?;
    let owner_puzzle_hash = StandardArgs::curry_tree_hash(owner_pk);
    if datastore.info.owner_puzzle_hash != owner_puzzle_hash.into() {
        return Err(CliError::Custom(
            "Active wallet is not the datastore owner".to_string(),
        ));
    }

    let new_metadata = DataStoreMetadata {
        root_hash: build_root_hash(&new_records)?,
        label: label.or(datastore.info.metadata.label.clone()),
        description: description.or(datastore.info.metadata.description.clone()),
        bytes: None,
        size_proof: None,
    };

    println!("Updating datastore metadata:");
    println!("  root_hash: {}", hex::encode(new_metadata.root_hash));
    println!(
        "  label: {}",
        new_metadata.label.as_deref().unwrap_or("(none)")
    );
    println!(
        "  description: {}",
        new_metadata.description.as_deref().unwrap_or("(none)")
    );
    println!("A one-sided offer will be created. It will contain:");
    println!("  - 1 mojo");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;
    offer
        .spend_bundle()
        .coin_spends
        .iter()
        .for_each(|cs| ctx.insert(cs.clone()));

    let owner_layer = StandardLayer::new(owner_pk);
    let recreate = DataStore::<()>::owner_create_coin_condition(
        &mut ctx,
        datastore.info.launcher_id,
        owner_puzzle_hash.into(),
        oracle_delegated_puzzles(),
        false,
    )
    .map_err(CliError::Driver)?;
    let metadata_condition =
        DataStore::new_metadata_condition(&mut ctx, new_metadata).map_err(CliError::Driver)?;

    let inner_spend = owner_layer
        .spend_with_conditions(
            &mut ctx,
            Conditions::new().with(recreate).with(metadata_condition),
        )
        .map_err(CliError::Driver)?;
    let datastore_coin_id = datastore.coin.coin_id();
    let dl_spend = datastore.spend(&mut ctx, inner_spend)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        Conditions::new().assert_concurrent_spend(datastore_coin_id),
        &security_coin_sk,
        get_constants(testnet11),
    )
    .map_err(CliError::Driver)?;

    let wallet_sig = hex_string_to_signature(
        &sage
            .sign_coin_spends(vec![dl_spend.clone()], false, true)
            .await?
            .spend_bundle
            .aggregated_signature
            .replace("0x", ""),
    )?;

    ctx.insert(dl_spend);

    let spend_bundle = offer.take(SpendBundle::new(
        ctx.take(),
        security_coin_sig + &wallet_sig,
    ));

    println!("Submitting transaction...");
    let resp = client.push_tx(spend_bundle).await?;

    if confirm_pushed_transaction(&client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

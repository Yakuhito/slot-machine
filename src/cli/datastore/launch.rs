use chia_protocol::SpendBundle;
use chia_puzzle_types::standard::StandardArgs;
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{
        create_security_coin, decode_offer, spend_security_coin, DataStoreMetadata, Launcher,
        Offer, SpendContext,
    },
};

use crate::{
    assets_xch_only, build_root_hash, confirm_pushed_transaction, get_coinset_client,
    get_constants, hex_string_to_pubkey, load_and_dedupe_csv, no_assets, parse_amount,
    yes_no_prompt, CliError, SageClient,
};

use super::oracle_delegated_puzzles;

pub async fn datastore_launch(
    csv_path: String,
    label: Option<String>,
    description: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let records = load_and_dedupe_csv(&csv_path)?;
    let root_hash = build_root_hash(&records)?;
    let metadata = DataStoreMetadata {
        root_hash,
        label,
        description,
        bytes: None,
        size_proof: None,
    };

    let fee = parse_amount(&fee_str, false)?;

    println!("NFT whitelist entries: {}", records.len());
    println!("Metadata root hash: {}", hex::encode(root_hash));

    let sage = SageClient::new()?;
    let derivation_resp = sage.get_derivations(false, 0, 1).await?;
    let user_address = &derivation_resp.derivations[0].address;
    let owner_pk = hex_string_to_pubkey(&derivation_resp.derivations[0].public_key)?;
    let owner_puzzle_hash = StandardArgs::curry_tree_hash(owner_pk);
    println!(
        "Datastore owner will be the active wallet (address: {})",
        user_address
    );

    println!("A one-sided offer will be needed for launch. It will contain:");
    println!("  -1 mojo to create the datastore singleton");
    println!("  - {} XCH ({} mojos) reserved as fees", fee_str, fee);

    yes_no_prompt("Proceed?")?;

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;
    println!("Offer with id {} generated.", offer_resp.offer_id);

    let mut ctx = SpendContext::new();
    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;
    offer
        .spend_bundle()
        .coin_spends
        .iter()
        .for_each(|cs| ctx.insert(cs.clone()));

    let (launch_conditions, datastore) = Launcher::new(security_coin.coin_id(), 1)
        .mint_datastore(
            &mut ctx,
            metadata,
            owner_puzzle_hash,
            oracle_delegated_puzzles(),
        )
        .map_err(CliError::Driver)?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        launch_conditions,
        &security_coin_sk,
        get_constants(testnet11),
    )
    .map_err(CliError::Driver)?;

    println!(
        "Datastore launcher id (SAVE THIS): {}",
        hex::encode(datastore.info.launcher_id)
    );

    let spend_bundle = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(spend_bundle).await?;

    if confirm_pushed_transaction(&client, &resp, security_coin.coin_id(), true).await? {
        println!("Confirmed!");
    }

    Ok(())
}

use chia::{bls::PublicKey, protocol::SpendBundle};
use chia_wallet_sdk::{
    coinset::ChiaRpcClient,
    driver::{decode_offer, Launcher, Offer, SpendContext},
};

use crate::{
    assets_xch_only, create_security_coin, get_coinset_client, get_constants, no_assets,
    parse_amount, print_medieval_vault_configuration, spend_security_coin, wait_for_coin,
    yes_no_prompt, CliError, MedievalVaultHint, P2MOfNDelegateDirectArgs, SageClient,
};

pub async fn multisig_launch(
    pubkeys_str: String,
    m: usize,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    let mut pubkeys = Vec::new();
    for pubkey_str in pubkeys_str.split(',') {
        let pubkey = PublicKey::from_bytes(
            &hex::decode(pubkey_str.trim().replace("0x", ""))
                .map_err(CliError::ParseHex)?
                .try_into()
                .unwrap(),
        )
        .map_err(CliError::InvalidPublicKey)?;
        pubkeys.push(pubkey);
    }

    let fee = parse_amount(&fee_str, false)?;

    println!("You're about to create a new multisig with the following settings:");
    print_medieval_vault_configuration(m, &pubkeys)?;
    println!("  Testnet: {}", testnet11);

    println!("A one-sided offer offering 1 mojo and {} XCH ({} mojos) as fee will be generated and used to launch the multisig.", fee_str, fee);
    yes_no_prompt("Continue?")?;

    let sage = SageClient::new()?;
    let mut ctx = SpendContext::new();

    let offer_resp = sage
        .make_offer(no_assets(), assets_xch_only(1), fee, None, None, false)
        .await?;

    println!("Offer with id {} generated.", offer_resp.offer_id);

    let offer = Offer::from_spend_bundle(&mut ctx, &decode_offer(&offer_resp.offer)?)?;
    let (security_coin_sk, security_coin) =
        create_security_coin(&mut ctx, offer.offered_coins().xch[0])?;

    let launcher = Launcher::new(security_coin.coin_id(), 1);
    let launcher_coin = launcher.coin();
    let launch_hints = MedievalVaultHint {
        my_launcher_id: launcher_coin.coin_id(),
        m,
        public_key_list: pubkeys.clone(),
    };
    println!(
        "Multisig (medieval launch) launcher id (SAVE THIS): {}",
        hex::encode(launcher_coin.coin_id().to_bytes())
    );

    let (create_conditions, _vault_coin) = launcher.spend(
        &mut ctx,
        P2MOfNDelegateDirectArgs::curry_tree_hash(m, pubkeys.clone()).into(),
        launch_hints,
    )?;

    let security_coin_sig = spend_security_coin(
        &mut ctx,
        security_coin,
        create_conditions,
        &security_coin_sk,
        get_constants(testnet11),
    )?;

    let sb = offer.take(SpendBundle::new(ctx.take(), security_coin_sig));

    println!("Submitting transaction...");
    let client = get_coinset_client(testnet11);
    let resp = client.push_tx(sb).await?;

    println!("Transaction submitted; status='{}'", resp.status);

    wait_for_coin(&client, security_coin.coin_id(), true).await?;
    println!("Confirmed!");

    Ok(())
}

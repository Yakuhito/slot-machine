use chia::{bls::PublicKey, protocol::SpendBundle};
use chia_wallet_sdk::{ChiaRpcClient, Launcher, SpendContext, SpendWithConditions, StandardLayer};

use crate::{
    get_coinset_client, get_constants, get_xch_coin, parse_amount, partial_sign,
    print_medieval_vault_configuration, sign_standard_transaction, wait_for_coin, yes_no_prompt,
    CliError, MedievalVaultHint, P2MOfNDelegateDirectArgs, SageClient,
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

    let fee = parse_amount(fee_str.clone(), false)?;

    println!("You're about to create a new multisig with the following settings:");
    print_medieval_vault_configuration(m, &pubkeys)?;
    println!("  Fee: {} XCH ({} mojos)", fee_str, fee);
    println!("  Testnet: {}", testnet11);

    yes_no_prompt("Continue?")?;

    let client = SageClient::new()?;
    let mut ctx = SpendContext::new();

    let (sk, coin) = get_xch_coin(&client, &mut ctx, 1, fee, testnet11).await?;

    let launcher = Launcher::new(coin.coin_id(), 1);
    let launcher_coin = launcher.coin();
    let launch_hints = MedievalVaultHint {
        my_launcher_id: launcher_coin.coin_id(),
        m,
        public_key_list: pubkeys.clone(),
    };
    println!(
        "Vault launcher id: {}",
        hex::encode(launcher_coin.coin_id().to_bytes())
    );

    let (create_conditions, _vault_coin) = launcher.spend(
        &mut ctx,
        P2MOfNDelegateDirectArgs::curry_tree_hash(m, pubkeys.clone()).into(),
        launch_hints,
    )?;

    let coin_spend =
        StandardLayer::new(sk.public_key()).spend_with_conditions(&mut ctx, create_conditions)?;
    ctx.spend(coin, coin_spend)?;

    let coin_spends = ctx.take();
    let mut sig = partial_sign(&client, &coin_spends).await?;

    sig.aggregate(&sign_standard_transaction(
        &mut ctx,
        coin,
        coin_spend,
        &sk,
        get_constants(testnet11),
    )?);

    let spend_bundle = SpendBundle::new(coin_spends, sig);

    println!("Submitting spend bundle...");
    let cli = get_coinset_client(testnet11);
    let response = cli.push_tx(spend_bundle).await.map_err(CliError::Reqwest)?;
    if !response.success {
        eprintln!(
            "Failed to submit spend bundle: {}",
            response.error.unwrap_or("Unknown error".to_string())
        );
        return Err(CliError::Custom(
            "Failed to submit spend bundle".to_string(),
        ));
    }
    println!("Spend bundle successfully included in mempool :)");

    wait_for_coin(&cli, coin.coin_id(), true).await?;

    println!("Vault successfully created!");
    println!(
        "As a reminder, the vault launcher id is: {}",
        hex::encode(launcher_coin.coin_id().to_bytes())
    );

    Ok(())
}

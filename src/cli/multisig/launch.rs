use chia::bls::PublicKey;

use crate::{get_alias_map, parse_amount, yes_no_prompt, CliError, SageClient};

pub async fn multisig_launch(
    pubkeys_str: String,
    m: usize,
    testnet11: bool,
    fee_str: String,
    sage_ssl_path: String,
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

    let alias_map = get_alias_map()?;

    println!("You're about to create a new multisig with the following settings:");
    println!("  Public Key List:");
    for pubkey in pubkeys {
        println!(
            "    - {}",
            alias_map
                .get(&pubkey)
                .unwrap_or(&format!("0x{}", hex::encode(pubkey.to_bytes())))
        );
    }
    println!("  Signature Threshold: {}", m);
    println!("  Fee: {} XCH ({} mojos)", fee_str, fee);
    println!("  Testnet: {}", testnet11);

    yes_no_prompt("Continue?")?;

    let client = SageClient::new(sage_ssl_path)?;

    Ok(())
}

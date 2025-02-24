use chia::bls::PublicKey;

use crate::CliError;

pub async fn multisig_launch(
    pubkeys_str: String,
    m: usize,
    testnet11: bool,
    fee: String,
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

    Ok(())
}

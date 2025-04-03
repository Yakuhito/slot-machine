use chia::bls::{verify, Signature};

use crate::{hex_string_to_bytes, hex_string_to_bytes32, hex_string_to_pubkey, CliError};

pub async fn multisig_verify_signature(
    raw_message_str: String,
    pubkey_str: String,
    signature_str: String,
) -> Result<(), CliError> {
    let raw_message = hex_string_to_bytes32(&raw_message_str)?;
    let pubkey = hex_string_to_pubkey(&pubkey_str)?;

    let signature = hex_string_to_bytes(if signature_str.len() != 96 * 2 {
        signature_str.split("-").last().unwrap()
    } else {
        &signature_str
    })?;
    let signature = Signature::from_bytes(&signature.to_vec().try_into().unwrap())
        .map_err(CliError::InvalidPublicKey)?;

    if verify(&signature, &pubkey, raw_message) {
        println!("Signature is OK");
    } else {
        eprintln!("!!! INVALID SIGNATURE !!!");
        return Err(CliError::Custom("Signature is invalid".to_string()));
    }

    Ok(())
}

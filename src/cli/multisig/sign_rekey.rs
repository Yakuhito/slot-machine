use crate::CliError;

pub async fn multisig_sign_rekey(
    new_pubkeys_str: String,
    new_m: usize,
    my_pubkey_str: String,
    launcher_id: String,
    testnet11: bool,
    debug: bool,
) -> Result<(), CliError> {
    todo!("TODO")
}

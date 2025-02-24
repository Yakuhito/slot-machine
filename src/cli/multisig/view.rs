use crate::{get_coinset_client, hex_string_to_bytes32, CliError};

pub async fn multisig_view(launcher_id_str: String, testnet11: bool) -> Result<(), CliError> {
    let _launcher_id = hex_string_to_bytes32(&launcher_id_str)?;
    let _cli = get_coinset_client(testnet11);

    println!("Viewing vault...");
    todo!("Get vault records and parse spends")
}

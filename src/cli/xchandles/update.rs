use crate::CliError;

pub async fn xchandles_update(
    launcher_id_str: String,
    handle: String,
    new_owner_nft: Option<String>,
    new_resolved_nft: Option<String>,
    testnet11: bool,
    fee_str: String,
) -> Result<(), CliError> {
    Ok(())
}

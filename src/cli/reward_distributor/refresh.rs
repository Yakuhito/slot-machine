use chia_wallet_sdk::driver::{RewardDistributorType, SpendContext};

use crate::{
    get_coinset_client, hex_string_to_bytes32, sync_distributor, CliError, Db,
};

pub async fn reward_distributor_refresh(
    launcher_id_str: String,
    testnet11: bool,
    _fee_str: String,
) -> Result<(), CliError> {
    let launcher_id = hex_string_to_bytes32(&launcher_id_str)?;

    println!("Syncing reward distributor...");
    let client = get_coinset_client(testnet11);
    let db = Db::new(false).await?;
    let mut ctx = SpendContext::new();
    let distributor = sync_distributor(&client, &db, &mut ctx, launcher_id).await?;

    match distributor.info.constants.reward_distributor_type {
        RewardDistributorType::CuratedNft {
            refreshable: true, ..
        } => Err(CliError::Custom(
            "Refresh for curated NFT distributors is not yet implemented".to_string(),
        )),
        RewardDistributorType::CuratedNft {
            refreshable: false, ..
        } => Err(CliError::Custom(
            "This reward distributor is not refreshable".to_string(),
        )),
        _ => Err(CliError::Custom(
            "Refresh is only supported for refreshable curated NFT reward distributors".to_string(),
        )),
    }
}

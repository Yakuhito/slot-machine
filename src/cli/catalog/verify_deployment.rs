use chia_wallet_sdk::SpendContext;

use crate::{
    get_coinset_client, load_catalog_state_schedule_csv, print_medieval_vault_configuration,
    CatalogRegistryConstants, CatalogRegistryState, CliError, DefaultCatMakerArgs,
    MultisigSingleton,
};

use crate::sync_multisig_singleton;

pub async fn catalog_verify_deployment(testnet11: bool) -> Result<(), CliError> {
    let mut ctx = SpendContext::new();
    let cli = get_coinset_client(testnet11);
    let catalog_constants = CatalogRegistryConstants::get(testnet11);

    println!("Verifying CATalog deployment (testnet: {})...", testnet11);

    println!("Now let's analyze the price singleton.");
    let (MultisigSingleton::Vault(my_vault), Some(state_scheduler_info)) =
        sync_multisig_singleton::<CatalogRegistryState>(
            &cli,
            &mut ctx,
            catalog_constants.launcher_id,
            None,
        )
        .await?
    else {
        return Err(CliError::Custom(
            "Price singleton was not created correctly or is still in its state scheduler phase."
                .to_string(),
        ));
    };

    let price_schedule_csv_filename = if testnet11 {
        "catalog_price_schedule_testnet11.csv"
    } else {
        "catalog_price_schedule_mainnet.csv"
    };
    print!(
        "Checking executed price schedule against '{}' (TRUSTED SOURCE)... ",
        price_schedule_csv_filename
    );

    let price_schedule = load_catalog_state_schedule_csv(price_schedule_csv_filename)?;
    let mut price_schedule_ok = true;
    for (i, record) in price_schedule.iter().enumerate() {
        let (block, state) = state_scheduler_info.state_schedule[i];
        if record.block_height != block
            || record.registration_price != state.registration_price
            || record.asset_id != DefaultCatMakerArgs::curry_tree_hash(record.asset_id).into()
        {
            price_schedule_ok = false;
            break;
        }
    }

    if price_schedule_ok {
        println!("OK");
    } else {
        println!("FAILED");
        return Err(CliError::Custom(
            "Price schedule does not match the one defined in the csv.".to_string(),
        ));
    }

    println!("Current (latest unspent) vault info:");
    print_medieval_vault_configuration(my_vault.info.m, &my_vault.info.public_key_list)?;

    Ok(())
}

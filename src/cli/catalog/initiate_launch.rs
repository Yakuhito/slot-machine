use crate::cli::{
    csv::load_catalog_premine_csv,
    utils::{yes_no_prompt, CliError},
};

pub fn initiate_catalog_launch(csv_filename: &str) -> Result<(), CliError> {
    println!("Welcome to the CATalog launch setup, deployer.");

    yes_no_prompt(
        format!(
            "Premine data will be loaded from '{}' - do you confirm?",
            csv_filename
        )
        .as_str(),
    )?;

    println!("Loading premine data...");

    let data = load_catalog_premine_csv(csv_filename)?;

    Ok(())
}

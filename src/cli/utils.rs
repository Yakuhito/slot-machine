use std::io::{self, Write};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("csv: {0}")]
    Csv(#[from] csv::Error),

    #[error("that's not a clear 'yes'")]
    YesNoPromptRejected,
}

pub fn yes_no_prompt(prompt: &str) -> Result<(), CliError> {
    let mut input = String::new();
    print!("{} (yes/no): ", prompt);
    io::stdout().flush().map_err(CliError::Io)?;

    io::stdin().read_line(&mut input).map_err(CliError::Io)?;
    let input = input.trim().to_lowercase();

    if input != "yes" {
        return Err(CliError::YesNoPromptRejected);
    }

    Ok(())
}

use clap::{Parser, Subcommand};

use super::catalog::initiate_catalog_launch;

#[derive(Parser)]
#[command(
    name = "Slot Machine CLI",
    about = "A CLI for interacting with the first two dApps using the slot primitive, CATalog and CNS"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interact with CATalog
    Catalog {
        #[command(subcommand)]
        action: CatalogCliAction,
    },
    /// Interact with CNS
    Cns {
        #[command(subcommand)]
        action: CnsCliAction,
    },
}

#[derive(Subcommand)]
enum CatalogCliAction {
    /// Launches a new CATalog deployment
    InitiateLaunch,
    /// Unrolls an existing launch
    UnrollLaunch,
    /// Verifies the built-in deployment is valid
    VerifDeployment,
}

#[derive(Subcommand)]
enum CnsCliAction {
    /// Launches a new CNS deployment
    InitiateLaunch,
    /// Unrolls an existing launch
    UnrollLaunch,
    /// Verifies the built-in deployment is valid
    VerifDeployment,
}

pub fn run_cli() {
    let args = Cli::parse();

    let res = match args.command {
        Commands::Catalog { action } => match action {
            CatalogCliAction::InitiateLaunch => {
                initiate_catalog_launch("catalog_premine_testnet11.csv")
            }
            CatalogCliAction::UnrollLaunch => {
                todo!("not yet implemented");
            }
            CatalogCliAction::VerifDeployment => {
                todo!("not yet implemented");
            }
        },

        Commands::Cns { action } => match action {
            CnsCliAction::InitiateLaunch => {
                todo!("not yet implemented");
            }
            CnsCliAction::UnrollLaunch => {
                todo!("not yet implemented");
            }
            CnsCliAction::VerifDeployment => {
                todo!("not yet implemented");
            }
        },
    };

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
}

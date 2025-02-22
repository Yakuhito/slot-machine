use clap::{Parser, Subcommand};

use super::catalog_initiate_launch;

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
    /// Interact with XCHandles
    Xchandles {
        #[command(subcommand)]
        action: XchandlesCliAction,
    },
}

#[derive(Subcommand)]
enum CatalogCliAction {
    /// Launches a new CATalog deployment
    InitiateLaunch,
    /// Continues/finishes an existing launch
    ContinueLaunch,
    /// Verifies the built-in deployment is valid
    VerifyDeployment,
}

#[derive(Subcommand)]
enum XchandlesCliAction {
    /// Launches a new XCHandles deployment
    InitiateLaunch,
    /// Continues/finishes an existing launch
    ContinueLaunch,
    /// Verifies the built-in deployment is valid
    VerifyDeployment,
}

pub async fn run_cli() {
    let args = Cli::parse();

    let res = match args.command {
        Commands::Catalog { action } => match action {
            CatalogCliAction::InitiateLaunch => catalog_initiate_launch(true).await,
            CatalogCliAction::ContinueLaunch => {
                todo!("not yet implemented");
            }
            CatalogCliAction::VerifyDeployment => {
                todo!("not yet implemented");
            }
        },

        Commands::Xchandles { action } => match action {
            XchandlesCliAction::InitiateLaunch => {
                todo!("not yet implemented");
            }
            XchandlesCliAction::ContinueLaunch => {
                todo!("not yet implemented");
            }
            XchandlesCliAction::VerifyDeployment => {
                todo!("not yet implemented");
            }
        },
    };

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
}

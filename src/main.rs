use clap::{Parser, Subcommand};

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
        action: CatalogAction,
    },
    /// Interact with CNS
    Cns {
        #[command(subcommand)]
        action: CnsAction,
    },
}

#[derive(Subcommand)]
enum CatalogAction {
    /// Launches a new CATalog deployment
    InitiateLaunch {
        /// Offer for initiating launch
        #[arg(short, long, help = "Offer for initiating launch (2 mojos)")]
        offer: String,
    },
    /// Unrolls an existing launch
    UnrollLaunch,
    /// Verifies the built-in deployment is valid
    VerifDeployment,
}

#[derive(Subcommand)]
enum CnsAction {
    /// Launches a new CNS deployment
    InitiateLaunch,
    /// Unrolls an existing launch
    UnrollLaunch,
    /// Verifies the built-in deployment is valid
    VerifDeployment,
}

fn main() {
    let args = Cli::parse();

    match args.command {
        Commands::Catalog { action } => match action {
            CatalogAction::InitiateLaunch { offer } => {
                // Implement the logic here
                println!("Initiating CATalog launch with offer: {}", offer);
            }
            CatalogAction::UnrollLaunch => {
                todo!("not yet implemented");
            }
            CatalogAction::VerifDeployment => {
                todo!("not yet implemented");
            }
        },

        Commands::Cns { action } => match action {
            CnsAction::InitiateLaunch => {
                todo!("not yet implemented");
            }
            CnsAction::UnrollLaunch => {
                todo!("not yet implemented");
            }
            CnsAction::VerifDeployment => {
                todo!("not yet implemented");
            }
        },
    }
}

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "Slot Machine CLI",
    about = "A CLI for interacting with the first two slot PoCs, CATalog and CNS"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Catalog {
        #[command(subcommand)]
        action: CatalogAction,
    },
    Cns {
        #[command(subcommand)]
        action: CnsAction,
    },
}

#[derive(Subcommand)]
enum CatalogAction {
    InitiateLaunch,
    ContinueLaunch,
}

#[derive(Subcommand)]
enum CnsAction {
    InitiateLaunch,
    ContinueLaunch,
}

fn main() {
    let args = Cli::parse();

    match args.command {
        Commands::Catalog { action } => match action {
            CatalogAction::InitiateLaunch => todo!("to implement catalog intiate-launch"),
            CatalogAction::ContinueLaunch => todo!("to implement catalog continue-launch"),
        },

        Commands::Cns { action } => todo!("to implement :)"),
    }
}

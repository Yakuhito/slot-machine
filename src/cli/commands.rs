use clap::{Parser, Subcommand};

use super::{catalog_initiate_launch, multisig_launch, multisig_view};

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
    /// Interact with XCHandles
    Xchandles {
        #[command(subcommand)]
        action: XchandlesCliAction,
    },
    /// Interact with CATalog
    Catalog {
        #[command(subcommand)]
        action: CatalogCliAction,
    },
    /// Multisig (price singletons) operations
    Multisig {
        #[command(subcommand)]
        action: MultisigCliAction,
    },
}

#[derive(Subcommand)]
enum MultisigCliAction {
    /// Launches a new multisig
    Launch {
        /// Comma-separated list of pubkeys (no spaces)
        #[arg(long)]
        pubkeys: String,

        /// Threshold required for spends (m from m-of-n)
        #[arg(short)]
        m: usize,

        /// Use testnet11 (default: mainnet)
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use for the launch, in XCH (default: 0.0025 XCH)
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// View history of a vault
    View {
        /// Vault (singleton) launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11 (default: mainnet)
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
    /// Sign a rekey transaction for the vault
    SignRekey,
    /// Broadcast a rekey transaction for the vault
    BroadcastRekey,
    // Todo: Sign CATalog/XCHandles state updates; perform update
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
        Commands::Multisig { action } => match action {
            MultisigCliAction::Launch {
                pubkeys,
                m,
                testnet11,
                fee,
            } => multisig_launch(pubkeys, m, testnet11, fee).await,
            MultisigCliAction::View {
                launcher_id,
                testnet11,
            } => multisig_view(launcher_id, testnet11).await,
            MultisigCliAction::SignRekey => {
                todo!("not yet implemented");
            }
            MultisigCliAction::BroadcastRekey => {
                todo!("not yet implemented");
            }
        },
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

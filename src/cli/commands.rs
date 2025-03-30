use clap::{Parser, Subcommand};

use super::{
    catalog_continue_launch, catalog_initiate_launch, catalog_listen, catalog_register,
    catalog_unroll_state_scheduler, catalog_verify_deployment,
    multisig_broadcast_catalog_state_update, multisig_broadcast_rekey,
    multisig_sign_catalog_state_update, multisig_sign_rekey, multisig_verify_signature,
    multisig_view,
};

#[derive(Parser)]
#[command(
    name = "Slot Machine CLI",
    about = "A CLI for interacting with the first dApps that use the slot primitive: CATalog, CNS, and the DIG Reward Distributor"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[allow(clippy::large_enum_variant)]
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
    /// View history of a vault
    View {
        /// Vault (singleton) launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
    /// Sign a rekey transaction for the vault
    SignRekey {
        /// New pubkeys for the vault (comma-separated list of hex strings)
        #[arg(long)]
        new_pubkeys: String,

        /// New m (signature threshold) for the vault
        #[arg(long)]
        new_m: usize,

        /// Pubkey to sign with (hex string)
        #[arg(long)]
        my_pubkey: String,

        /// Vault (singleton) launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use debug signing method (pk prompt)
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Broadcast a rekey transaction for the vault
    BroadcastRekey {
        /// New pubkeys for the vault (comma-separated list of hex strings)
        #[arg(long)]
        new_pubkeys: String,

        /// New m (signature threshold) for the vault
        #[arg(long)]
        new_m: usize,

        /// Collected m signatures (comma-separated list)
        #[arg(long)]
        sigs: String,

        /// Vault (singleton) launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Verify a signature
    VerifySignature {
        /// Raw message (hex string - delegated puzzle hash)
        #[arg(long)]
        raw_message: String,

        /// Signature (hex string)
        #[arg(long)]
        signature: String,

        /// Public key of signer (hex string)
        #[arg(long)]
        pubkey: String,
    },
    /// Sign a CATalog state update transaction
    SignCatalogStateUpdate {
        /// New payment asset id
        #[arg(long)]
        new_payment_asset_id: String,

        /// New payment asset amount
        #[arg(long)]
        new_payment_asset_amount: String,

        /// Pubkey to sign with (hex string)
        #[arg(long)]
        my_pubkey: String,

        /// Vault (singleton) launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use debug signing method (pk prompt)
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Broadcast a CATalog state update transaction
    BroadcastCatalogStateUpdate {
        /// New payment asset id
        #[arg(long)]
        new_payment_asset_id: String,

        /// New payment asset amount
        #[arg(long)]
        new_payment_asset_amount: String,

        /// Collected m signatures (comma-separated list)
        #[arg(long)]
        sigs: String,

        /// Vault (singleton) launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum CatalogCliAction {
    /// Launches a new CATalog deployment
    InitiateLaunch {
        /// Comma-separated list of price singleton pubkeys (no spaces)
        #[arg(long)]
        pubkeys: String,

        /// Threshold required for price singleton spends (m from m-of-n)
        #[arg(short)]
        m: usize,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use for the launch, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Continues/finishes an existing launch
    ContinueLaunch {
        /// Payment asset id (payment CAT tail hash from launch initiation)
        #[arg(long)]
        payment_asset_id: String,

        /// How many CATs to deploy for this spend
        #[arg(long)]
        cats_per_spend: usize,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Unrolls the state scheduler
    UnrollStateScheduler {
        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Verifies the built-in deployment is valid
    VerifyDeployment {
        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
    /// Register a new CAT
    Register {
        /// TAIL reveal (hex CLVM)
        #[arg(long)]
        tail_reveal: String,

        /// Initial CAT ticker (e.g., "SBX")
        #[arg(long)]
        ticker: String,

        /// Initial CAT name (e.g., "Spacebucks")
        #[arg(long)]
        name: String,

        /// Initial CAT image URIs (comma-separated list of URIs)
        #[arg(long)]
        image_uris: String,

        /// Initial CAT image hash
        #[arg(long)]
        image_hash: String,

        /// Initial on-chain CAT description (e.g., "Galactic money for a galactic galaxy")
        #[arg(long, default_value = "")]
        description: String,

        /// Initial on-chain CAT precision (do not change unless you know what you are doing)
        #[arg(long, default_value = "3")]
        precision: u8,

        /// Initial CAT metadata URIs (comma-separated list of URIs)
        #[arg(long, default_value = "")]
        metadata_uris: String,

        /// Initial CAT metadata hash
        #[arg(long, required = false)]
        metadata_hash: Option<String>,

        /// Initial CAT license URIs (comma-separated list of URIs)
        #[arg(long, default_value = "")]
        license_uris: String,

        /// Initial CAT license hash
        #[arg(long, required = false)]
        license_hash: Option<String>,

        /// CAT NFT recipient (if not provided, defaults to owner of current wallet)
        #[arg(long, required = false)]
        recipient: Option<String>,

        /// Payment asset id (payment CAT tail hash)
        #[arg(long)]
        payment_asset_id: String,

        /// Payment CAT amount (only provide if refunding)
        #[arg(long, required = false)]
        payment_cat_amount: Option<String>,

        /// Refund path
        #[arg(long, default_value_t = false)]
        refund: bool,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use local database instead of CATalog API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Log the final transaction to a file (sb.debug)
        #[arg(long, default_value_t = false)]
        log: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },

    /// Listen for CATalog spends
    Listen {
        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
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
            MultisigCliAction::View {
                launcher_id,
                testnet11,
            } => multisig_view(launcher_id, testnet11).await,
            MultisigCliAction::SignRekey {
                new_pubkeys,
                new_m,
                my_pubkey,
                launcher_id,
                testnet11,
                debug,
            } => {
                multisig_sign_rekey(new_pubkeys, new_m, my_pubkey, launcher_id, testnet11, debug)
                    .await
            }
            MultisigCliAction::BroadcastRekey {
                new_pubkeys,
                new_m,
                sigs,
                launcher_id,
                testnet11,
                fee,
            } => {
                multisig_broadcast_rekey(new_pubkeys, new_m, sigs, launcher_id, testnet11, fee)
                    .await
            }
            MultisigCliAction::VerifySignature {
                raw_message,
                signature,
                pubkey,
            } => multisig_verify_signature(raw_message, signature, pubkey).await,
            MultisigCliAction::SignCatalogStateUpdate {
                new_payment_asset_id,
                new_payment_asset_amount,
                my_pubkey,
                launcher_id,
                testnet11,
                debug,
            } => {
                multisig_sign_catalog_state_update(
                    new_payment_asset_id,
                    new_payment_asset_amount,
                    my_pubkey,
                    launcher_id,
                    testnet11,
                    debug,
                )
                .await
            }
            MultisigCliAction::BroadcastCatalogStateUpdate {
                new_payment_asset_id,
                new_payment_asset_amount,
                sigs,
                launcher_id,
                testnet11,
                fee,
            } => {
                multisig_broadcast_catalog_state_update(
                    new_payment_asset_id,
                    new_payment_asset_amount,
                    launcher_id,
                    sigs,
                    testnet11,
                    fee,
                )
                .await
            }
        },
        Commands::Catalog { action } => match action {
            CatalogCliAction::InitiateLaunch {
                pubkeys,
                m,
                testnet11,
                fee,
            } => catalog_initiate_launch(pubkeys, m, testnet11, fee).await,
            CatalogCliAction::ContinueLaunch {
                payment_asset_id,
                cats_per_spend,
                testnet11,
                fee,
            } => catalog_continue_launch(payment_asset_id, cats_per_spend, testnet11, fee).await,
            CatalogCliAction::UnrollStateScheduler { testnet11, fee } => {
                catalog_unroll_state_scheduler(testnet11, fee).await
            }
            CatalogCliAction::VerifyDeployment { testnet11 } => {
                catalog_verify_deployment(testnet11).await
            }
            CatalogCliAction::Register {
                tail_reveal,
                ticker,
                name,
                image_uris,
                image_hash,
                description,
                precision,
                metadata_uris,
                metadata_hash,
                license_uris,
                license_hash,
                recipient,
                refund,
                testnet11,
                local,
                log,
                payment_asset_id,
                payment_cat_amount,
                fee,
            } => {
                catalog_register(
                    tail_reveal,
                    ticker,
                    name,
                    description,
                    precision,
                    image_uris,
                    image_hash,
                    metadata_uris,
                    metadata_hash,
                    license_uris,
                    license_hash,
                    recipient,
                    refund,
                    testnet11,
                    local,
                    log,
                    payment_asset_id,
                    payment_cat_amount,
                    fee,
                )
                .await
            }
            CatalogCliAction::Listen { testnet11 } => catalog_listen(testnet11).await,
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

use clap::{Parser, Subcommand};

use super::{
    catalog_broadcast_state_update, catalog_continue_launch, catalog_initiate_launch,
    catalog_listen, catalog_register, catalog_sign_state_update, catalog_unroll_state_scheduler,
    catalog_verify_deployment, dig_add_rewards, dig_broadcast_mirror_update, dig_clawback_rewards,
    dig_commit_rewards, dig_initiate_payout, dig_launch, dig_new_epoch, dig_sign_mirror_update,
    dig_sync, multisig_broadcast_rekey, multisig_launch, multisig_sign_rekey,
    multisig_verify_signature, multisig_view,
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
    /// Interact with DIG Reward Distributors
    Dig {
        #[command(subcommand)]
        action: DigCliAction,
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
    /// Launch a standalone multisig (e.g., for a validator)
    Launch {
        /// Comma-separated list of price singleton pubkeys (no spaces)
        #[arg(long)]
        pubkeys: String,

        /// Threshold required for price singleton spends (m from m-of-n)
        #[arg(long)]
        m: usize,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
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

#[derive(Subcommand)]
enum XchandlesCliAction {
    /// Launches a new XCHandles deployment
    InitiateLaunch,
    /// Continues/finishes an existing launch
    ContinueLaunch,
    /// Verifies the built-in deployment is valid
    VerifyDeployment,
}

#[derive(Subcommand)]
enum DigCliAction {
    /// Launches a new DIG Reward Distributor deployment
    Launch {
        /// Validator singleton launcher id
        #[arg(long)]
        validator_launcher_id: String,

        /// Validator payout address
        #[arg(long)]
        validator_payout_address: String,

        /// First epoch start timestamp
        #[arg(long)]
        first_epoch_start_timestamp: u64,

        /// Reserve (reward token) asset id
        #[arg(long)]
        reserve_asset_id: String,

        /// Launch comment (will be included after the hint that creates the launcher)
        #[arg(long)]
        comment: String,

        /// Seconds in an epoch
        #[arg(long, default_value = "604800")]
        epoch_seconds: u64,

        /// Maximum # seconds the distributor can be 'tricked' into not paying (lower invalidates transactions faster)
        #[arg(long, default_value = "600")]
        max_seconds_offset: u64,

        /// Payout threshold (in the reward token)
        #[arg(long, default_value = "0.1")]
        payout_threshold: String,

        /// Validator fee (in basis points)
        #[arg(long, default_value = "700")]
        validator_fee_bps: u64,

        /// Withdrawal share (how much of a clawed back commitment the recipient gets back)
        #[arg(long, default_value = "9000")]
        withdrawal_share_bps: u64,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Commits rewards to a future epoch
    CommitRewards {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Rewards to commit (in CATs)
        #[arg(long)]
        reward_amount: String,

        /// Epoch start timestamp
        #[arg(long)]
        epoch_start: u64,

        /// Address that will be able to claw back the rewards
        #[arg(long)]
        clawback_address: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Claws back a previous reward commitment
    ClawbackRewards {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Address that will be able to claw back the rewards
        #[arg(long)]
        clawback_address: String,

        /// Epoch start timestamp
        #[arg(long, required = false)]
        epoch_start: Option<u64>,

        /// Commitment amount (in CAT mojos)
        #[arg(long, required = false)]
        reward_amount: Option<String>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Syncs the reward distributor
    Sync {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Update timestamp (defaults to maximum value = timestamp of last transaction block)
        #[arg(long, required = false)]
        update_time: Option<u64>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Starts a new epoch (auto-syncs if needed)
    NewEpoch {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Signs a mirror update action
    SignMirrorUpdate {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Mirror payout puzzle hash
        #[arg(long)]
        mirror_payout_puzzle_hash: String,

        /// Mirror shares
        #[arg(long, default_value = "1")]
        mirror_shares: u64,

        /// Pubkey to sign with (hex string)
        #[arg(long)]
        my_pubkey: String,

        /// Remove mirror (if not provided, mirror will be added)
        #[arg(long, default_value_t = false)]
        remove_mirror: bool,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use debug signing method (pk prompt)
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Broadcasts a mirror update action
    BroadcastMirrorUpdate {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Mirror payout puzzle hash
        #[arg(long)]
        mirror_payout_puzzle_hash: String,

        /// Mirror shares
        #[arg(long, default_value = "1")]
        mirror_shares: u64,

        /// Signatures (comma-separated list)
        #[arg(long)]
        sigs: String,

        /// Remove mirror (if not provided, mirror will be added)
        #[arg(long, default_value_t = false)]
        remove_mirror: bool,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Adds rewards to the current epoch
    AddRewards {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Reward amount (in CAT mojos)
        #[arg(long)]
        reward_amount: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Initiates a payout
    InitiatePayout {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Mirror payout puzzle hash
        #[arg(long)]
        mirror_payout_puzzle_hash: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
}

pub async fn run_cli() {
    let args = Cli::parse();

    let res = match args.command {
        Commands::Multisig { action } => match action {
            MultisigCliAction::View {
                launcher_id,
                testnet11,
            } => multisig_view(launcher_id, testnet11).await,
            MultisigCliAction::Launch {
                pubkeys,
                m,
                testnet11,
                fee,
            } => multisig_launch(pubkeys, m, testnet11, fee).await,
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
            } => multisig_verify_signature(raw_message, pubkey, signature).await,
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
            CatalogCliAction::SignCatalogStateUpdate {
                new_payment_asset_id,
                new_payment_asset_amount,
                my_pubkey,
                launcher_id,
                testnet11,
                debug,
            } => {
                catalog_sign_state_update(
                    new_payment_asset_id,
                    new_payment_asset_amount,
                    my_pubkey,
                    launcher_id,
                    testnet11,
                    debug,
                )
                .await
            }
            CatalogCliAction::BroadcastCatalogStateUpdate {
                new_payment_asset_id,
                new_payment_asset_amount,
                sigs,
                launcher_id,
                testnet11,
                fee,
            } => {
                catalog_broadcast_state_update(
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
        Commands::Dig { action } => match action {
            DigCliAction::Launch {
                validator_launcher_id,
                validator_payout_address,
                first_epoch_start_timestamp,
                epoch_seconds,
                max_seconds_offset,
                payout_threshold,
                validator_fee_bps,
                withdrawal_share_bps,
                reserve_asset_id,
                comment,
                testnet11,
                fee,
            } => {
                dig_launch(
                    validator_launcher_id,
                    validator_payout_address,
                    first_epoch_start_timestamp,
                    epoch_seconds,
                    max_seconds_offset,
                    payout_threshold,
                    validator_fee_bps,
                    withdrawal_share_bps,
                    reserve_asset_id,
                    comment,
                    testnet11,
                    fee,
                )
                .await
            }
            DigCliAction::CommitRewards {
                launcher_id,
                reward_amount,
                epoch_start,
                clawback_address,
                testnet11,
                fee,
            } => {
                dig_commit_rewards(
                    launcher_id,
                    reward_amount,
                    epoch_start,
                    clawback_address,
                    testnet11,
                    fee,
                )
                .await
            }
            DigCliAction::ClawbackRewards {
                launcher_id,
                clawback_address,
                epoch_start,
                reward_amount,
                testnet11,
                fee,
            } => {
                dig_clawback_rewards(
                    launcher_id,
                    clawback_address,
                    epoch_start,
                    reward_amount,
                    testnet11,
                    fee,
                )
                .await
            }
            DigCliAction::Sync {
                launcher_id,
                update_time,
                testnet11,
                fee,
            } => dig_sync(launcher_id, update_time, testnet11, fee).await,
            DigCliAction::NewEpoch {
                launcher_id,
                testnet11,
                fee,
            } => dig_new_epoch(launcher_id, testnet11, fee).await,
            DigCliAction::SignMirrorUpdate {
                launcher_id,
                mirror_payout_puzzle_hash,
                mirror_shares,
                my_pubkey,
                remove_mirror,
                testnet11,
                debug,
            } => {
                dig_sign_mirror_update(
                    launcher_id,
                    mirror_payout_puzzle_hash,
                    mirror_shares,
                    my_pubkey,
                    remove_mirror,
                    testnet11,
                    debug,
                )
                .await
            }
            DigCliAction::BroadcastMirrorUpdate {
                launcher_id,
                mirror_payout_puzzle_hash,
                mirror_shares,
                sigs,
                remove_mirror,
                testnet11,
                fee,
            } => {
                dig_broadcast_mirror_update(
                    launcher_id,
                    mirror_payout_puzzle_hash,
                    mirror_shares,
                    sigs,
                    remove_mirror,
                    testnet11,
                    fee,
                )
                .await
            }
            DigCliAction::AddRewards {
                launcher_id,
                reward_amount,
                testnet11,
                fee,
            } => dig_add_rewards(launcher_id, reward_amount, testnet11, fee).await,
            DigCliAction::InitiatePayout {
                launcher_id,
                mirror_payout_puzzle_hash,
                testnet11,
                fee,
            } => dig_initiate_payout(launcher_id, mirror_payout_puzzle_hash, testnet11, fee).await,
        },
    };

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
}

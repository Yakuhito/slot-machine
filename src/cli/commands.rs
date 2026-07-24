use clap::{Parser, Subcommand};

use crate::{
    reward_distributor_stake, reward_distributor_unstake, xchandles_broadcast_state_update,
    xchandles_execute_update, xchandles_sign_state_update,
};

use super::{
    catalog_broadcast_state_update, catalog_continue_launch, catalog_initiate_launch,
    catalog_listen, catalog_register, catalog_sign_state_update, catalog_unroll_state_scheduler,
    catalog_verify_deployment, datastore_launch, datastore_update, datastore_view,
    multisig_broadcast_rekey, multisig_launch, multisig_sign_rekey, multisig_verify_signature,
    multisig_view, reward_distributor_add_rewards, reward_distributor_broadcast_entry_update,
    reward_distributor_clawback_rewards, reward_distributor_commit_available_rewards,
    reward_distributor_commit_rewards, reward_distributor_initiate_payout,
    reward_distributor_launch, reward_distributor_new_epoch, reward_distributor_refresh,
    reward_distributor_sign_entry_update, reward_distributor_sync, reward_distributor_view,
    xchandles_continue_launch, xchandles_expire, xchandles_extend, xchandles_initiate_launch,
    xchandles_initiate_update, xchandles_listen, xchandles_register,
    xchandles_unroll_state_scheduler, xchandles_verify_deployment, xchandles_view,
};

#[derive(Parser)]
#[command(
    name = "Slot Machine CLI",
    about = "A CLI for interacting with the first dApps that use the slot primitive: CATalog, XCHandles, and reward distributors"
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
    /// Interact with Reward Distributors
    RewardDistributor {
        #[command(subcommand)]
        action: RewardDistributorCliAction,
    },
    /// Interact with DataStores
    Datastore {
        #[command(subcommand)]
        action: DatastoreCliAction,
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
    /// Launch a standalone multisig (e.g., for a manager)
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
    SignStateUpdate {
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
    BroadcastStateUpdate {
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
    InitiateLaunch {
        /// Comma-separated list of price singleton pubkeys (no spaces)
        #[arg(long)]
        pubkeys: String,

        /// Threshold required for price singleton spends (m from m-of-n)
        #[arg(short)]
        m: usize,

        /// Payout address for precommits
        #[arg(long)]
        payout_address: String,

        /// Relative block height for precommits
        #[arg(long, default_value = "32")]
        relative_block_height: u32,

        /// Registration base period in seconds (e.g., a year)
        #[arg(long, default_value = "31557600")]
        registration_period: u64,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use for the launch, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Continues/finishes an existing launch
    ContinueLaunch {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Skip checking this number of handles for being created/precommitted (for efficiency only)
        #[arg(long, default_value = "0")]
        skip: usize,

        /// Payment asset id (payment CAT tail hash from launch initiation)
        #[arg(long)]
        payment_asset_id: String,

        /// Royalty address for minted NFTs
        #[arg(long)]
        royalty_address: String,

        /// Royalty basis points for the launch
        #[arg(long, default_value = "1000")]
        royalty_basis_points: u16,

        /// How many handles to deploy for this spend
        #[arg(long)]
        handles_per_spend: usize,

        /// Start timestamp for premine
        #[arg(long)]
        start_time: Option<u64>,

        /// Registration base period in seconds (e.g., a year)
        #[arg(long, default_value = "31557600")]
        registration_period: u64,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Unrolls the state scheduler
    UnrollStateScheduler {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use local database instead of XCHandles API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Verifies the built-in deployment is valid
    VerifyDeployment {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
    /// Registers a new handle
    Register {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Handle to register
        #[arg(long)]
        handle: String,

        /// NFT (nft1...) to register the handle to (must be in active wallet)
        #[arg(long)]
        nft: String,

        /// Number of periods to register the handle for
        #[arg(long, default_value = "1")]
        num_periods: u64,

        /// Refund address
        #[arg(long)]
        refund_address: Option<String>,

        /// Secret to register the handle with
        #[arg(long)]
        secret: Option<String>,

        /// Start time (UNIX timestamp)
        #[arg(long)]
        start_time: Option<u64>,

        /// Use the registration 'refund' path
        #[arg(long)]
        refund: bool,

        /// Use testnet11
        #[arg(long)]
        testnet11: bool,

        /// Payment asset id
        #[arg(long)]
        payment_asset_id: String,

        /// Payment CAT base price
        #[arg(long)]
        payment_cat_base_price: String,

        /// Registration base period in seconds (e.g., a year)
        #[arg(long, default_value = "31557600")]
        registration_period: u64,

        /// Use local database instead of XCHandles API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Log the final transaction to a file (sb.debug)
        #[arg(long)]
        log: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    // Extend the registration of a handle
    Extend {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Handle to extend
        #[arg(long)]
        handle: String,

        /// Number of periods (e.g., years) to extend the handle for
        #[arg(long, default_value = "1")]
        num_periods: u64,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Payment asset id
        #[arg(long)]
        payment_asset_id: String,

        /// Payment CAT base price
        #[arg(long)]
        payment_cat_base_price: String,

        /// Registration base period in seconds (e.g., a year)
        #[arg(long, default_value = "31557600")]
        registration_period: u64,

        /// Use local database instead of XCHandles API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Initiates an update to the data associated with a handle
    InitiateUpdate {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Handle to update
        #[arg(long)]
        handle: String,

        /// New NFT the handle will point to
        #[arg(long)]
        new_nft: String,

        /// Minimum block height (defaults to current peak)
        #[arg(long)]
        min_height: Option<u32>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use local database instead of XCHandles API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Finishes a previously initiated handle update
    ExecuteUpdate {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Handle to update
        #[arg(long)]
        handle: String,

        /// New NFT the handle will point to
        #[arg(long)]
        new_nft: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use local database instead of XCHandles API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Expires a handle (re-registers after the intial registration expired)
    Expire {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Handle to expire
        #[arg(long)]
        handle: String,

        /// NFT (nft1...) to register the handle to
        #[arg(long)]
        nft: String,

        /// Expire time (UNIX timestamp)
        #[arg(long)]
        expire_time: Option<u64>,

        /// Number of periods to register the handle for
        #[arg(long, default_value = "1")]
        num_periods: u64,

        /// Refund address
        #[arg(long)]
        refund_address: Option<String>,

        /// Secret to register the handle with
        #[arg(long)]
        secret: Option<String>,

        /// Use the 'refund' path to recover a precommit coin
        #[arg(long)]
        refund: bool,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Payment asset id
        #[arg(long)]
        payment_asset_id: String,

        /// Payment CAT base price
        #[arg(long)]
        payment_cat_base_price: String,

        /// Registration base period in seconds (e.g., a year)
        #[arg(long, default_value = "31557600")]
        registration_period: u64,

        /// Committed expiration (old expiration for refunds where someone re-registered the handle before you)
        #[arg(long)]
        committed_expiration: Option<u64>,

        /// Use local database instead of XCHandles API
        #[arg(long, default_value_t = false)]
        local: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Listen for XCHandles spends
    Listen {
        /// XCHandles (sub)registry launcher ids (comma-separated list)
        #[arg(long)]
        launcher_ids: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
    /// Shows up-to-date information about an XCHandles registry
    View {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Payment asset id hint
        #[arg(long)]
        payment_asset_id: Option<String>,

        /// Payment CAT base price hint
        #[arg(long)]
        payment_cat_base_price: Option<String>,

        /// Registration base period in seconds (e.g., a year)
        #[arg(long, default_value = "31557600")]
        registration_period: Option<u64>,
    },
    /// Signs a proposed state update for an XCHandles registry
    SignStateUpdate {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// New payment asset id
        #[arg(long)]
        new_payment_asset_id: String,

        /// New payment CAT base price
        #[arg(long)]
        new_payment_cat_base_price: String,

        /// New registration base period in seconds
        #[arg(long, default_value = "31557600")]
        new_registration_period: u64,

        /// Current payment asset id hint (for current state verification)
        #[arg(long)]
        payment_asset_id: Option<String>,

        /// Current payment CAT base price hint (for current state verification)
        #[arg(long)]
        payment_cat_base_price: Option<String>,

        /// Current registration base period in seconds (for current state verification)
        #[arg(long, default_value = "31557600")]
        registration_period: Option<u64>,

        /// My public key
        #[arg(long)]
        my_pubkey: String,

        /// Multisig/price singleton launcher id
        #[arg(long)]
        multisig_launcher_id: String,

        /// Testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Debug signing mode
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Broadcasts a state update for an XCHandles registry
    BroadcastStateUpdate {
        /// XCHandles (sub)registry launcher id
        #[arg(long)]
        launcher_id: String,

        /// New payment asset id
        #[arg(long)]
        new_payment_asset_id: String,

        /// New payment CAT base price
        #[arg(long)]
        new_payment_cat_base_price: String,

        /// New registration base period in seconds
        #[arg(long, default_value = "31557600")]
        new_registration_period: u64,

        /// Current payment asset id hint (for current state verification)
        #[arg(long)]
        payment_asset_id: Option<String>,

        /// Current payment CAT base price hint (for current state verification)
        #[arg(long)]
        payment_cat_base_price: Option<String>,

        /// Current registration base period in seconds (for current state verification)
        #[arg(long, default_value = "31557600")]
        registration_period: Option<u64>,

        /// Multisig/price singleton launcher id
        #[arg(long)]
        multisig_launcher_id: String,

        /// Signatures from signers
        #[arg(long)]
        signatures: String,

        /// Testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
}

#[derive(Subcommand)]
enum RewardDistributorCliAction {
    /// Launches a new reward distributor
    Launch {
        /// Manager singleton launcher id (for managed reward distributors)
        #[arg(long)]
        manager_launcher_id: Option<String>,

        /// Collection DID (for NFT collection reward distributors)
        #[arg(long)]
        collection_did: Option<String>,

        /// DataStore launcher id (for curated NFT reward distributors)
        #[arg(long)]
        store_launcher_id: Option<String>,

        /// Whether the curated NFT distributor supports refresh (default: false)
        #[arg(long, default_value_t = false)]
        refreshable: bool,

        /// Stakeable CAT asset id (for CAT reward distributors)
        #[arg(long)]
        stake_asset_id: Option<String>,

        /// Require approval for payouts
        #[arg(long, default_value_t = false)]
        require_payout_approval: bool,

        /// Fee payout address
        #[arg(long)]
        fee_payout_address: String,

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
        #[arg(long, default_value = "0.001")]
        payout_threshold: String,

        /// Fee (in basis points)
        #[arg(long, default_value = "1000")]
        fee_bps: u64,

        /// Withdrawal share (how much of a clawed back commitment the recipient gets back)
        #[arg(long, default_value = "8000")]
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
    /// Finds Reward CATs sent to the permissionless next-epoch puzzle and commits them
    CommitAvailableRewards {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Address that can claw back each commitment (defaults to no clawback)
        #[arg(long)]
        clawback_address: Option<String>,

        /// Maximum number of available coins to commit
        #[arg(long, default_value_t = 32)]
        max_coins: usize,

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
    /// Signs an entry update action
    SignEntryUpdate {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Entry payout puzzle hash
        #[arg(long)]
        entry_payout_puzzle_hash: String,

        /// Entry shares
        #[arg(long, default_value = "1")]
        entry_shares: u64,

        /// Pubkey to sign with (hex string)
        #[arg(long)]
        my_pubkey: String,

        /// Remove entry (if not provided, entry will be added)
        #[arg(long, default_value_t = false)]
        remove_entry: bool,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Use debug signing method (pk prompt)
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Broadcasts an entry update action
    BroadcastEntryUpdate {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Entry payout puzzle hash
        #[arg(long)]
        entry_payout_puzzle_hash: String,

        /// Entry shares
        #[arg(long, default_value = "1")]
        entry_shares: u64,

        /// Signatures (comma-separated list)
        #[arg(long)]
        sigs: String,

        /// Remove entry (if not provided, entry will be added)
        #[arg(long, default_value_t = false)]
        remove_entry: bool,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Stake an NFT or CAT into the reward distributor
    Stake {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// NFT id (nft1...) (required for NFT collection and curated NFT distributors)
        #[arg(long)]
        nft: Option<String>,

        /// Stake amount in CAT mojos (required for CAT distributors)
        #[arg(long)]
        stake_amount: Option<String>,

        /// Whitelist CSV (required for curated NFT distributors)
        #[arg(long)]
        csv: Option<String>,

        /// Custody address (xch1...) for the entry slot; defaults to first wallet derivation
        #[arg(long)]
        custody_address: Option<String>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Unstake an NFT or CAT from the reward distributor
    Unstake {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Custody address (xch1...) used when staking; defaults to first wallet derivation
        #[arg(long)]
        custody_address: Option<String>,

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

        /// Custody address (xch1...) for the entry slot; defaults to first wallet derivation
        #[arg(long)]
        custody_address: Option<String>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Views up-to-date information about a reward distributor
    View {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
    },
    /// Refresh locked NFT share counts from an updated whitelist CSV
    Refresh {
        /// Reward distributor singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Whitelist CSV whose root must match the current on-chain datastore root
        #[arg(long)]
        csv: String,

        /// Custody address (xch1...) used when staking; defaults to first wallet derivation
        #[arg(long)]
        custody_address: Option<String>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
}

#[derive(Subcommand)]
enum DatastoreCliAction {
    /// Launch a new datastore
    Launch {
        /// CSV file with columns nft_id,weight
        #[arg(long)]
        csv: String,

        /// Optional label (will be stored as on-chain metadata)
        #[arg(long)]
        label: Option<String>,

        /// Optional description (will be stored as on-chain metadata)
        #[arg(long)]
        description: Option<String>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// Update datastore (metadata and NFT whitelist root hash)
    Update {
        /// Datastore singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Previous whitelist CSV
        #[arg(long)]
        old_csv: String,

        /// Updated whitelist CSV
        #[arg(long)]
        new_csv: String,

        /// New label (will be stored as on-chain metadata; empty = keep current)
        #[arg(long)]
        label: Option<String>,

        /// New description (will be stored as on-chain metadata; empty = keep current)
        #[arg(long)]
        description: Option<String>,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,

        /// Fee to use, in XCH
        #[arg(long, default_value = "0.0025")]
        fee: String,
    },
    /// View current DataStore metadata and latest coin
    View {
        /// DataStore singleton launcher id
        #[arg(long)]
        launcher_id: String,

        /// Use testnet11
        #[arg(long, default_value_t = false)]
        testnet11: bool,
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
            CatalogCliAction::SignStateUpdate {
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
            CatalogCliAction::BroadcastStateUpdate {
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
            XchandlesCliAction::InitiateLaunch {
                pubkeys,
                m,
                payout_address,
                relative_block_height,
                registration_period,
                testnet11,
                fee,
            } => {
                xchandles_initiate_launch(
                    pubkeys,
                    m,
                    payout_address,
                    relative_block_height,
                    registration_period,
                    testnet11,
                    fee,
                )
                .await
            }
            XchandlesCliAction::ContinueLaunch {
                launcher_id,
                skip,
                payment_asset_id,
                royalty_address,
                royalty_basis_points,
                handles_per_spend,
                start_time,
                registration_period,
                testnet11,
                fee,
            } => {
                xchandles_continue_launch(
                    launcher_id,
                    skip,
                    payment_asset_id,
                    royalty_address,
                    royalty_basis_points,
                    handles_per_spend,
                    start_time,
                    registration_period,
                    testnet11,
                    fee,
                )
                .await
            }
            XchandlesCliAction::UnrollStateScheduler {
                launcher_id,
                testnet11,
                local,
                fee,
            } => xchandles_unroll_state_scheduler(launcher_id, testnet11, local, fee).await,
            XchandlesCliAction::VerifyDeployment {
                launcher_id,
                testnet11,
            } => xchandles_verify_deployment(launcher_id, testnet11).await,
            XchandlesCliAction::Register {
                launcher_id,
                handle,
                nft,
                num_periods,
                refund_address,
                secret,
                start_time,
                refund,
                testnet11,
                payment_asset_id,
                payment_cat_base_price,
                registration_period,
                local,
                log,
                fee,
            } => {
                xchandles_register(
                    launcher_id,
                    handle,
                    nft,
                    num_periods,
                    refund_address,
                    secret,
                    start_time,
                    refund,
                    testnet11,
                    payment_asset_id,
                    payment_cat_base_price,
                    registration_period,
                    log,
                    local,
                    fee,
                )
                .await
            }
            XchandlesCliAction::Extend {
                launcher_id,
                handle,
                num_periods,
                testnet11,
                payment_asset_id,
                payment_cat_base_price,
                registration_period,
                local,
                fee,
            } => {
                xchandles_extend(
                    launcher_id,
                    handle,
                    num_periods,
                    testnet11,
                    payment_asset_id,
                    payment_cat_base_price,
                    registration_period,
                    local,
                    fee,
                )
                .await
            }
            XchandlesCliAction::InitiateUpdate {
                launcher_id,
                handle,
                new_nft,
                min_height,
                testnet11,
                local,
                fee,
            } => {
                xchandles_initiate_update(
                    launcher_id,
                    handle,
                    new_nft,
                    min_height,
                    testnet11,
                    local,
                    fee,
                )
                .await
            }
            XchandlesCliAction::ExecuteUpdate {
                launcher_id,
                handle,
                new_nft,
                testnet11,
                local,
                fee,
            } => {
                xchandles_execute_update(launcher_id, handle, new_nft, testnet11, local, fee).await
            }
            XchandlesCliAction::Expire {
                launcher_id,
                handle,
                nft,
                refund_address,
                secret,
                expire_time,
                num_periods,
                refund,
                testnet11,
                payment_asset_id,
                payment_cat_base_price,
                registration_period,
                committed_expiration,
                local,
                fee,
            } => {
                xchandles_expire(
                    launcher_id,
                    handle,
                    nft,
                    num_periods,
                    refund_address,
                    secret,
                    expire_time,
                    refund,
                    testnet11,
                    payment_asset_id,
                    payment_cat_base_price,
                    registration_period,
                    committed_expiration,
                    local,
                    fee,
                )
                .await
            }
            XchandlesCliAction::Listen {
                testnet11,
                launcher_ids,
            } => xchandles_listen(launcher_ids, testnet11).await,
            XchandlesCliAction::View {
                launcher_id,
                testnet11,
                payment_asset_id,
                payment_cat_base_price,
                registration_period,
            } => {
                xchandles_view(
                    launcher_id,
                    testnet11,
                    payment_asset_id,
                    payment_cat_base_price,
                    registration_period,
                )
                .await
            }
            XchandlesCliAction::SignStateUpdate {
                launcher_id,
                new_payment_asset_id,
                new_payment_cat_base_price,
                new_registration_period,
                payment_asset_id,
                payment_cat_base_price,
                registration_period,
                my_pubkey,
                multisig_launcher_id,
                testnet11,
                debug,
            } => {
                xchandles_sign_state_update(
                    launcher_id,
                    new_payment_asset_id,
                    new_payment_cat_base_price,
                    new_registration_period,
                    payment_asset_id,
                    payment_cat_base_price,
                    registration_period,
                    my_pubkey,
                    multisig_launcher_id,
                    testnet11,
                    debug,
                )
                .await
            }
            XchandlesCliAction::BroadcastStateUpdate {
                launcher_id,
                new_payment_asset_id,
                new_payment_cat_base_price,
                new_registration_period,
                payment_asset_id,
                payment_cat_base_price,
                registration_period,
                multisig_launcher_id,
                signatures,
                testnet11,
                fee,
            } => {
                xchandles_broadcast_state_update(
                    launcher_id,
                    new_payment_asset_id,
                    new_payment_cat_base_price,
                    new_registration_period,
                    payment_asset_id,
                    payment_cat_base_price,
                    registration_period,
                    multisig_launcher_id,
                    signatures,
                    testnet11,
                    fee,
                )
                .await
            }
        },
        Commands::RewardDistributor { action } => match action {
            RewardDistributorCliAction::Launch {
                manager_launcher_id,
                collection_did,
                store_launcher_id,
                refreshable,
                stake_asset_id,
                require_payout_approval,
                fee_payout_address,
                first_epoch_start_timestamp,
                epoch_seconds,
                max_seconds_offset,
                payout_threshold,
                fee_bps,
                withdrawal_share_bps,
                reserve_asset_id,
                comment,
                testnet11,
                fee,
            } => {
                reward_distributor_launch(
                    manager_launcher_id,
                    collection_did,
                    store_launcher_id,
                    refreshable,
                    stake_asset_id,
                    require_payout_approval,
                    fee_payout_address,
                    first_epoch_start_timestamp,
                    epoch_seconds,
                    max_seconds_offset,
                    payout_threshold,
                    fee_bps,
                    withdrawal_share_bps,
                    reserve_asset_id,
                    comment,
                    testnet11,
                    fee,
                )
                .await
            }
            RewardDistributorCliAction::CommitRewards {
                launcher_id,
                reward_amount,
                epoch_start,
                clawback_address,
                testnet11,
                fee,
            } => {
                reward_distributor_commit_rewards(
                    launcher_id,
                    reward_amount,
                    epoch_start,
                    clawback_address,
                    testnet11,
                    fee,
                )
                .await
            }
            RewardDistributorCliAction::CommitAvailableRewards {
                launcher_id,
                clawback_address,
                max_coins,
                testnet11,
                fee,
            } => {
                reward_distributor_commit_available_rewards(
                    launcher_id,
                    clawback_address,
                    max_coins,
                    testnet11,
                    fee,
                )
                .await
            }
            RewardDistributorCliAction::ClawbackRewards {
                launcher_id,
                clawback_address,
                epoch_start,
                reward_amount,
                testnet11,
                fee,
            } => {
                reward_distributor_clawback_rewards(
                    launcher_id,
                    clawback_address,
                    epoch_start,
                    reward_amount,
                    testnet11,
                    fee,
                )
                .await
            }
            RewardDistributorCliAction::Sync {
                launcher_id,
                update_time,
                testnet11,
                fee,
            } => reward_distributor_sync(launcher_id, update_time, testnet11, fee).await,
            RewardDistributorCliAction::NewEpoch {
                launcher_id,
                testnet11,
                fee,
            } => reward_distributor_new_epoch(launcher_id, testnet11, fee).await,
            RewardDistributorCliAction::SignEntryUpdate {
                launcher_id,
                entry_payout_puzzle_hash,
                entry_shares,
                my_pubkey,
                remove_entry,
                testnet11,
                debug,
            } => {
                reward_distributor_sign_entry_update(
                    launcher_id,
                    entry_payout_puzzle_hash,
                    entry_shares,
                    my_pubkey,
                    remove_entry,
                    testnet11,
                    debug,
                )
                .await
            }
            RewardDistributorCliAction::BroadcastEntryUpdate {
                launcher_id,
                entry_payout_puzzle_hash,
                entry_shares,
                sigs,
                remove_entry,
                testnet11,
                fee,
            } => {
                reward_distributor_broadcast_entry_update(
                    launcher_id,
                    entry_payout_puzzle_hash,
                    entry_shares,
                    sigs,
                    remove_entry,
                    testnet11,
                    fee,
                )
                .await
            }
            RewardDistributorCliAction::Stake {
                launcher_id,
                nft,
                stake_amount,
                csv,
                custody_address,
                testnet11,
                fee,
            } => {
                reward_distributor_stake(
                    launcher_id,
                    nft,
                    stake_amount,
                    csv,
                    custody_address,
                    testnet11,
                    fee,
                )
                .await
            }
            RewardDistributorCliAction::Unstake {
                launcher_id,
                custody_address,
                testnet11,
                fee,
            } => reward_distributor_unstake(launcher_id, custody_address, testnet11, fee).await,
            RewardDistributorCliAction::AddRewards {
                launcher_id,
                reward_amount,
                testnet11,
                fee,
            } => reward_distributor_add_rewards(launcher_id, reward_amount, testnet11, fee).await,
            RewardDistributorCliAction::InitiatePayout {
                launcher_id,
                custody_address,
                testnet11,
                fee,
            } => {
                reward_distributor_initiate_payout(launcher_id, custody_address, testnet11, fee)
                    .await
            }
            RewardDistributorCliAction::View {
                launcher_id,
                testnet11,
            } => reward_distributor_view(launcher_id, testnet11).await,
            RewardDistributorCliAction::Refresh {
                launcher_id,
                csv,
                custody_address,
                testnet11,
                fee,
            } => {
                reward_distributor_refresh(launcher_id, csv, custody_address, testnet11, fee).await
            }
        },
        Commands::Datastore { action } => match action {
            DatastoreCliAction::Launch {
                csv,
                label,
                description,
                testnet11,
                fee,
            } => datastore_launch(csv, label, description, testnet11, fee).await,
            DatastoreCliAction::Update {
                launcher_id,
                old_csv,
                new_csv,
                label,
                description,
                testnet11,
                fee,
            } => {
                datastore_update(
                    launcher_id,
                    old_csv,
                    new_csv,
                    label,
                    description,
                    testnet11,
                    fee,
                )
                .await
            }
            DatastoreCliAction::View {
                launcher_id,
                testnet11,
            } => datastore_view(launcher_id, testnet11).await,
        },
    };

    if let Err(err) = res {
        eprintln!("Error: {err}");
    }
}

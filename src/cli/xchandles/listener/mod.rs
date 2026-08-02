//! Public XCHandles listener: singleton discovery, follow, and HTTP reads.
//!
//! Tickets 11–14 extend the same real-HTTP fixture and golden contract established here.

mod api;
mod discovery;
mod error;
mod freshness;
mod handle_store;
mod index;
mod pending_store;
mod refs;
mod registration_store;
mod store;
mod types;

pub use api::{ListenerApiState, listener_router, serve_listener};
pub use discovery::{
    DiscoveredSingleton, DiscoveryResult, FollowSpendResult, ParsedNftState,
    discover_singleton_in_block, follow_singleton_spend,
};
pub use error::ApiError;
pub use freshness::FreshnessState;
pub use handle_store::{
    DbHandleSlotStore, HandleSlotRecord, HandleSlotStore, MemoryHandleSlotStore, StoredHandleSlot,
    prune_handle_history, push_handle_replacement, rollback_handle_to_before,
};
pub use index::SingletonIndexer;
pub use pending_store::{
    DbPendingUpdateStore, MemoryPendingUpdateStore, PendingUpdateRecord, PendingUpdateStore,
    StoredPendingUpdate, clear_pending_current, prune_pending_history, push_pending_replacement,
    rollback_pending_to_before,
};
pub use refs::{SingletonReference, dereferenced_launchers, references_from_action_log};
pub use registration_store::{
    DbRegistrationStore, MemoryRegistrationStore, RegistrationActionKind, RegistrationRecord,
    RegistrationStore, RegistryRegistrationStats, StoredRegistration, StoredRegistrationEvent,
    prune_registration_history, push_registration_replacement, rollback_registration_to_before,
    rollback_stats_to_before,
};
pub use store::{
    DbSingletonStore, FollowRecordStatus, FollowedSingleton, MemorySingletonStore, SingletonStore,
    StoredSingletonState, prune_history, push_replacement, rollback_to_before,
};
pub use types::{
    ApiErrorBody, HandleProofResponse, HandleQuery, HandleSlotJson, PendingTransferQuery,
    PendingTransferResponse, RecentRegistrationItem, RecentRegistrationsQuery,
    RecentRegistrationsResponse, RegistrationQuery, RegistrationResponse, SingletonNftDetails,
    SingletonQuery, SingletonResponse, SlotNeighborsJson, hex32, is_canonical_handle,
    parse_launcher_id,
};

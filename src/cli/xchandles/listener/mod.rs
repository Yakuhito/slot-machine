//! Public XCHandles listener: singleton discovery, follow, and HTTP reads.
//!
//! Tickets 11–14 extend the same real-HTTP fixture and golden contract established here.

mod api;
mod discovery;
mod error;
mod freshness;
mod handle_store;
mod index;
mod refs;
mod registration_store;
mod store;
mod types;

pub use api::{listener_router, serve_listener, ListenerApiState};
pub use discovery::{
    discover_singleton_in_block, follow_singleton_spend, DiscoveryResult, DiscoveredSingleton,
    FollowSpendResult, ParsedNftState,
};
pub use error::ApiError;
pub use freshness::FreshnessState;
pub use handle_store::{
    prune_handle_history, push_handle_replacement, rollback_handle_to_before, DbHandleSlotStore,
    HandleSlotRecord, HandleSlotStore, MemoryHandleSlotStore, StoredHandleSlot,
};
pub use index::SingletonIndexer;
pub use refs::{dereferenced_launchers, references_from_action_log, SingletonReference};
pub use registration_store::{
    prune_registration_history, push_registration_replacement, rollback_registration_to_before,
    rollback_stats_to_before, DbRegistrationStore, MemoryRegistrationStore, RegistrationActionKind,
    RegistrationRecord, RegistrationStore, RegistryRegistrationStats, StoredRegistration,
    StoredRegistrationEvent,
};
pub use store::{
    prune_history, push_replacement, rollback_to_before, DbSingletonStore, FollowRecordStatus,
    FollowedSingleton, MemorySingletonStore, SingletonStore, StoredSingletonState,
};
pub use types::{
    hex32, is_canonical_handle, parse_launcher_id, ApiErrorBody, HandleProofResponse, HandleQuery,
    HandleSlotJson, RecentRegistrationItem, RecentRegistrationsQuery, RecentRegistrationsResponse,
    RegistrationQuery, RegistrationResponse, SingletonNftDetails, SingletonQuery, SingletonResponse,
    SlotNeighborsJson,
};

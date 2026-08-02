//! Public XCHandles listener: singleton discovery, follow, and HTTP reads.
//!
//! Tickets 11–14 extend the same real-HTTP fixture and golden contract established here.

mod api;
mod discovery;
mod error;
mod freshness;
mod index;
mod refs;
mod store;
mod types;

pub use api::{listener_router, serve_listener, ListenerApiState};
pub use discovery::{
    discover_singleton_in_block, follow_singleton_spend, DiscoveryResult, DiscoveredSingleton,
    FollowSpendResult, ParsedNftState,
};
pub use error::ApiError;
pub use freshness::FreshnessState;
pub use index::SingletonIndexer;
pub use refs::{dereferenced_launchers, references_from_action_log, SingletonReference};
pub use store::{
    prune_history, push_replacement, rollback_to_before, DbSingletonStore, FollowRecordStatus,
    FollowedSingleton, MemorySingletonStore, SingletonStore, StoredSingletonState,
};
pub use types::{
    hex32, parse_launcher_id, ApiErrorBody, SingletonNftDetails, SingletonQuery, SingletonResponse,
};

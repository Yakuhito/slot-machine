use std::collections::HashMap;
use std::sync::Arc;

use chia_protocol::Bytes32;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Confirmed registration or expiry-auction purchase fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationActionKind {
    Register,
    Expire,
}

/// Canonical latest-registration projection for one Handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRegistration {
    pub registry_launcher_id: Bytes32,
    pub handle: String,
    pub handle_hash: Bytes32,
    pub registration_secret: Bytes32,
    pub action_kind: RegistrationActionKind,
    pub protocol_fee: u64,
    pub confirmation_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationRecord {
    pub registry_launcher_id: Bytes32,
    pub handle_hash: Bytes32,
    pub current: Option<StoredRegistration>,
    /// Prior facts for pre-final reorganization restoration.
    pub history: Vec<StoredRegistration>,
}

/// One confirmed Registration Event for the recent feed (oldest-first in storage).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRegistrationEvent {
    pub handle: String,
    pub action_kind: RegistrationActionKind,
    pub confirmation_height: u32,
}

/// Per-registry cumulative register count and confirmed event log.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryRegistrationStats {
    pub total_registered: u64,
    pub events: Vec<StoredRegistrationEvent>,
}

#[async_trait::async_trait]
pub trait RegistrationStore: Send + Sync {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<RegistrationRecord>;
    async fn upsert(&self, record: RegistrationRecord);
    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32);
    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)>;

    async fn get_stats(&self, registry_launcher_id: Bytes32) -> RegistryRegistrationStats;
    async fn set_stats(&self, registry_launcher_id: Bytes32, stats: RegistryRegistrationStats);
    async fn all_stats_registry_ids(&self) -> Vec<Bytes32>;
}

fn key(registry: Bytes32, handle_hash: Bytes32) -> (Bytes32, Bytes32) {
    (registry, handle_hash)
}

#[derive(Default)]
pub struct MemoryRegistrationStore {
    records: RwLock<HashMap<(Bytes32, Bytes32), RegistrationRecord>>,
    stats: RwLock<HashMap<Bytes32, RegistryRegistrationStats>>,
}

impl MemoryRegistrationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[async_trait::async_trait]
impl RegistrationStore for MemoryRegistrationStore {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<RegistrationRecord> {
        self.records
            .read()
            .await
            .get(&key(registry_launcher_id, handle_hash))
            .cloned()
    }

    async fn upsert(&self, record: RegistrationRecord) {
        self.records.write().await.insert(
            key(record.registry_launcher_id, record.handle_hash),
            record,
        );
    }

    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32) {
        self.records
            .write()
            .await
            .remove(&key(registry_launcher_id, handle_hash));
    }

    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)> {
        self.records.read().await.keys().copied().collect()
    }

    async fn get_stats(&self, registry_launcher_id: Bytes32) -> RegistryRegistrationStats {
        self.stats
            .read()
            .await
            .get(&registry_launcher_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn set_stats(&self, registry_launcher_id: Bytes32, stats: RegistryRegistrationStats) {
        self.stats.write().await.insert(registry_launcher_id, stats);
    }

    async fn all_stats_registry_ids(&self) -> Vec<Bytes32> {
        self.stats.read().await.keys().copied().collect()
    }
}

/// Retain current fact, every replaced fact in the last 32 blocks, and one older predecessor.
pub fn prune_registration_history(history: &mut Vec<StoredRegistration>, peak: u32) {
    if history.is_empty() {
        return;
    }
    let cutoff = peak.saturating_sub(32);
    let mut keep_recent = Vec::new();
    let mut older = None;
    for state in history.drain(..) {
        if state.confirmation_height >= cutoff {
            keep_recent.push(state);
        } else {
            older = Some(state);
        }
    }
    if let Some(pred) = older {
        history.push(pred);
    }
    history.extend(keep_recent);
}

pub fn push_registration_replacement(
    record: &mut RegistrationRecord,
    new_state: StoredRegistration,
    peak: u32,
) {
    if let Some(prev) = record.current.take() {
        record.history.push(prev);
    }
    record.current = Some(new_state);
    prune_registration_history(&mut record.history, peak);
}

/// Restore prior registration fact after a pre-final reorganization.
pub fn rollback_registration_to_before(record: &mut RegistrationRecord, from_height: u32) {
    if let Some(cur) = &record.current {
        if cur.confirmation_height >= from_height {
            record.current = None;
        }
    }
    let mut restored = None;
    let mut kept = Vec::new();
    for state in record.history.drain(..) {
        if state.confirmation_height < from_height {
            restored = Some(state.clone());
            kept.push(state);
        }
    }
    record.history = kept;
    if record.current.is_none() {
        record.current = restored;
    }
}

/// Drop orphaned recent events and reverse register counts for a reorganization.
pub fn rollback_stats_to_before(stats: &mut RegistryRegistrationStats, from_height: u32) {
    while let Some(ev) = stats.events.last() {
        if ev.confirmation_height < from_height {
            break;
        }
        let ev = stats.events.pop().expect("last checked");
        if ev.action_kind == RegistrationActionKind::Register {
            stats.total_registered = stats.total_registered.saturating_sub(1);
        }
    }
}

/// SQLite-backed registration projections used by the production `listen` process.
pub struct DbRegistrationStore {
    db: Arc<futures::lock::Mutex<crate::Db>>,
}

impl DbRegistrationStore {
    pub fn new(db: Arc<futures::lock::Mutex<crate::Db>>) -> Arc<Self> {
        Arc::new(Self { db })
    }
}

#[async_trait::async_trait]
impl RegistrationStore for DbRegistrationStore {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<RegistrationRecord> {
        let db = self.db.lock().await;
        let json = db
            .get_registration_record_json(registry_launcher_id, handle_hash)
            .await
            .ok()??;
        serde_json::from_str(&json).ok()
    }

    async fn upsert(&self, record: RegistrationRecord) {
        let json = match serde_json::to_string(&record) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("registration serialize error: {e}");
                return;
            }
        };
        let db = self.db.lock().await;
        if let Err(e) = db
            .upsert_registration_record_json(record.registry_launcher_id, record.handle_hash, &json)
            .await
        {
            eprintln!("registration upsert error: {e}");
        }
    }

    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32) {
        let db = self.db.lock().await;
        let _ = db
            .delete_registration_record(registry_launcher_id, handle_hash)
            .await;
    }

    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)> {
        let db = self.db.lock().await;
        db.all_registration_keys().await.unwrap_or_default()
    }

    async fn get_stats(&self, registry_launcher_id: Bytes32) -> RegistryRegistrationStats {
        let db = self.db.lock().await;
        let Ok(Some(json)) = db.get_registration_stats_json(registry_launcher_id).await else {
            return RegistryRegistrationStats::default();
        };
        serde_json::from_str(&json).unwrap_or_default()
    }

    async fn set_stats(&self, registry_launcher_id: Bytes32, stats: RegistryRegistrationStats) {
        let json = match serde_json::to_string(&stats) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("registration stats serialize error: {e}");
                return;
            }
        };
        let db = self.db.lock().await;
        if let Err(e) = db
            .upsert_registration_stats_json(registry_launcher_id, &json)
            .await
        {
            eprintln!("registration stats upsert error: {e}");
        }
    }

    async fn all_stats_registry_ids(&self) -> Vec<Bytes32> {
        let db = self.db.lock().await;
        db.all_registration_stats_registry_ids()
            .await
            .unwrap_or_default()
    }
}

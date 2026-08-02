use std::collections::HashMap;
use std::sync::Arc;

use chia_protocol::Bytes32;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Canonical pending-update projection for one Handle (from an InitiateUpdate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPendingUpdate {
    pub registry_launcher_id: Bytes32,
    pub handle_hash: Bytes32,
    pub new_owner_launcher_id: Bytes32,
    pub new_resolved_launcher_id: Bytes32,
    pub update_confirmation_height: u32,
    pub minimum_execution_height: u32,
    pub update_initiator_coin_id: Bytes32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdateRecord {
    pub registry_launcher_id: Bytes32,
    pub handle_hash: Bytes32,
    pub current: Option<StoredPendingUpdate>,
    /// Prior pending states for pre-final reorganization restoration.
    pub history: Vec<StoredPendingUpdate>,
}

#[async_trait::async_trait]
pub trait PendingUpdateStore: Send + Sync {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<PendingUpdateRecord>;
    async fn upsert(&self, record: PendingUpdateRecord);
    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32);
    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)>;
}

fn key(registry: Bytes32, handle_hash: Bytes32) -> (Bytes32, Bytes32) {
    (registry, handle_hash)
}

#[derive(Default)]
pub struct MemoryPendingUpdateStore {
    inner: RwLock<HashMap<(Bytes32, Bytes32), PendingUpdateRecord>>,
}

impl MemoryPendingUpdateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[async_trait::async_trait]
impl PendingUpdateStore for MemoryPendingUpdateStore {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<PendingUpdateRecord> {
        self.inner
            .read()
            .await
            .get(&key(registry_launcher_id, handle_hash))
            .cloned()
    }

    async fn upsert(&self, record: PendingUpdateRecord) {
        self.inner
            .write()
            .await
            .insert(key(record.registry_launcher_id, record.handle_hash), record);
    }

    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32) {
        self.inner
            .write()
            .await
            .remove(&key(registry_launcher_id, handle_hash));
    }

    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)> {
        self.inner.read().await.keys().copied().collect()
    }
}

/// Retain current state, every replaced state in the last 32 blocks, and one older predecessor.
pub fn prune_pending_history(history: &mut Vec<StoredPendingUpdate>, peak: u32) {
    if history.is_empty() {
        return;
    }
    let cutoff = peak.saturating_sub(32);
    let mut keep_recent = Vec::new();
    let mut older = None;
    for state in history.drain(..) {
        if state.update_confirmation_height >= cutoff {
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

pub fn push_pending_replacement(
    record: &mut PendingUpdateRecord,
    new_state: StoredPendingUpdate,
    peak: u32,
) {
    if let Some(prev) = record.current.take() {
        record.history.push(prev);
    }
    record.current = Some(new_state);
    prune_pending_history(&mut record.history, peak);
}

/// Clear the current pending update while retaining history for rollback.
pub fn clear_pending_current(record: &mut PendingUpdateRecord, peak: u32) {
    if let Some(prev) = record.current.take() {
        record.history.push(prev);
        prune_pending_history(&mut record.history, peak);
    }
}

/// Restore prior pending-update state after a pre-final reorganization.
pub fn rollback_pending_to_before(record: &mut PendingUpdateRecord, from_height: u32) {
    if let Some(cur) = &record.current {
        if cur.update_confirmation_height >= from_height {
            record.current = None;
        }
    }
    let mut restored = None;
    let mut kept = Vec::new();
    for state in record.history.drain(..) {
        if state.update_confirmation_height < from_height {
            restored = Some(state.clone());
            kept.push(state);
        }
    }
    record.history = kept;
    if record.current.is_none() {
        record.current = restored;
    }
}

/// SQLite-backed pending-update projection used by the production `listen` process.
pub struct DbPendingUpdateStore {
    db: Arc<futures::lock::Mutex<crate::Db>>,
}

impl DbPendingUpdateStore {
    pub fn new(db: Arc<futures::lock::Mutex<crate::Db>>) -> Arc<Self> {
        Arc::new(Self { db })
    }
}

#[async_trait::async_trait]
impl PendingUpdateStore for DbPendingUpdateStore {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<PendingUpdateRecord> {
        let db = self.db.lock().await;
        let json = db
            .get_pending_update_record_json(registry_launcher_id, handle_hash)
            .await
            .ok()??;
        serde_json::from_str(&json).ok()
    }

    async fn upsert(&self, record: PendingUpdateRecord) {
        let json = match serde_json::to_string(&record) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("pending update serialize error: {e}");
                return;
            }
        };
        let db = self.db.lock().await;
        if let Err(e) = db
            .upsert_pending_update_record_json(
                record.registry_launcher_id,
                record.handle_hash,
                &json,
            )
            .await
        {
            eprintln!("pending update upsert error: {e}");
        }
    }

    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32) {
        let db = self.db.lock().await;
        let _ = db
            .delete_pending_update_record(registry_launcher_id, handle_hash)
            .await;
    }

    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)> {
        let db = self.db.lock().await;
        db.all_pending_update_keys().await.unwrap_or_default()
    }
}

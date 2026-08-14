use std::collections::HashMap;
use std::sync::Arc;

use chia_protocol::Bytes32;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Canonical Handle-slot projection used by unified Handle proofs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredHandleSlot {
    pub registry_launcher_id: Bytes32,
    pub handle_hash: Bytes32,
    pub counter: u64,
    pub neighbors_left: Bytes32,
    pub neighbors_right: Bytes32,
    pub expiration: u64,
    pub owner_launcher_id: Bytes32,
    pub resolved_launcher_id: Bytes32,
    pub parent_coin_id: Bytes32,
    pub confirmation_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleSlotRecord {
    pub registry_launcher_id: Bytes32,
    pub handle_hash: Bytes32,
    pub current: Option<StoredHandleSlot>,
    /// Prior states for pre-final reorganization restoration.
    pub history: Vec<StoredHandleSlot>,
}

#[async_trait::async_trait]
pub trait HandleSlotStore: Send + Sync {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<HandleSlotRecord>;
    async fn upsert(&self, record: HandleSlotRecord);
    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32);
    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)>;
}

fn key(registry: Bytes32, handle_hash: Bytes32) -> (Bytes32, Bytes32) {
    (registry, handle_hash)
}

#[derive(Default)]
pub struct MemoryHandleSlotStore {
    inner: RwLock<HashMap<(Bytes32, Bytes32), HandleSlotRecord>>,
}

impl MemoryHandleSlotStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[async_trait::async_trait]
impl HandleSlotStore for MemoryHandleSlotStore {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<HandleSlotRecord> {
        self.inner
            .read()
            .await
            .get(&key(registry_launcher_id, handle_hash))
            .cloned()
    }

    async fn upsert(&self, record: HandleSlotRecord) {
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
pub fn prune_handle_history(history: &mut Vec<StoredHandleSlot>, peak: u32) {
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

pub fn push_handle_replacement(
    record: &mut HandleSlotRecord,
    new_state: StoredHandleSlot,
    peak: u32,
) {
    if let Some(prev) = record.current.take() {
        record.history.push(prev);
    }
    record.current = Some(new_state);
    prune_handle_history(&mut record.history, peak);
}

/// Restore prior Handle-slot state after a pre-final reorganization.
pub fn rollback_handle_to_before(record: &mut HandleSlotRecord, from_height: u32) {
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

/// SQLite-backed Handle-slot projection used by the production `listen` process.
pub struct DbHandleSlotStore {
    db: Arc<futures::lock::Mutex<crate::Db>>,
}

impl DbHandleSlotStore {
    pub fn new(db: Arc<futures::lock::Mutex<crate::Db>>) -> Arc<Self> {
        Arc::new(Self { db })
    }
}

#[async_trait::async_trait]
impl HandleSlotStore for DbHandleSlotStore {
    async fn get(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
    ) -> Option<HandleSlotRecord> {
        let db = self.db.lock().await;
        let json = db
            .get_handle_slot_record_json(registry_launcher_id, handle_hash)
            .await
            .ok()??;
        serde_json::from_str(&json).ok()
    }

    async fn upsert(&self, record: HandleSlotRecord) {
        let json = match serde_json::to_string(&record) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("handle slot serialize error: {e}");
                return;
            }
        };
        let db = self.db.lock().await;
        if let Err(e) = db
            .upsert_handle_slot_record_json(record.registry_launcher_id, record.handle_hash, &json)
            .await
        {
            eprintln!("handle slot upsert error: {e}");
        }
    }

    async fn remove(&self, registry_launcher_id: Bytes32, handle_hash: Bytes32) {
        let db = self.db.lock().await;
        let _ = db
            .delete_handle_slot_record(registry_launcher_id, handle_hash)
            .await;
    }

    async fn all_keys(&self) -> Vec<(Bytes32, Bytes32)> {
        let db = self.db.lock().await;
        db.all_handle_slot_keys().await.unwrap_or_default()
    }
}

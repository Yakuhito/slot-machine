use std::collections::HashMap;
use std::sync::Arc;

use chia_protocol::Bytes32;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::registration_store::RegistrationStore;

/// Parent coin id plus compact lineage proof needed to spend a Handle slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotParentLineage {
    pub parent_coin_id: Bytes32,
    pub parent_parent_id: Bytes32,
    pub parent_inner_puzzle_hash: Bytes32,
}

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
    #[serde(default)]
    pub parent_parent_id: Bytes32,
    #[serde(default)]
    pub parent_inner_puzzle_hash: Bytes32,
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
    /// Named slots with `current.expiration` in `[min_expiration, max_expiration]`,
    /// ordered by `(expiration, handle)`, strictly after `after` when set, at most `limit`.
    /// Memory uses `registrations` for handle strings; SQLite joins and ignores it.
    async fn list_named_in_expiration_window(
        &self,
        registry: Bytes32,
        min_expiration: u64,
        max_expiration: u64,
        after: Option<(u64, String)>,
        limit: usize,
        registrations: &dyn RegistrationStore,
    ) -> Vec<(String, StoredHandleSlot)>;
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

    async fn list_named_in_expiration_window(
        &self,
        registry: Bytes32,
        min_expiration: u64,
        max_expiration: u64,
        after: Option<(u64, String)>,
        limit: usize,
        registrations: &dyn RegistrationStore,
    ) -> Vec<(String, StoredHandleSlot)> {
        if limit == 0 {
            return Vec::new();
        }
        let candidates: Vec<StoredHandleSlot> = self
            .inner
            .read()
            .await
            .values()
            .filter_map(|record| {
                if record.registry_launcher_id != registry {
                    return None;
                }
                let slot = record.current.as_ref()?;
                if slot.expiration < min_expiration || slot.expiration > max_expiration {
                    return None;
                }
                Some(slot.clone())
            })
            .collect();

        let mut out = Vec::new();
        for slot in candidates {
            let Some(reg_rec) = registrations.get(registry, slot.handle_hash).await else {
                continue;
            };
            let Some(reg_cur) = reg_rec.current else {
                continue;
            };
            if reg_cur.handle.is_empty() {
                continue;
            }
            if let Some((exp, handle)) = after.as_ref() {
                if !(slot.expiration > *exp
                    || (slot.expiration == *exp && reg_cur.handle.as_str() > handle.as_str()))
                {
                    continue;
                }
            }
            out.push((reg_cur.handle, slot));
        }
        out.sort_by(|a, b| {
            a.1.expiration
                .cmp(&b.1.expiration)
                .then_with(|| a.0.cmp(&b.0))
        });
        out.truncate(limit);
        out
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
        let expiration = record
            .current
            .as_ref()
            .map(|slot| slot.expiration)
            .unwrap_or(0);
        let db = self.db.lock().await;
        if let Err(e) = db
            .upsert_handle_slot_record_json(
                record.registry_launcher_id,
                record.handle_hash,
                &json,
                expiration,
            )
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

    async fn list_named_in_expiration_window(
        &self,
        registry: Bytes32,
        min_expiration: u64,
        max_expiration: u64,
        after: Option<(u64, String)>,
        limit: usize,
        _registrations: &dyn RegistrationStore,
    ) -> Vec<(String, StoredHandleSlot)> {
        let db = self.db.lock().await;
        let rows = db
            .list_named_handle_slots_in_expiration_window(
                registry,
                min_expiration,
                max_expiration,
                after,
                limit,
            )
            .await
            .unwrap_or_default();
        let mut out = Vec::new();
        for (json, handle) in rows {
            if handle.is_empty() {
                continue;
            }
            let Ok(record) = serde_json::from_str::<HandleSlotRecord>(&json) else {
                continue;
            };
            let Some(slot) = record.current else {
                continue;
            };
            out.push((handle, slot));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::registration_store::{
        MemoryRegistrationStore, RegistrationActionKind, RegistrationRecord, StoredRegistration,
    };
    use super::*;
    use clvm_utils::ToTreeHash;

    fn b32(fill: u8) -> Bytes32 {
        Bytes32::new([fill; 32])
    }

    fn named_slot(registry: Bytes32, handle: &str, expiration: u64) -> StoredHandleSlot {
        StoredHandleSlot {
            registry_launcher_id: registry,
            handle_hash: handle.tree_hash().into(),
            counter: 1,
            neighbors_left: Bytes32::default(),
            neighbors_right: Bytes32::new([0xff; 32]),
            expiration,
            owner_launcher_id: b32(0x11),
            resolved_launcher_id: b32(0x11),
            parent_coin_id: b32(0x31),
            parent_parent_id: b32(0xee),
            parent_inner_puzzle_hash: b32(0xef),
            confirmation_height: 90,
        }
    }

    fn named_reg(registry: Bytes32, handle: &str) -> StoredRegistration {
        StoredRegistration {
            registry_launcher_id: registry,
            handle: handle.to_string(),
            handle_hash: handle.tree_hash().into(),
            registration_secret: b32(0x22),
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 1000,
            confirmation_height: 90,
        }
    }

    async fn upsert_named(
        slots: &MemoryHandleSlotStore,
        regs: &MemoryRegistrationStore,
        registry: Bytes32,
        handle: &str,
        expiration: u64,
    ) {
        let slot = named_slot(registry, handle, expiration);
        slots
            .upsert(HandleSlotRecord {
                registry_launcher_id: registry,
                handle_hash: slot.handle_hash,
                current: Some(slot),
                history: vec![],
            })
            .await;
        let reg = named_reg(registry, handle);
        regs.upsert(RegistrationRecord {
            registry_launcher_id: registry,
            handle_hash: reg.handle_hash,
            current: Some(reg),
            history: vec![],
        })
        .await;
    }

    #[tokio::test]
    async fn lists_named_slots_in_window_ordered_cursor_limited() {
        let registry = b32(0xaa);
        let other = b32(0xbb);
        let slots = MemoryHandleSlotStore::new();
        let regs = MemoryRegistrationStore::new();

        upsert_named(&slots, &regs, registry, "bob", 10).await;
        upsert_named(&slots, &regs, registry, "alice", 20).await;
        upsert_named(&slots, &regs, registry, "carol", 30).await;
        upsert_named(&slots, &regs, registry, "dave", 40).await;
        upsert_named(&slots, &regs, other, "other", 25).await;

        let unnamed = named_slot(registry, "zzzz", 15);
        slots
            .upsert(HandleSlotRecord {
                registry_launcher_id: registry,
                handle_hash: unnamed.handle_hash,
                current: Some(unnamed),
                history: vec![],
            })
            .await;

        let page = slots
            .list_named_in_expiration_window(registry, 10, 30, None, 2, &regs)
            .await;
        assert_eq!(
            page.iter()
                .map(|(h, s)| (h.as_str(), s.expiration))
                .collect::<Vec<_>>(),
            vec![("bob", 10), ("alice", 20)]
        );

        let rest = slots
            .list_named_in_expiration_window(
                registry,
                10,
                30,
                Some((20, "alice".into())),
                10,
                &regs,
            )
            .await;
        assert_eq!(
            rest.iter()
                .map(|(h, s)| (h.as_str(), s.expiration))
                .collect::<Vec<_>>(),
            vec![("carol", 30)]
        );
    }
}

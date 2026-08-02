use std::collections::HashMap;
use std::sync::Arc;

use chia_protocol::{Bytes32, Coin};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::discovery::ParsedNftState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FollowRecordStatus {
    /// Discovery found zero matches; retry on later peaks.
    Incomplete,
    /// Discovery found multiple matches.
    Mismatch,
    /// Actively followed with reconstructable state.
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSingletonState {
    pub launcher_id: Bytes32,
    pub parent_coin_id: Bytes32,
    pub amount: u64,
    pub inner_puzzle_hash: Bytes32,
    pub confirmation_height: u32,
    pub melted: bool,
    pub melt_height: Option<u32>,
    pub nft: Option<ParsedNftState>,
    /// Coin id of the current live (or last melted) singleton coin.
    pub coin_id: Bytes32,
}

impl StoredSingletonState {
    pub fn from_coin(
        launcher_id: Bytes32,
        coin: Coin,
        inner_puzzle_hash: Bytes32,
        confirmation_height: u32,
        melted: bool,
        melt_height: Option<u32>,
        nft: Option<ParsedNftState>,
    ) -> Self {
        Self {
            launcher_id,
            parent_coin_id: coin.parent_coin_info,
            amount: coin.amount,
            inner_puzzle_hash,
            confirmation_height,
            melted,
            melt_height,
            nft,
            coin_id: coin.coin_id(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowedSingleton {
    pub launcher_id: Bytes32,
    pub expected_full_puzzle_hash: Bytes32,
    pub expected_inner_puzzle_hash: Bytes32,
    pub discovery_height: u32,
    pub status: FollowRecordStatus,
    /// Current canonical state when Active (or Incomplete/Mismatch with no state).
    pub current: Option<StoredSingletonState>,
    /// Prior states: every replacement within 32 blocks plus one older predecessor.
    pub history: Vec<StoredSingletonState>,
    /// Number of canonical Handle slots currently referencing this launcher.
    pub reference_count: u32,
    /// Peak height when reference_count last dropped to zero; None while referenced.
    pub dereference_height: Option<u32>,
}

#[async_trait::async_trait]
pub trait SingletonStore: Send + Sync {
    async fn get(&self, launcher_id: Bytes32) -> Option<FollowedSingleton>;
    async fn upsert(&self, record: FollowedSingleton);
    async fn remove(&self, launcher_id: Bytes32);
    async fn all_launcher_ids(&self) -> Vec<Bytes32>;
    async fn bump_reference(&self, launcher_id: Bytes32);
    async fn drop_reference(&self, launcher_id: Bytes32, at_height: u32);
}

#[derive(Default)]
pub struct MemorySingletonStore {
    inner: RwLock<HashMap<Bytes32, FollowedSingleton>>,
}

impl MemorySingletonStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[async_trait::async_trait]
impl SingletonStore for MemorySingletonStore {
    async fn get(&self, launcher_id: Bytes32) -> Option<FollowedSingleton> {
        self.inner.read().await.get(&launcher_id).cloned()
    }

    async fn upsert(&self, record: FollowedSingleton) {
        self.inner.write().await.insert(record.launcher_id, record);
    }

    async fn remove(&self, launcher_id: Bytes32) {
        self.inner.write().await.remove(&launcher_id);
    }

    async fn all_launcher_ids(&self) -> Vec<Bytes32> {
        self.inner.read().await.keys().copied().collect()
    }

    async fn bump_reference(&self, launcher_id: Bytes32) {
        let mut guard = self.inner.write().await;
        if let Some(rec) = guard.get_mut(&launcher_id) {
            rec.reference_count = rec.reference_count.saturating_add(1);
            rec.dereference_height = None;
        }
    }

    async fn drop_reference(&self, launcher_id: Bytes32, at_height: u32) {
        let mut guard = self.inner.write().await;
        if let Some(rec) = guard.get_mut(&launcher_id) {
            rec.reference_count = rec.reference_count.saturating_sub(1);
            if rec.reference_count == 0 {
                rec.dereference_height = Some(at_height);
            }
        }
    }
}

/// Retain current state, every replaced state in the last 32 blocks, and one older predecessor.
pub fn prune_history(history: &mut Vec<StoredSingletonState>, peak: u32) {
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

pub fn push_replacement(
    record: &mut FollowedSingleton,
    new_state: StoredSingletonState,
    peak: u32,
) {
    if let Some(prev) = record.current.take() {
        record.history.push(prev);
    }
    record.current = Some(new_state);
    prune_history(&mut record.history, peak);
}

/// Restore prior state after a pre-final reorganization that orphans `from_height` and above.
pub fn rollback_to_before(record: &mut FollowedSingleton, from_height: u32) {
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

/// SQLite-backed store used by the production `listen` process.
pub struct DbSingletonStore {
    db: Arc<futures::lock::Mutex<crate::Db>>,
}

impl DbSingletonStore {
    pub fn new(db: Arc<futures::lock::Mutex<crate::Db>>) -> Arc<Self> {
        Arc::new(Self { db })
    }
}

#[async_trait::async_trait]
impl SingletonStore for DbSingletonStore {
    async fn get(&self, launcher_id: Bytes32) -> Option<FollowedSingleton> {
        let db = self.db.lock().await;
        let json = db.get_followed_singleton_json(launcher_id).await.ok()??;
        serde_json::from_str(&json).ok()
    }

    async fn upsert(&self, record: FollowedSingleton) {
        let json = match serde_json::to_string(&record) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("followed singleton serialize error: {e}");
                return;
            }
        };
        let db = self.db.lock().await;
        if let Err(e) = db
            .upsert_followed_singleton_json(record.launcher_id, &json)
            .await
        {
            eprintln!("followed singleton upsert error: {e}");
        }
    }

    async fn remove(&self, launcher_id: Bytes32) {
        let db = self.db.lock().await;
        let _ = db.delete_followed_singleton(launcher_id).await;
    }

    async fn all_launcher_ids(&self) -> Vec<Bytes32> {
        let db = self.db.lock().await;
        db.all_followed_singleton_ids().await.unwrap_or_default()
    }

    async fn bump_reference(&self, launcher_id: Bytes32) {
        if let Some(mut rec) = self.get(launcher_id).await {
            rec.reference_count = rec.reference_count.saturating_add(1);
            rec.dereference_height = None;
            self.upsert(rec).await;
        }
    }

    async fn drop_reference(&self, launcher_id: Bytes32, at_height: u32) {
        if let Some(mut rec) = self.get(launcher_id).await {
            rec.reference_count = rec.reference_count.saturating_sub(1);
            if rec.reference_count == 0 {
                rec.dereference_height = Some(at_height);
            }
            self.upsert(rec).await;
        }
    }
}

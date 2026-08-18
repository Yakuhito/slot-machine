use std::sync::Arc;

use chia_protocol::{Bytes32, CoinSpend};
use clvm_utils::ToTreeHash;
use clvmr::Allocator;
use tokio::sync::RwLock;

use super::discovery::{
    discover_singleton_in_block, follow_singleton_spend, DiscoveryResult, FollowSpendResult,
};
use super::freshness::FreshnessState;
use super::handle_store::{
    push_handle_replacement, rollback_handle_to_before, HandleSlotRecord, HandleSlotStore,
    StoredHandleSlot,
};
use super::pending_store::{
    clear_pending_current, push_pending_replacement, rollback_pending_to_before,
    PendingUpdateRecord, PendingUpdateStore, StoredPendingUpdate,
};
use super::refs::{dereferenced_launchers, references_from_action_log, SingletonReference};
use super::registration_store::{
    push_registration_event, push_registration_replacement, rollback_registration_to_before,
    rollback_stats_to_before, RegistrationActionKind, RegistrationRecord, RegistrationStore,
    StoredRegistration, StoredRegistrationEvent,
};
use super::store::{
    push_replacement, rollback_to_before, FollowRecordStatus, FollowedSingleton, SingletonStore,
    StoredSingletonState,
};
use chia_wallet_sdk::driver::XchandlesActionLog;
use chia_wallet_sdk::types::puzzles::XchandlesHandleSlotValue;

/// Applies registry-transition singleton discovery and subsequent lineage follows.
pub struct SingletonIndexer {
    pub store: Arc<dyn SingletonStore>,
    pub handle_slots: Arc<dyn HandleSlotStore>,
    pub registrations: Arc<dyn RegistrationStore>,
    pub pending_updates: Arc<dyn PendingUpdateStore>,
    pub freshness: Arc<RwLock<FreshnessState>>,
}

impl SingletonIndexer {
    pub fn new(
        store: Arc<dyn SingletonStore>,
        handle_slots: Arc<dyn HandleSlotStore>,
        registrations: Arc<dyn RegistrationStore>,
        pending_updates: Arc<dyn PendingUpdateStore>,
        freshness: Arc<RwLock<FreshnessState>>,
    ) -> Self {
        Self {
            store,
            handle_slots,
            registrations,
            pending_updates,
            freshness,
        }
    }

    pub async fn note_peak(
        &self,
        indexed: u32,
        upstream: u32,
        now_unix: u64,
        confirmed_timestamp: u64,
    ) {
        let mut f = self.freshness.write().await;
        f.indexed_peak_height = indexed;
        f.upstream_peak_height = upstream;
        f.last_successful_peak_unix = now_unix;
        f.confirmed_timestamp = confirmed_timestamp;
        f.rolling_back = false;
        f.resyncing = false;
    }

    pub async fn begin_rollback(&self) {
        self.freshness.write().await.rolling_back = true;
    }

    pub async fn begin_resync(&self) {
        self.freshness.write().await.resyncing = true;
    }

    /// Process a registry spend's action logs against the block's coin spends.
    pub async fn on_registry_transition(
        &self,
        allocator: &mut Allocator,
        height: u32,
        block_spends: &[CoinSpend],
        logs: &[XchandlesActionLog],
    ) -> Result<(), String> {
        for log in logs {
            for reference in references_from_action_log(log) {
                self.discover_reference(allocator, height, block_spends, reference)
                    .await?;
            }
            self.track_slot_reference_changes(height, log).await;
        }
        Ok(())
    }

    /// Project a created Handle slot into the unified-proof store.
    pub async fn project_handle_slot(
        &self,
        registry_launcher_id: Bytes32,
        value: XchandlesHandleSlotValue,
        parent_coin_id: Bytes32,
        confirmation_height: u32,
    ) {
        let stored = StoredHandleSlot {
            registry_launcher_id,
            handle_hash: value.handle_hash,
            counter: value.counter,
            neighbors_left: value.neighbors.left_value,
            neighbors_right: value.neighbors.right_value,
            expiration: value.expiration,
            owner_launcher_id: value.owner_launcher_id,
            resolved_launcher_id: value.resolved_launcher_id,
            parent_coin_id,
            confirmation_height,
        };
        let mut record = self
            .handle_slots
            .get(registry_launcher_id, value.handle_hash)
            .await
            .unwrap_or(HandleSlotRecord {
                registry_launcher_id,
                handle_hash: value.handle_hash,
                current: None,
                history: Vec::new(),
            });
        push_handle_replacement(&mut record, stored, confirmation_height);
        self.handle_slots.upsert(record).await;
    }

    /// Project every created Handle slot from action logs.
    pub async fn project_handle_slots_from_logs(
        &self,
        registry_launcher_id: Bytes32,
        height: u32,
        logs: &[XchandlesActionLog],
        parent_for: impl Fn(Bytes32) -> Option<Bytes32>,
    ) {
        let mut created = Vec::new();
        for log in logs {
            log.extend_created_handle_slots(&mut created);
        }
        for value in created {
            let value_hash: Bytes32 = value.tree_hash().into();
            let parent = parent_for(value_hash).unwrap_or_default();
            self.project_handle_slot(registry_launcher_id, value, parent, height)
                .await;
        }
    }

    /// Project register/expire registration facts and recent-event feed from action logs.
    pub async fn project_registrations_from_logs(
        &self,
        registry_launcher_id: Bytes32,
        height: u32,
        logs: &[XchandlesActionLog],
    ) {
        for log in logs {
            let (action_kind, handle, secret, protocol_fee, handle_hash) = match log {
                XchandlesActionLog::Register(e) => (
                    RegistrationActionKind::Register,
                    e.precommit_value.handle.clone(),
                    e.precommit_value.secret,
                    e.total_price,
                    e.created_handle_slot.handle_hash,
                ),
                XchandlesActionLog::Expire(e) => (
                    RegistrationActionKind::Expire,
                    e.precommit_value.handle.clone(),
                    e.precommit_value.secret,
                    e.total_price,
                    e.created_slot.handle_hash,
                ),
                // Extension and other lifecycle actions never replace the registration fact
                // and never change total_registered.
                _ => continue,
            };

            let stored = StoredRegistration {
                registry_launcher_id,
                handle: handle.clone(),
                handle_hash,
                registration_secret: secret,
                action_kind,
                protocol_fee,
                confirmation_height: height,
            };
            let mut record = self
                .registrations
                .get(registry_launcher_id, handle_hash)
                .await
                .unwrap_or(RegistrationRecord {
                    registry_launcher_id,
                    handle_hash,
                    current: None,
                    history: Vec::new(),
                });
            push_registration_replacement(&mut record, stored, height);
            self.registrations.upsert(record).await;

            let mut stats = self.registrations.get_stats(registry_launcher_id).await;
            push_registration_event(
                &mut stats,
                StoredRegistrationEvent {
                    handle,
                    action_kind,
                    confirmation_height: height,
                },
            );
            if action_kind == RegistrationActionKind::Register {
                stats.total_registered = stats.total_registered.saturating_add(1);
            }
            self.registrations
                .set_stats(registry_launcher_id, stats)
                .await;
        }
    }

    /// Project InitiateUpdate creations and clear pending on execute/invalidate.
    pub async fn project_pending_updates_from_logs(
        &self,
        registry_launcher_id: Bytes32,
        height: u32,
        logs: &[XchandlesActionLog],
    ) {
        for log in logs {
            match log {
                XchandlesActionLog::InitiateUpdate(e) => {
                    let value = e.created_update_slot;
                    let stored = StoredPendingUpdate {
                        registry_launcher_id,
                        handle_hash: value.handle_hash,
                        new_owner_launcher_id: value.new_owner_launcher_id,
                        new_resolved_launcher_id: value.new_resolved_launcher_id,
                        update_confirmation_height: height,
                        minimum_execution_height: value.min_height,
                        update_initiator_coin_id: value.update_initiator_coin_id,
                    };
                    let mut record = self
                        .pending_updates
                        .get(registry_launcher_id, value.handle_hash)
                        .await
                        .unwrap_or(PendingUpdateRecord {
                            registry_launcher_id,
                            handle_hash: value.handle_hash,
                            current: None,
                            history: Vec::new(),
                        });
                    push_pending_replacement(&mut record, stored, height);
                    self.pending_updates.upsert(record).await;
                }
                XchandlesActionLog::ExecuteUpdate(e) => {
                    self.clear_pending_for_handle(
                        registry_launcher_id,
                        e.spent_update_slot.handle_hash,
                        height,
                    )
                    .await;
                }
                // Any other action that spends this Handle slot without creating a
                // replacement update invalidates the pending executor path.
                XchandlesActionLog::Extend(e) => {
                    self.clear_pending_for_handle(
                        registry_launcher_id,
                        e.spent_slot.handle_hash,
                        height,
                    )
                    .await;
                }
                XchandlesActionLog::Oracle(e) => {
                    self.clear_pending_for_handle(
                        registry_launcher_id,
                        e.spent_slot.handle_hash,
                        height,
                    )
                    .await;
                }
                XchandlesActionLog::Expire(e) => {
                    self.clear_pending_for_handle(
                        registry_launcher_id,
                        e.spent_slot.handle_hash,
                        height,
                    )
                    .await;
                }
                XchandlesActionLog::Refund(e) => {
                    if let Some(spent) = e.spent_slot {
                        self.clear_pending_for_handle(
                            registry_launcher_id,
                            spent.handle_hash,
                            height,
                        )
                        .await;
                    }
                }
                XchandlesActionLog::Register(_) | XchandlesActionLog::DelegatedState(_) => {}
            }
        }
    }

    async fn clear_pending_for_handle(
        &self,
        registry_launcher_id: Bytes32,
        handle_hash: Bytes32,
        height: u32,
    ) {
        let Some(mut record) = self
            .pending_updates
            .get(registry_launcher_id, handle_hash)
            .await
        else {
            return;
        };
        if record.current.is_none() {
            return;
        }
        clear_pending_current(&mut record, height);
        if record.current.is_none() && record.history.is_empty() {
            self.pending_updates
                .remove(registry_launcher_id, handle_hash)
                .await;
        } else {
            self.pending_updates.upsert(record).await;
        }
    }

    async fn discover_reference(
        &self,
        allocator: &mut Allocator,
        height: u32,
        block_spends: &[CoinSpend],
        reference: SingletonReference,
    ) -> Result<(), String> {
        // Re-reference after cleanup: rediscover from this block.
        let existing = self.store.get(reference.launcher_id).await;
        if let Some(rec) = &existing {
            if rec.status == FollowRecordStatus::Active && rec.current.is_some() {
                self.store.bump_reference(reference.launcher_id).await;
                return Ok(());
            }
        }

        let result = discover_singleton_in_block(
            allocator,
            block_spends,
            reference.launcher_id,
            reference.expected_full_puzzle_hash,
            reference.expected_inner_puzzle_hash,
        )
        .map_err(|e| e.to_string())?;

        let (status, current) = match result {
            DiscoveryResult::Incomplete => (FollowRecordStatus::Incomplete, None),
            DiscoveryResult::Mismatch => (FollowRecordStatus::Mismatch, None),
            DiscoveryResult::Found(found) => {
                let melted = found.melted;
                let state = StoredSingletonState::from_coin(
                    reference.launcher_id,
                    found.coin,
                    found.inner_puzzle_hash,
                    height,
                    melted,
                    if melted { Some(height) } else { None },
                    found.nft,
                );
                (FollowRecordStatus::Active, Some(state))
            }
        };

        let mut reference_count = 1;
        let mut history = Vec::new();
        if let Some(prev) = existing {
            reference_count = prev.reference_count.max(1);
            history = prev.history;
        }

        self.store
            .upsert(FollowedSingleton {
                launcher_id: reference.launcher_id,
                expected_full_puzzle_hash: reference.expected_full_puzzle_hash,
                expected_inner_puzzle_hash: reference.expected_inner_puzzle_hash,
                discovery_height: height,
                status,
                current,
                history,
                reference_count,
                dereference_height: None,
            })
            .await;
        Ok(())
    }

    async fn track_slot_reference_changes(&self, height: u32, log: &XchandlesActionLog) {
        let pairs: Vec<(XchandlesHandleSlotValue, XchandlesHandleSlotValue)> = match log {
            XchandlesActionLog::Extend(e) => vec![(e.spent_slot, e.created_slot)],
            XchandlesActionLog::Oracle(e) => vec![(e.spent_slot, e.created_slot)],
            XchandlesActionLog::Expire(e) => vec![(e.spent_slot, e.created_slot)],
            XchandlesActionLog::ExecuteUpdate(e) => {
                vec![(e.spent_handle_slot, e.created_slot)]
            }
            XchandlesActionLog::InitiateUpdate(e) => {
                vec![(e.spent_slot, e.created_handle_slot)]
            }
            XchandlesActionLog::Refund(e) => match (e.spent_slot, e.created_slot) {
                (Some(s), Some(c)) => vec![(s, c)],
                _ => Vec::new(),
            },
            // New handle is created, not a replacement of either neighbor.
            // Pairing spent_left with created_handle would falsely drop the
            // left neighbor's Owner/Resolved NFT follow.
            XchandlesActionLog::Register(e) => vec![
                (e.spent_left_slot, e.created_left_slot),
                (e.spent_right_slot, e.created_right_slot),
            ],
            XchandlesActionLog::DelegatedState(_) => Vec::new(),
        };

        for (spent, created) in pairs {
            for launcher in dereferenced_launchers(&spent, &created) {
                self.store.drop_reference(launcher, height).await;
            }
            for launcher in [created.owner_launcher_id, created.resolved_launcher_id] {
                if let Some(rec) = self.store.get(launcher).await {
                    if rec.reference_count == 0 || rec.dereference_height.is_some() {
                        self.store.bump_reference(launcher).await;
                    }
                }
            }
        }
    }

    /// Retry incomplete discoveries and follow live coins that spent in this block.
    pub async fn on_block(
        &self,
        allocator: &mut Allocator,
        height: u32,
        block_spends: &[CoinSpend],
    ) -> Result<(), String> {
        let ids = self.store.all_launcher_ids().await;
        for launcher_id in ids {
            let Some(mut record) = self.store.get(launcher_id).await else {
                continue;
            };

            if record.status == FollowRecordStatus::Incomplete {
                let result = discover_singleton_in_block(
                    allocator,
                    block_spends,
                    record.launcher_id,
                    record.expected_full_puzzle_hash,
                    record.expected_inner_puzzle_hash,
                )
                .map_err(|e| e.to_string())?;
                match result {
                    DiscoveryResult::Found(found) => {
                        let melted = found.melted;
                        record.status = FollowRecordStatus::Active;
                        record.current = Some(StoredSingletonState::from_coin(
                            record.launcher_id,
                            found.coin,
                            found.inner_puzzle_hash,
                            height,
                            melted,
                            if melted { Some(height) } else { None },
                            found.nft,
                        ));
                        self.store.upsert(record).await;
                    }
                    DiscoveryResult::Mismatch => {
                        record.status = FollowRecordStatus::Mismatch;
                        self.store.upsert(record).await;
                    }
                    DiscoveryResult::Incomplete => {}
                }
                continue;
            }

            if record.status != FollowRecordStatus::Active {
                continue;
            }
            let Some(current) = record.current.clone() else {
                continue;
            };
            if current.melted {
                continue;
            }

            let Some(spend) = block_spends
                .iter()
                .find(|s| s.coin.coin_id() == current.coin_id)
            else {
                continue;
            };

            match follow_singleton_spend(allocator, spend, launcher_id)
                .map_err(|e| e.to_string())?
            {
                FollowSpendResult::Next(next) => {
                    let new_state = StoredSingletonState::from_coin(
                        launcher_id,
                        next.coin,
                        next.inner_puzzle_hash,
                        height,
                        false,
                        None,
                        next.nft,
                    );
                    push_replacement(&mut record, new_state, height);
                    self.store.upsert(record).await;
                }
                FollowSpendResult::Melted {
                    last_coin,
                    inner_puzzle_hash,
                    nft,
                } => {
                    let new_state = StoredSingletonState::from_coin(
                        launcher_id,
                        last_coin,
                        inner_puzzle_hash,
                        height,
                        true,
                        Some(height),
                        nft,
                    );
                    push_replacement(&mut record, new_state, height);
                    self.store.upsert(record).await;
                }
            }
        }

        self.cleanup_finalized(height).await;
        Ok(())
    }

    async fn cleanup_finalized(&self, peak: u32) {
        for launcher_id in self.store.all_launcher_ids().await {
            let Some(rec) = self.store.get(launcher_id).await else {
                continue;
            };
            if rec.reference_count == 0 {
                if let Some(deref_h) = rec.dereference_height {
                    if peak >= deref_h.saturating_add(32) {
                        self.store.remove(launcher_id).await;
                    }
                }
            }
        }
    }

    /// Pre-final reorganization: restore lineage and Handle-slot state confirmed before `from_height`.
    pub async fn rollback(&self, from_height: u32) {
        self.begin_rollback().await;
        for launcher_id in self.store.all_launcher_ids().await {
            let Some(mut rec) = self.store.get(launcher_id).await else {
                continue;
            };
            if rec.discovery_height >= from_height {
                self.store.remove(launcher_id).await;
                continue;
            }
            rollback_to_before(&mut rec, from_height);
            if let Some(deref_h) = rec.dereference_height {
                if deref_h >= from_height {
                    rec.dereference_height = None;
                    if rec.reference_count == 0 {
                        rec.reference_count = 1;
                    }
                }
            }
            self.store.upsert(rec).await;
        }

        for (registry, handle_hash) in self.handle_slots.all_keys().await {
            let Some(mut rec) = self.handle_slots.get(registry, handle_hash).await else {
                continue;
            };
            rollback_handle_to_before(&mut rec, from_height);
            if rec.current.is_none() && rec.history.is_empty() {
                self.handle_slots.remove(registry, handle_hash).await;
            } else {
                self.handle_slots.upsert(rec).await;
            }
        }

        let mut touched_registries: std::collections::HashSet<Bytes32> = self
            .registrations
            .all_stats_registry_ids()
            .await
            .into_iter()
            .collect();
        for (registry, handle_hash) in self.registrations.all_keys().await {
            touched_registries.insert(registry);
            let Some(mut rec) = self.registrations.get(registry, handle_hash).await else {
                continue;
            };
            rollback_registration_to_before(&mut rec, from_height);
            if rec.current.is_none() && rec.history.is_empty() {
                self.registrations.remove(registry, handle_hash).await;
            } else {
                self.registrations.upsert(rec).await;
            }
        }
        for registry in touched_registries {
            let mut stats = self.registrations.get_stats(registry).await;
            rollback_stats_to_before(&mut stats, from_height);
            self.registrations.set_stats(registry, stats).await;
        }

        for (registry, handle_hash) in self.pending_updates.all_keys().await {
            let Some(mut rec) = self.pending_updates.get(registry, handle_hash).await else {
                continue;
            };
            rollback_pending_to_before(&mut rec, from_height);
            if rec.current.is_none() && rec.history.is_empty() {
                self.pending_updates.remove(registry, handle_hash).await;
            } else {
                self.pending_updates.upsert(rec).await;
            }
        }
    }
}

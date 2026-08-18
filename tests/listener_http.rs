//! Real-HTTP listener fixture and golden contracts for Tickets 10-14.
//!
//! Later tickets must extend this same server fixture and consume these goldens
//! rather than redefining the singleton/error envelope independently.
//!
//! Seams under test for Ticket 14:
//! - `GET /expiring?view=active|soon` HTTP contract (success, pagination, errors)
//! - Auction premium / membership against confirmed timestamp + 420s
//! - Directory membership under fork rollback

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chia_protocol::Bytes32;
use chia_puzzle_types::singleton::SingletonArgs;
use chia_wallet_sdk::driver::{Launcher, SingletonInfo, SpendContext, StandardLayer};
use chia_wallet_sdk::test::{BlsPair, Simulator};
use chia_wallet_sdk::types::Conditions;
use clvm_utils::ToTreeHash;
use clvmr::Allocator;
use serde_json::Value;
use slot_machine::{
    discover_singleton_in_block, listener_router, push_handle_replacement,
    push_pending_replacement, push_registration_replacement, push_replacement, rollback_to_before,
    DiscoveryResult, FollowRecordStatus, FollowedSingleton, FreshnessState, HandleSlotRecord,
    HandleSlotStore, ListenerApiState, MemoryHandleSlotStore, MemoryPendingUpdateStore,
    MemoryRegistrationStore, MemorySingletonStore, ParsedNftState, PendingUpdateRecord,
    PendingUpdateStore, RegistrationActionKind, RegistrationRecord, RegistrationStore,
    RegistryPricing, RegistryRegistrationStats, SingletonIndexer, SingletonStore, StoredHandleSlot,
    StoredPendingUpdate, StoredRegistration, StoredRegistrationEvent, StoredSingletonState,
};
use tokio::sync::RwLock;

fn load_golden(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/listener/goldens/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).expect("parse golden")
}

fn normalize_request_id(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        if obj.contains_key("request_id") {
            obj.insert(
                "request_id".into(),
                Value::String("FIXED_REQUEST_ID".into()),
            );
        }
    }
    v
}

struct RunningListener {
    base: String,
    store: Arc<MemorySingletonStore>,
    handle_slots: Arc<MemoryHandleSlotStore>,
    registrations: Arc<MemoryRegistrationStore>,
    pending_updates: Arc<MemoryPendingUpdateStore>,
    freshness: Arc<RwLock<FreshnessState>>,
    #[allow(dead_code)] // available for pricing-override tests
    registry_pricing: Arc<RwLock<HashMap<Bytes32, RegistryPricing>>>,
    _join: tokio::task::JoinHandle<()>,
}

impl RunningListener {
    async fn spawn(freshness: FreshnessState) -> Self {
        Self::spawn_with_registries(freshness, vec![b32(0xaa)], None).await
    }

    async fn spawn_with_registries(
        freshness: FreshnessState,
        registry_launcher_ids: Vec<Bytes32>,
        now_unix_override: Option<u64>,
    ) -> Self {
        let store = MemorySingletonStore::shared();
        let handle_slots = MemoryHandleSlotStore::shared();
        let registrations = MemoryRegistrationStore::shared();
        let pending_updates = MemoryPendingUpdateStore::shared();
        let freshness = Arc::new(RwLock::new(freshness));
        let mut pricing_map = HashMap::new();
        for id in &registry_launcher_ids {
            pricing_map.insert(
                *id,
                RegistryPricing {
                    base_price: 5_000,
                    registration_period: 31_557_600,
                },
            );
        }
        let registry_pricing = Arc::new(RwLock::new(pricing_map));
        let state = ListenerApiState {
            store: store.clone() as Arc<dyn SingletonStore>,
            handle_slots: handle_slots.clone() as Arc<dyn HandleSlotStore>,
            registrations: registrations.clone() as Arc<dyn RegistrationStore>,
            pending_updates: pending_updates.clone() as Arc<dyn PendingUpdateStore>,
            freshness: Arc::clone(&freshness),
            registry_pricing: Arc::clone(&registry_pricing),
            registry_launcher_ids,
            now_unix_override,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = listener_router(state);
        let join = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base = format!("http://{addr}");
        for _ in 0..100 {
            if reqwest::get(format!("{base}/healthz")).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            base,
            store,
            handle_slots,
            registrations,
            pending_updates,
            freshness,
            registry_pricing,
            _join: join,
        }
    }
}

fn b32(byte: u8) -> Bytes32 {
    Bytes32::new([byte; 32])
}

fn active_record(launcher: Bytes32, state: StoredSingletonState) -> FollowedSingleton {
    FollowedSingleton {
        launcher_id: launcher,
        expected_full_puzzle_hash: b32(0xab),
        expected_inner_puzzle_hash: state.inner_puzzle_hash,
        discovery_height: state.confirmation_height,
        status: FollowRecordStatus::Active,
        current: Some(state),
        history: Vec::new(),
        reference_count: 1,
        dereference_height: None,
    }
}

async fn upsert_handle_slot(server: &RunningListener, slot: StoredHandleSlot) {
    server
        .handle_slots
        .upsert(HandleSlotRecord {
            registry_launcher_id: slot.registry_launcher_id,
            handle_hash: slot.handle_hash,
            current: Some(slot),
            history: Vec::new(),
        })
        .await;
}

async fn upsert_registration(server: &RunningListener, reg: StoredRegistration) {
    let mut record = RegistrationRecord {
        registry_launcher_id: reg.registry_launcher_id,
        handle_hash: reg.handle_hash,
        current: None,
        history: Vec::new(),
    };
    let height = reg.confirmation_height;
    push_registration_replacement(&mut record, reg, height);
    server.registrations.upsert(record).await;
}

#[tokio::test]
async fn real_http_golden_singleton_shapes_and_errors() {
    let server =
        RunningListener::spawn(FreshnessState::fresh_at(116, FreshnessState::now_unix())).await;
    let client = reqwest::Client::new();

    let launcher = b32(0x11);
    let nft_state = StoredSingletonState {
        launcher_id: launcher,
        parent_coin_id: b32(0x22),
        amount: 1,
        inner_puzzle_hash: b32(0x33),
        confirmation_height: 100,
        melted: false,
        melt_height: None,
        nft: Some(ParsedNftState {
            metadata_treehash: b32(0x44),
            metadata_updater_puzzle_hash: b32(0x55),
            current_owner: None,
            royalty_puzzle_hash: b32(0x66),
            royalty_basis_points: 420,
            p2_puzzle_hash: b32(0x77),
            metadata_clvm: Some(vec![0x80]),
        }),
        coin_id: b32(0x88),
    };
    server
        .store
        .upsert(active_record(launcher, nft_state))
        .await;

    let resp = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(launcher)
        ))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap(),
        "*"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("singleton_nft_success.json"));
    assert!(body.get("nft").unwrap().get("metadata").is_none());

    let with_meta: Value = client
        .get(format!(
            "{}/singletons/{}?include_metadata=true",
            server.base,
            hex::encode(launcher)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(with_meta["nft"]["metadata"].as_str().unwrap(), "80");

    *server.freshness.write().await = FreshnessState::fresh_at(60, FreshnessState::now_unix());
    let launcher = b32(0xaa);
    server
        .store
        .upsert(active_record(
            launcher,
            StoredSingletonState {
                launcher_id: launcher,
                parent_coin_id: b32(0xbb),
                amount: 1,
                inner_puzzle_hash: b32(0xcc),
                confirmation_height: 50,
                melted: false,
                melt_height: None,
                nft: None,
                coin_id: b32(0x99),
            },
        ))
        .await;
    let body: Value = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(launcher)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body, load_golden("singleton_non_nft_success.json"));

    *server.freshness.write().await = FreshnessState::fresh_at(50, FreshnessState::now_unix());
    let launcher = b32(0xdd);
    server
        .store
        .upsert(active_record(
            launcher,
            StoredSingletonState {
                launcher_id: launcher,
                parent_coin_id: b32(0xee),
                amount: 1,
                inner_puzzle_hash: b32(0xff),
                confirmation_height: 10,
                melted: true,
                melt_height: Some(42),
                nft: None,
                coin_id: b32(0x01),
            },
        ))
        .await;
    let body: Value = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(launcher)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body, load_golden("singleton_melted_success.json"));

    let err = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(b32(0x00))
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 404);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_singleton_not_followed.json")
    );

    let launcher = b32(0x12);
    server
        .store
        .upsert(FollowedSingleton {
            launcher_id: launcher,
            expected_full_puzzle_hash: b32(0x13),
            expected_inner_puzzle_hash: b32(0x14),
            discovery_height: 1,
            status: FollowRecordStatus::Incomplete,
            current: None,
            history: Vec::new(),
            reference_count: 1,
            dereference_height: None,
        })
        .await;
    let err = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(launcher)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 503);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_singleton_incomplete.json")
    );

    server
        .store
        .upsert(FollowedSingleton {
            launcher_id: launcher,
            expected_full_puzzle_hash: b32(0x13),
            expected_inner_puzzle_hash: b32(0x14),
            discovery_height: 1,
            status: FollowRecordStatus::Mismatch,
            current: None,
            history: Vec::new(),
            reference_count: 1,
            dereference_height: None,
        })
        .await;
    let err = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(launcher)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 503);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_singleton_mismatch.json")
    );

    *server.freshness.write().await = FreshnessState {
        indexed_peak_height: 100,
        upstream_peak_height: 200,
        last_successful_peak_unix: FreshnessState::now_unix(),
        confirmed_timestamp: FreshnessState::now_unix(),
        rolling_back: false,
        resyncing: false,
    };
    let err = client
        .get(format!(
            "{}/singletons/{}",
            server.base,
            hex::encode(launcher)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 503);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_index_stale.json")
    );

    let err = client
        .get(format!("{}/singletons/not-a-launcher", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 400);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_invalid_launcher_id.json")
    );
}

#[tokio::test]
async fn discovery_follow_melt_rollback_cleanup_and_rediscovery() -> anyhow::Result<()> {
    let mut sim = Simulator::new();
    let mut ctx = SpendContext::new();
    let bls = BlsPair::default();
    let p2 = StandardLayer::new(bls.pk);
    let launcher_coin = sim.new_coin(chia_puzzles::SINGLETON_LAUNCHER_HASH.into(), 1);
    let launcher = Launcher::new(launcher_coin.parent_coin_info, 1);
    let (_, did) = launcher.create_simple_did(&mut ctx, &p2)?;
    sim.spend_coins(ctx.take(), std::slice::from_ref(&bls.sk))?;

    let launcher_id = did.info.launcher_id;
    let inner: Bytes32 = did.info.inner_puzzle_hash().into();
    let full: Bytes32 = SingletonArgs::curry_tree_hash(launcher_id, inner.into()).into();
    assert_eq!(did.coin.puzzle_hash, full);

    let mut ctx = SpendContext::new();
    let new_did = did.update(&mut ctx, &p2, Conditions::new())?;
    let spends = ctx.take();
    let discovery_spend = spends
        .iter()
        .find(|s| s.coin.coin_id() == did.coin.coin_id())
        .cloned()
        .expect("did spend");
    sim.spend_coins(spends, std::slice::from_ref(&bls.sk))?;

    let mut allocator = Allocator::new();
    let found = discover_singleton_in_block(
        &mut allocator,
        std::slice::from_ref(&discovery_spend),
        launcher_id,
        full,
        inner,
    )?;
    let DiscoveryResult::Found(discovered) = found else {
        panic!("expected Found, got {found:?}");
    };
    assert!(!discovered.melted);
    assert_eq!(discovered.coin.coin_id(), new_did.coin.coin_id());
    assert!(discovered.nft.is_none(), "DID is a non-NFT singleton");

    let store = MemorySingletonStore::shared();
    let freshness = Arc::new(RwLock::new(FreshnessState::fresh_at(
        10,
        FreshnessState::now_unix(),
    )));
    let indexer = SingletonIndexer::new(
        store.clone() as Arc<dyn SingletonStore>,
        MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
        MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        Arc::clone(&freshness),
    );

    store
        .upsert(FollowedSingleton {
            launcher_id,
            expected_full_puzzle_hash: full,
            expected_inner_puzzle_hash: inner,
            discovery_height: 5,
            status: FollowRecordStatus::Incomplete,
            current: None,
            history: Vec::new(),
            reference_count: 1,
            dereference_height: None,
        })
        .await;

    let mut allocator = Allocator::new();
    indexer
        .on_block(&mut allocator, 5, std::slice::from_ref(&discovery_spend))
        .await
        .unwrap();
    let rec = store.get(launcher_id).await.unwrap();
    assert_eq!(rec.status, FollowRecordStatus::Active);

    let mut ctx = SpendContext::new();
    let after = new_did.update(&mut ctx, &p2, Conditions::new())?;
    let spends = ctx.take();
    let follow_spend = spends
        .iter()
        .find(|s| s.coin.coin_id() == new_did.coin.coin_id())
        .cloned()
        .unwrap();
    sim.spend_coins(spends, std::slice::from_ref(&bls.sk))?;

    let mut allocator = Allocator::new();
    indexer
        .on_block(&mut allocator, 8, &[follow_spend])
        .await
        .unwrap();
    let rec = store.get(launcher_id).await.unwrap();
    assert_eq!(rec.current.as_ref().unwrap().coin_id, after.coin.coin_id());
    assert_eq!(rec.history.len(), 1);

    // Melt terminal state + rollback restoration
    let mut rec = store.get(launcher_id).await.unwrap();
    let mut melted_state = rec.current.clone().unwrap();
    melted_state.confirmation_height = 9;
    melted_state.melted = true;
    melted_state.melt_height = Some(9);
    push_replacement(&mut rec, melted_state, 9);
    store.upsert(rec).await;

    let mut rec = store.get(launcher_id).await.unwrap();
    rollback_to_before(&mut rec, 9);
    store.upsert(rec).await;
    let rec = store.get(launcher_id).await.unwrap();
    assert!(!rec.current.as_ref().unwrap().melted);

    // Dereference finality cleanup
    store.drop_reference(launcher_id, 10).await;
    for launcher in store.all_launcher_ids().await {
        let rec = store.get(launcher).await.unwrap();
        if rec.reference_count == 0 {
            if let Some(deref_h) = rec.dereference_height {
                if 42 >= deref_h.saturating_add(32) {
                    store.remove(launcher).await;
                }
            }
        }
    }
    assert!(store.get(launcher_id).await.is_none());

    // Rediscovery after cleanup (new reference in a later block)
    let mut allocator = Allocator::new();
    let again = discover_singleton_in_block(
        &mut allocator,
        std::slice::from_ref(&discovery_spend),
        launcher_id,
        full,
        inner,
    )?;
    assert!(matches!(again, DiscoveryResult::Found(_)));

    // Multiple matches → integrity failure
    let mut allocator = Allocator::new();
    let dup = discover_singleton_in_block(
        &mut allocator,
        &[discovery_spend.clone(), discovery_spend],
        launcher_id,
        full,
        inner,
    )?;
    assert_eq!(dup, DiscoveryResult::Mismatch);

    // Forked-chain style: indexer.rollback removes post-fork discoveries
    store
        .upsert(FollowedSingleton {
            launcher_id,
            expected_full_puzzle_hash: full,
            expected_inner_puzzle_hash: inner,
            discovery_height: 20,
            status: FollowRecordStatus::Active,
            current: Some(StoredSingletonState::from_coin(
                launcher_id,
                after.coin,
                inner,
                20,
                false,
                None,
                None,
            )),
            history: Vec::new(),
            reference_count: 1,
            dereference_height: None,
        })
        .await;
    indexer.rollback(15).await;
    assert!(store.get(launcher_id).await.is_none());

    Ok(())
}

#[tokio::test]
async fn real_http_golden_handle_proofs_and_errors() {
    let registry = b32(0xaa);
    let secondary = b32(0xbb);
    let server = RunningListener::spawn_with_registries(
        FreshnessState::fresh_at(116, FreshnessState::now_unix()),
        vec![registry, secondary],
        Some(1_700_000_000),
    )
    .await;
    let client = reqwest::Client::new();

    // Strict Handle syntax - no normalization.
    for bad in ["ab", "ABC", "@alice", " alice", "alice!", "%40alice"] {
        let err = client
            .get(format!("{}/handle/{}", server.base, bad))
            .send()
            .await
            .unwrap();
        assert_eq!(err.status(), 400, "path={bad}");
        assert_eq!(
            normalize_request_id(err.json().await.unwrap()),
            load_golden("error_invalid_handle.json"),
            "path={bad}"
        );
    }

    // Percent-decode exactly once: %61lice → "alice" (valid).
    let unknown = client
        .get(format!("{}/handle/%61lice", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    assert_eq!(
        normalize_request_id(unknown.json().await.unwrap()),
        load_golden("error_handle_not_found.json")
    );

    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let resolved = b32(0x11);
    let owner = b32(0xcc);

    let nft_state = StoredSingletonState {
        launcher_id: resolved,
        parent_coin_id: b32(0x22),
        amount: 1,
        inner_puzzle_hash: b32(0x33),
        confirmation_height: 100,
        melted: false,
        melt_height: None,
        nft: Some(ParsedNftState {
            metadata_treehash: b32(0x44),
            metadata_updater_puzzle_hash: b32(0x55),
            current_owner: None,
            royalty_puzzle_hash: b32(0x66),
            royalty_basis_points: 420,
            p2_puzzle_hash: b32(0x77),
            metadata_clvm: Some(vec![0x80]),
        }),
        coin_id: b32(0x88),
    };
    server
        .store
        .upsert(active_record(resolved, nft_state))
        .await;

    // Distinct incomplete Owner must not break Resolved proof.
    server
        .store
        .upsert(FollowedSingleton {
            launcher_id: owner,
            expected_full_puzzle_hash: b32(0x13),
            expected_inner_puzzle_hash: b32(0x14),
            discovery_height: 1,
            status: FollowRecordStatus::Incomplete,
            current: None,
            history: Vec::new(),
            reference_count: 1,
            dereference_height: None,
        })
        .await;

    upsert_handle_slot(
        &server,
        StoredHandleSlot {
            registry_launcher_id: registry,
            handle_hash,
            counter: 0,
            neighbors_left: Bytes32::default(),
            neighbors_right: Bytes32::new([0xff; 32]),
            expiration: 4_102_444_800,
            owner_launcher_id: owner,
            resolved_launcher_id: resolved,
            parent_coin_id: b32(0xdd),
            confirmation_height: 90,
        },
    )
    .await;

    let resp = client
        .get(format!("{}/handle/{handle}", server.base))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap(),
        "*"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("handle_nft_success.json"));
    assert!(body["resolved_singleton"]["nft"]
        .as_object()
        .unwrap()
        .get("metadata")
        .is_none());

    let with_meta: Value = client
        .get(format!(
            "{}/handle/{handle}?include_metadata=true",
            server.base
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        with_meta["resolved_singleton"]["nft"]["metadata"]
            .as_str()
            .unwrap(),
        "80"
    );
    assert!(with_meta["resolved_singleton"]["nft"]["metadata_treehash"].is_string());

    // Default registry = first configured; explicit never falls back.
    let explicit_missing = client
        .get(format!(
            "{}/handle/{handle}?launcher_id={}",
            server.base,
            hex::encode(b32(0xee))
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(explicit_missing.status(), 404);
    assert_eq!(
        normalize_request_id(explicit_missing.json().await.unwrap()),
        load_golden("error_registry_not_followed.json")
    );

    let bad_launcher = client
        .get(format!(
            "{}/handle/{handle}?launcher_id=not-a-launcher",
            server.base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_launcher.status(), 400);
    assert_eq!(
        normalize_request_id(bad_launcher.json().await.unwrap()),
        load_golden("error_invalid_launcher_id.json")
    );

    // Secondary registry has no slot for alice → handle_not_found (no fallback).
    let no_fallback = client
        .get(format!(
            "{}/handle/{handle}?launcher_id={}",
            server.base,
            hex::encode(secondary)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(no_fallback.status(), 404);
    assert_eq!(
        normalize_request_id(no_fallback.json().await.unwrap()),
        load_golden("error_handle_not_found.json")
    );

    // Missing Resolved → resolution_incomplete
    server.store.remove(resolved).await;
    let err = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 503);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_resolution_incomplete.json")
    );

    // Mismatch Resolved → resolution_mismatch
    server
        .store
        .upsert(FollowedSingleton {
            launcher_id: resolved,
            expected_full_puzzle_hash: b32(0x13),
            expected_inner_puzzle_hash: b32(0x14),
            discovery_height: 1,
            status: FollowRecordStatus::Mismatch,
            current: None,
            history: Vec::new(),
            reference_count: 1,
            dereference_height: None,
        })
        .await;
    let err = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 503);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_resolution_mismatch.json")
    );

    // Non-NFT Resolved → 200 without p2 address fields (nft null)
    server
        .store
        .upsert(active_record(
            resolved,
            StoredSingletonState {
                launcher_id: resolved,
                parent_coin_id: b32(0x22),
                amount: 1,
                inner_puzzle_hash: b32(0x33),
                confirmation_height: 100,
                melted: false,
                melt_height: None,
                nft: None,
                coin_id: b32(0x88),
            },
        ))
        .await;
    let body: Value = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["resolved_singleton"]["nft"], Value::Null);
    assert_eq!(body["resolved_singleton"]["melted"], Value::Bool(false));

    // Melted Resolved → 200
    server
        .store
        .upsert(active_record(
            resolved,
            StoredSingletonState {
                launcher_id: resolved,
                parent_coin_id: b32(0x22),
                amount: 1,
                inner_puzzle_hash: b32(0x33),
                confirmation_height: 100,
                melted: true,
                melt_height: Some(101),
                nft: None,
                coin_id: b32(0x88),
            },
        ))
        .await;
    let body: Value = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["resolved_singleton"]["melted"], Value::Bool(true));

    // Expiration: expired without bypass → handle_expired only
    upsert_handle_slot(
        &server,
        StoredHandleSlot {
            registry_launcher_id: registry,
            handle_hash,
            counter: 0,
            neighbors_left: Bytes32::default(),
            neighbors_right: Bytes32::new([0xff; 32]),
            expiration: 1000,
            owner_launcher_id: owner,
            resolved_launcher_id: resolved,
            parent_coin_id: b32(0xdd),
            confirmation_height: 90,
        },
    )
    .await;
    let err = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 410);
    let err_body = normalize_request_id(err.json().await.unwrap());
    assert_eq!(err_body, load_golden("error_handle_expired.json"));
    assert!(err_body.get("slot").is_none());
    assert!(err_body.get("resolved_singleton").is_none());

    // Bypass returns complete expired proof.
    let bypass: Value = client
        .get(format!(
            "{}/handle/{handle}?bypass_expiration_safety_check=true",
            server.base
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bypass["slot"]["expiration"].as_u64().unwrap(), 1000);
    assert!(bypass.get("resolved_singleton").is_some());

    // Stale index
    *server.freshness.write().await = FreshnessState {
        indexed_peak_height: 100,
        upstream_peak_height: 200,
        last_successful_peak_unix: FreshnessState::now_unix(),
        confirmed_timestamp: FreshnessState::now_unix(),
        rolling_back: false,
        resyncing: false,
    };
    let err = client
        .get(format!(
            "{}/handle/{handle}?bypass_expiration_safety_check=true",
            server.base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(err.status(), 503);
    assert_eq!(
        normalize_request_id(err.json().await.unwrap()),
        load_golden("error_index_stale.json")
    );
}

#[tokio::test]
async fn handle_proof_restores_prior_slot_after_reorganization() {
    let registry = b32(0xaa);
    let handle = "bob1";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let resolved_a = b32(0x11);
    let resolved_b = b32(0x22);

    let handle_slots = MemoryHandleSlotStore::shared();
    let store = MemorySingletonStore::shared();
    let freshness = Arc::new(RwLock::new(FreshnessState::fresh_at(
        50,
        FreshnessState::now_unix(),
    )));
    let indexer = SingletonIndexer::new(
        store.clone() as Arc<dyn SingletonStore>,
        handle_slots.clone() as Arc<dyn HandleSlotStore>,
        MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        Arc::clone(&freshness),
    );

    let slot_a = StoredHandleSlot {
        registry_launcher_id: registry,
        handle_hash,
        counter: 0,
        neighbors_left: Bytes32::default(),
        neighbors_right: Bytes32::new([0xff; 32]),
        expiration: 4_102_444_800,
        owner_launcher_id: resolved_a,
        resolved_launcher_id: resolved_a,
        parent_coin_id: b32(0x31),
        confirmation_height: 10,
    };
    let slot_b = StoredHandleSlot {
        registry_launcher_id: registry,
        handle_hash,
        counter: 1,
        neighbors_left: Bytes32::default(),
        neighbors_right: Bytes32::new([0xff; 32]),
        expiration: 4_102_444_800,
        owner_launcher_id: resolved_b,
        resolved_launcher_id: resolved_b,
        parent_coin_id: b32(0x32),
        confirmation_height: 20,
    };

    let mut record = HandleSlotRecord {
        registry_launcher_id: registry,
        handle_hash,
        current: None,
        history: Vec::new(),
    };
    push_handle_replacement(&mut record, slot_a.clone(), 10);
    push_handle_replacement(&mut record, slot_b.clone(), 20);
    handle_slots.upsert(record).await;

    store
        .upsert(active_record(
            resolved_a,
            StoredSingletonState {
                launcher_id: resolved_a,
                parent_coin_id: b32(0x41),
                amount: 1,
                inner_puzzle_hash: b32(0x51),
                confirmation_height: 10,
                melted: false,
                melt_height: None,
                nft: None,
                coin_id: b32(0x61),
            },
        ))
        .await;
    store
        .upsert(active_record(
            resolved_b,
            StoredSingletonState {
                launcher_id: resolved_b,
                parent_coin_id: b32(0x42),
                amount: 1,
                inner_puzzle_hash: b32(0x52),
                confirmation_height: 20,
                melted: false,
                melt_height: None,
                nft: None,
                coin_id: b32(0x62),
            },
        ))
        .await;

    let state = ListenerApiState {
        store: store.clone() as Arc<dyn SingletonStore>,
        handle_slots: handle_slots.clone() as Arc<dyn HandleSlotStore>,
        registrations: MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        pending_updates: MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        freshness: Arc::clone(&freshness),
        registry_pricing: Arc::new(RwLock::new(HashMap::from([(
            registry,
            RegistryPricing {
                base_price: 5_000,
                registration_period: 31_557_600,
            },
        )]))),
        registry_launcher_ids: vec![registry],
        now_unix_override: Some(1_700_000_000),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = listener_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    for _ in 0..100 {
        if reqwest::get(format!("{base}/healthz")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let client = reqwest::Client::new();
    let before: Value = client
        .get(format!("{base}/handle/{handle}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        before["slot"]["resolved_launcher_id"].as_str().unwrap(),
        hex::encode(resolved_b)
    );
    assert_eq!(before["slot_confirmation_height"].as_u64().unwrap(), 20);

    // Reorganization orphans height 20; prior slot restored.
    indexer.rollback(20).await;
    // Clear rolling_back so reads succeed; production would note_peak after recovery.
    indexer
        .note_peak(
            50,
            50,
            FreshnessState::now_unix(),
            FreshnessState::now_unix(),
        )
        .await;

    let after: Value = client
        .get(format!("{base}/handle/{handle}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        after["slot"]["resolved_launcher_id"].as_str().unwrap(),
        hex::encode(resolved_a)
    );
    assert_eq!(after["slot_confirmation_height"].as_u64().unwrap(), 10);
    assert_eq!(
        after["slot_parent_coin_id"].as_str().unwrap(),
        hex::encode(b32(0x31))
    );
}

#[tokio::test]
async fn registration_golden_and_lifecycle_semantics() {
    let registry = b32(0xaa);
    let server =
        RunningListener::spawn(FreshnessState::fresh_at(116, FreshnessState::now_unix())).await;
    let client = reqwest::Client::new();
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let secret = b32(0xbb);

    upsert_registration(
        &server,
        StoredRegistration {
            registry_launcher_id: registry,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: secret,
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 1000,
            confirmation_height: 90,
        },
    )
    .await;

    let resp = client
        .get(format!("{}/registrations/{handle}", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("registration_success.json"));
    // Exact public fields only - no private recovery/browser keys.
    let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "action_kind",
            "confirmation_height",
            "handle",
            "indexed_peak_height",
            "protocol_fee",
            "registration_secret",
        ]
    );

    // Strict Handle + freshness reuse Ticket 11 codes.
    let bad = client
        .get(format!("{}/registrations/ALICE", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    assert_eq!(
        normalize_request_id(bad.json().await.unwrap())["code"],
        "invalid_handle"
    );

    *server.freshness.write().await = FreshnessState {
        indexed_peak_height: 10,
        upstream_peak_height: 40,
        last_successful_peak_unix: FreshnessState::now_unix(),
        confirmed_timestamp: FreshnessState::now_unix(),
        rolling_back: false,
        resyncing: false,
    };
    let stale = client
        .get(format!("{}/registrations/{handle}", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 503);
    assert_eq!(
        normalize_request_id(stale.json().await.unwrap())["code"],
        "index_stale"
    );
}

#[tokio::test]
async fn registration_readable_after_expiration_and_replaced_by_expire_not_extend() {
    let registry = b32(0xaa);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let server = RunningListener::spawn_with_registries(
        FreshnessState::fresh_at(200, 2_000_000_000),
        vec![registry],
        Some(2_000_000_000), // after any realistic expiration
    )
    .await;
    let client = reqwest::Client::new();

    let mut record = RegistrationRecord {
        registry_launcher_id: registry,
        handle_hash,
        current: None,
        history: Vec::new(),
    };
    push_registration_replacement(
        &mut record,
        StoredRegistration {
            registry_launcher_id: registry,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: b32(0x11),
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 1000,
            confirmation_height: 90,
        },
        90,
    );
    // Expiry-auction purchase replaces the prior registration fact.
    push_registration_replacement(
        &mut record,
        StoredRegistration {
            registry_launcher_id: registry,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: b32(0x22),
            action_kind: RegistrationActionKind::Expire,
            protocol_fee: 2500,
            confirmation_height: 150,
        },
        150,
    );
    server.registrations.upsert(record).await;

    // Extension leaves the latest registration unchanged - we simply do not project it.
    let body: Value = client
        .get(format!("{}/registrations/{handle}", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["action_kind"], "expire");
    assert_eq!(
        body["registration_secret"].as_str().unwrap(),
        hex::encode(b32(0x22))
    );
    assert_eq!(body["protocol_fee"].as_u64().unwrap(), 2500);
    assert_eq!(body["confirmation_height"].as_u64().unwrap(), 150);
}

#[tokio::test]
async fn registrations_recent_golden_limit_and_total_semantics() {
    let registry = b32(0xaa);
    let server =
        RunningListener::spawn(FreshnessState::fresh_at(116, FreshnessState::now_unix())).await;
    let client = reqwest::Client::new();

    let mut stats = RegistryRegistrationStats::default();
    for (handle, kind, height, bump) in [
        ("alice", RegistrationActionKind::Register, 90u32, true),
        ("bob", RegistrationActionKind::Register, 100, true),
        ("carol", RegistrationActionKind::Expire, 110, false),
    ] {
        stats.events.push(StoredRegistrationEvent {
            handle: handle.to_string(),
            action_kind: kind,
            confirmation_height: height,
        });
        if bump {
            stats.total_registered += 1;
        }
    }
    // Premine-style register already counted above; expire does not increment.
    // Extend/transfer/expiration would also not touch this projection.
    server.registrations.set_stats(registry, stats).await;

    let resp = client
        .get(format!("{}/recent-registrations?limit=50", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("registrations_recent_success.json"));
    let mut root_keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
    root_keys.sort();
    assert_eq!(
        root_keys,
        ["indexed_peak_height", "items", "total_registered"]
    );
    let mut item_keys: Vec<_> = body["items"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    item_keys.sort();
    assert_eq!(item_keys, ["action_kind", "confirmation_height", "handle"]);

    let limited: Value = client
        .get(format!("{}/recent-registrations?limit=1", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(limited["items"].as_array().unwrap().len(), 1);
    assert_eq!(limited["items"][0]["handle"], "carol");
    assert_eq!(limited["total_registered"].as_u64().unwrap(), 2);

    let capped: Value = client
        .get(format!("{}/recent-registrations?limit=999", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(capped["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn registrations_recent_path_looks_up_the_handle_named_recent() {
    let server =
        RunningListener::spawn(FreshnessState::fresh_at(116, FreshnessState::now_unix())).await;
    let client = reqwest::Client::new();

    let stolen = client
        .get(format!("{}/registrations/recent", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(stolen.status(), 404);
    assert_eq!(
        normalize_request_id(stolen.json().await.unwrap())["code"],
        "handle_not_found"
    );

    let feed = client
        .get(format!("{}/recent-registrations", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(feed.status(), 200);
    let body: Value = feed.json().await.unwrap();
    assert!(body["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn registration_reorganization_restores_prior_and_reverses_total() {
    let registry = b32(0xaa);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let registrations = MemoryRegistrationStore::shared();
    let store = MemorySingletonStore::shared();
    let handle_slots = MemoryHandleSlotStore::shared();
    let freshness = Arc::new(RwLock::new(FreshnessState::fresh_at(
        200,
        FreshnessState::now_unix(),
    )));
    let indexer = SingletonIndexer::new(
        store.clone() as Arc<dyn SingletonStore>,
        handle_slots.clone() as Arc<dyn HandleSlotStore>,
        registrations.clone() as Arc<dyn RegistrationStore>,
        MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        Arc::clone(&freshness),
    );

    let mut record = RegistrationRecord {
        registry_launcher_id: registry,
        handle_hash,
        current: None,
        history: Vec::new(),
    };
    push_registration_replacement(
        &mut record,
        StoredRegistration {
            registry_launcher_id: registry,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: b32(0x11),
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 1000,
            confirmation_height: 10,
        },
        10,
    );
    push_registration_replacement(
        &mut record,
        StoredRegistration {
            registry_launcher_id: registry,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: b32(0x22),
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 1000,
            confirmation_height: 20,
        },
        20,
    );
    registrations.upsert(record).await;
    registrations
        .set_stats(
            registry,
            RegistryRegistrationStats {
                total_registered: 2,
                events: vec![
                    StoredRegistrationEvent {
                        handle: handle.to_string(),
                        action_kind: RegistrationActionKind::Register,
                        confirmation_height: 10,
                    },
                    StoredRegistrationEvent {
                        handle: handle.to_string(),
                        action_kind: RegistrationActionKind::Register,
                        confirmation_height: 20,
                    },
                ],
            },
        )
        .await;

    let state = ListenerApiState {
        store: store.clone() as Arc<dyn SingletonStore>,
        handle_slots: handle_slots.clone() as Arc<dyn HandleSlotStore>,
        registrations: registrations.clone() as Arc<dyn RegistrationStore>,
        pending_updates: MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        freshness: Arc::clone(&freshness),
        registry_pricing: Arc::new(RwLock::new(HashMap::from([(
            registry,
            RegistryPricing {
                base_price: 5_000,
                registration_period: 31_557_600,
            },
        )]))),
        registry_launcher_ids: vec![registry],
        now_unix_override: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = listener_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    for _ in 0..100 {
        if reqwest::get(format!("{base}/healthz")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let client = reqwest::Client::new();

    let before: Value = client
        .get(format!("{base}/registrations/{handle}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        before["registration_secret"].as_str().unwrap(),
        hex::encode(b32(0x22))
    );
    let recent_before: Value = client
        .get(format!("{base}/recent-registrations?limit=50"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recent_before["total_registered"].as_u64().unwrap(), 2);
    assert_eq!(recent_before["items"].as_array().unwrap().len(), 2);

    indexer.rollback(20).await;
    indexer
        .note_peak(
            200,
            200,
            FreshnessState::now_unix(),
            FreshnessState::now_unix(),
        )
        .await;

    let after: Value = client
        .get(format!("{base}/registrations/{handle}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        after["registration_secret"].as_str().unwrap(),
        hex::encode(b32(0x11))
    );
    assert_eq!(after["confirmation_height"].as_u64().unwrap(), 10);

    let recent_after: Value = client
        .get(format!("{base}/recent-registrations?limit=50"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recent_after["total_registered"].as_u64().unwrap(), 1);
    assert_eq!(recent_after["items"].as_array().unwrap().len(), 1);
    assert_eq!(recent_after["items"][0]["confirmation_height"], 10);
}

#[tokio::test]
async fn project_registrations_from_logs_covers_register_expire_skips_extend() {
    use chia_wallet_sdk::driver::{
        XchandlesActionLog, XchandlesExpireActionLog, XchandlesExtendActionLog,
        XchandlesPrecommitValue, XchandlesRegisterActionLog,
    };
    use chia_wallet_sdk::types::puzzles::{XchandlesHandleSlotValue, XchandlesPricingSolution};

    let registry = b32(0xaa);
    let registrations = MemoryRegistrationStore::shared();
    let indexer = SingletonIndexer::new(
        MemorySingletonStore::shared() as Arc<dyn SingletonStore>,
        MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
        registrations.clone() as Arc<dyn RegistrationStore>,
        MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        Arc::new(RwLock::new(FreshnessState::fresh_at(
            50,
            FreshnessState::now_unix(),
        ))),
    );

    let handle_slot = |handle: &str, counter: u64| {
        XchandlesHandleSlotValue::new(
            counter,
            handle.tree_hash().into(),
            Bytes32::default(),
            Bytes32::new([0xff; 32]),
            4_102_444_800,
            b32(0x11),
            b32(0x11),
        )
    };
    let precommit = |handle: &str, secret: Bytes32| {
        XchandlesPrecommitValue::new(
            b32(0x01),
            (),
            b32(0x02),
            XchandlesPricingSolution {
                buy_time: 1_700_000_000,
                current_expiration: 0,
                handle: handle.to_string(),
                num_periods: 1,
            },
            handle.to_string(),
            secret,
            b32(0x11),
            b32(0x11),
        )
    };

    // Ordinary / Premine register increments total.
    let register_log = XchandlesActionLog::Register(XchandlesRegisterActionLog {
        spent_left_slot: handle_slot("left", 0),
        spent_right_slot: handle_slot("right", 0),
        created_left_slot: handle_slot("left", 1),
        created_handle_slot: handle_slot("alice", 0),
        created_right_slot: handle_slot("right", 1),
        precommit_value: precommit("alice", b32(0xa1)),
        total_price: 1000,
        registered_time: 31_557_600,
        owner_full_puzzle_hash: b32(0x31),
        resolved_full_puzzle_hash: None,
        owner_inner_puzzle_hash: b32(0x32),
        resolved_inner_puzzle_hash: b32(0x32),
    });
    indexer
        .project_registrations_from_logs(registry, 10, &[register_log])
        .await;

    // Extension must not replace the registration fact or change total.
    let extend_log = XchandlesActionLog::Extend(XchandlesExtendActionLog {
        spent_slot: handle_slot("alice", 0),
        created_slot: handle_slot("alice", 1),
        total_price: 1000,
        registered_time: 31_557_600,
    });
    indexer
        .project_registrations_from_logs(registry, 15, &[extend_log])
        .await;

    let after_extend = registrations
        .get(registry, "alice".tree_hash().into())
        .await
        .unwrap()
        .current
        .unwrap();
    assert_eq!(after_extend.action_kind, RegistrationActionKind::Register);
    assert_eq!(after_extend.confirmation_height, 10);
    assert_eq!(registrations.get_stats(registry).await.total_registered, 1);

    // Expiry-auction purchase replaces the fact but does not increment total.
    let expire_log = XchandlesActionLog::Expire(XchandlesExpireActionLog {
        spent_slot: handle_slot("alice", 1),
        created_slot: handle_slot("alice", 2),
        precommit_value: precommit("alice", b32(0xa2)),
        total_price: 2500,
        registered_time: 31_557_600,
        owner_full_puzzle_hash: b32(0x41),
        resolved_full_puzzle_hash: None,
        owner_inner_puzzle_hash: b32(0x42),
        resolved_inner_puzzle_hash: b32(0x42),
    });
    indexer
        .project_registrations_from_logs(registry, 20, &[expire_log])
        .await;

    let after_expire = registrations
        .get(registry, "alice".tree_hash().into())
        .await
        .unwrap()
        .current
        .unwrap();
    assert_eq!(after_expire.action_kind, RegistrationActionKind::Expire);
    assert_eq!(after_expire.registration_secret, b32(0xa2));
    assert_eq!(after_expire.protocol_fee, 2500);
    let stats = registrations.get_stats(registry).await;
    assert_eq!(stats.total_registered, 1);
    assert_eq!(stats.events.len(), 2);
    assert_eq!(
        stats.events[0].action_kind,
        RegistrationActionKind::Register
    );
    assert_eq!(stats.events[1].action_kind, RegistrationActionKind::Expire);
}

#[tokio::test]
async fn register_keeps_left_neighbor_nft_follow_after_finality() {
    use chia_wallet_sdk::driver::{
        XchandlesActionLog, XchandlesPrecommitValue, XchandlesRegisterActionLog,
    };
    use chia_wallet_sdk::types::puzzles::{XchandlesHandleSlotValue, XchandlesPricingSolution};

    let store = MemorySingletonStore::shared();
    let indexer = SingletonIndexer::new(
        store.clone() as Arc<dyn SingletonStore>,
        MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
        MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        MemoryPendingUpdateStore::shared() as Arc<dyn PendingUpdateStore>,
        Arc::new(RwLock::new(FreshnessState::fresh_at(
            50,
            FreshnessState::now_unix(),
        ))),
    );

    let alice_nft = b32(0xa1);
    let bob_nft = b32(0xb2);
    let alice_state = StoredSingletonState {
        launcher_id: alice_nft,
        parent_coin_id: b32(0x22),
        amount: 1,
        inner_puzzle_hash: b32(0x33),
        confirmation_height: 10,
        melted: false,
        melt_height: None,
        nft: None,
        coin_id: b32(0x88),
    };
    store
        .upsert(active_record(alice_nft, alice_state.clone()))
        .await;
    store
        .upsert(active_record(
            bob_nft,
            StoredSingletonState {
                launcher_id: bob_nft,
                ..alice_state
            },
        ))
        .await;

    let slot = |handle: &str, counter: u64, owner: Bytes32| {
        XchandlesHandleSlotValue::new(
            counter,
            handle.tree_hash().into(),
            Bytes32::default(),
            Bytes32::new([0xff; 32]),
            4_102_444_800,
            owner,
            owner,
        )
    };
    let precommit = |handle: &str, owner: Bytes32| {
        XchandlesPrecommitValue::new(
            b32(0x01),
            (),
            b32(0x02),
            XchandlesPricingSolution {
                buy_time: 1_700_000_000,
                current_expiration: 0,
                handle: handle.to_string(),
                num_periods: 1,
            },
            handle.to_string(),
            Bytes32::default(),
            owner,
            owner,
        )
    };

    // Later register inserts to the right of alice, spending alice's slot as left neighbor.
    let register_bob = XchandlesActionLog::Register(XchandlesRegisterActionLog {
        spent_left_slot: slot("alice", 0, alice_nft),
        spent_right_slot: slot("right", 0, Bytes32::default()),
        created_left_slot: slot("alice", 1, alice_nft),
        created_handle_slot: slot("bob", 0, bob_nft),
        created_right_slot: slot("right", 1, Bytes32::default()),
        precommit_value: precommit("bob", bob_nft),
        total_price: 1000,
        registered_time: 31_557_600,
        owner_full_puzzle_hash: b32(0x31),
        resolved_full_puzzle_hash: None,
        owner_inner_puzzle_hash: b32(0x32),
        resolved_inner_puzzle_hash: b32(0x32),
    });

    let mut allocator = Allocator::new();
    indexer
        .on_registry_transition(&mut allocator, 20, &[], &[register_bob])
        .await
        .unwrap();

    let alice_after = store
        .get(alice_nft)
        .await
        .expect("alice NFT still followed");
    assert_eq!(alice_after.reference_count, 1);
    assert_eq!(alice_after.status, FollowRecordStatus::Active);
    assert!(alice_after.dereference_height.is_none());

    indexer.on_block(&mut allocator, 52, &[]).await.unwrap();
    assert!(
        store.get(alice_nft).await.is_some(),
        "alice NFT must survive 32-block finality after a neighbor register"
    );
}

#[tokio::test]
async fn registration_registry_selection_matches_handle_semantics() {
    let default_registry = b32(0xaa);
    let other = b32(0xbb);
    let server = RunningListener::spawn_with_registries(
        FreshnessState::fresh_at(116, FreshnessState::now_unix()),
        vec![default_registry, other],
        None,
    )
    .await;
    let client = reqwest::Client::new();
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();

    upsert_registration(
        &server,
        StoredRegistration {
            registry_launcher_id: default_registry,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: b32(0x11),
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 1000,
            confirmation_height: 90,
        },
    )
    .await;
    upsert_registration(
        &server,
        StoredRegistration {
            registry_launcher_id: other,
            handle: handle.to_string(),
            handle_hash,
            registration_secret: b32(0x22),
            action_kind: RegistrationActionKind::Register,
            protocol_fee: 2000,
            confirmation_height: 91,
        },
    )
    .await;
    server
        .registrations
        .set_stats(
            default_registry,
            RegistryRegistrationStats {
                total_registered: 1,
                events: vec![StoredRegistrationEvent {
                    handle: handle.to_string(),
                    action_kind: RegistrationActionKind::Register,
                    confirmation_height: 90,
                }],
            },
        )
        .await;
    server
        .registrations
        .set_stats(
            other,
            RegistryRegistrationStats {
                total_registered: 7,
                events: vec![StoredRegistrationEvent {
                    handle: handle.to_string(),
                    action_kind: RegistrationActionKind::Register,
                    confirmation_height: 91,
                }],
            },
        )
        .await;

    let def: Value = client
        .get(format!("{}/registrations/{handle}", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(def["protocol_fee"].as_u64().unwrap(), 1000);

    let explicit: Value = client
        .get(format!(
            "{}/registrations/{handle}?launcher_id={}",
            server.base,
            hex::encode(other)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(explicit["protocol_fee"].as_u64().unwrap(), 2000);

    let unknown = client
        .get(format!(
            "{}/registrations/{handle}?launcher_id={}",
            server.base,
            hex::encode(b32(0xcc))
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    assert_eq!(
        normalize_request_id(unknown.json().await.unwrap())["code"],
        "registry_not_followed"
    );

    let recent_other: Value = client
        .get(format!(
            "{}/recent-registrations?limit=50&launcher_id={}",
            server.base,
            hex::encode(other)
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recent_other["total_registered"].as_u64().unwrap(), 7);
}

async fn upsert_pending(server: &RunningListener, pending: StoredPendingUpdate) {
    let mut record = PendingUpdateRecord {
        registry_launcher_id: pending.registry_launcher_id,
        handle_hash: pending.handle_hash,
        current: None,
        history: Vec::new(),
    };
    let height = pending.update_confirmation_height;
    push_pending_replacement(&mut record, pending, height);
    server.pending_updates.upsert(record).await;
}

fn unexpired_slot(registry: Bytes32, handle_hash: Bytes32, owner: Bytes32) -> StoredHandleSlot {
    StoredHandleSlot {
        registry_launcher_id: registry,
        handle_hash,
        counter: 1,
        neighbors_left: Bytes32::default(),
        neighbors_right: Bytes32::new([0xff; 32]),
        expiration: 4_102_444_800,
        owner_launcher_id: owner,
        resolved_launcher_id: owner,
        parent_coin_id: b32(0x31),
        confirmation_height: 90,
    }
}

fn owner_executor(owner: Bytes32, initiator: Bytes32, coin_id: Bytes32) -> StoredSingletonState {
    StoredSingletonState {
        launcher_id: owner,
        parent_coin_id: initiator,
        amount: 1,
        inner_puzzle_hash: b32(0x33),
        confirmation_height: 100,
        melted: false,
        melt_height: None,
        nft: None,
        coin_id,
    }
}

#[tokio::test]
async fn pending_transfer_golden_future_ready_and_exact_fields() {
    let registry = b32(0xaa);
    let owner = b32(0x11);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let initiator = b32(0x55);
    let executor = b32(0x66);
    let server =
        RunningListener::spawn(FreshnessState::fresh_at(116, FreshnessState::now_unix())).await;
    let client = reqwest::Client::new();

    upsert_handle_slot(&server, unexpired_slot(registry, handle_hash, owner)).await;
    server
        .store
        .upsert(active_record(
            owner,
            owner_executor(owner, initiator, executor),
        ))
        .await;
    upsert_pending(
        &server,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x33),
            new_resolved_launcher_id: b32(0x44),
            update_confirmation_height: 100,
            // Future minimum height remains performable.
            minimum_execution_height: 150,
            update_initiator_coin_id: initiator,
        },
    )
    .await;

    let resp = client
        .get(format!("{}/handle/{handle}/pending-transfer", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("pending_transfer_success.json"));
    let handle_resp = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(handle_resp.status(), 200);
    let handle_body: Value = handle_resp.json().await.unwrap();
    assert_eq!(handle_body["pending_transfer"], body);
    let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "current_executor_coin_id",
            "handle_hash",
            "minimum_execution_height",
            "new_owner_launcher_id",
            "new_resolved_launcher_id",
            "update_confirmation_height",
            "update_initiator_coin_id",
        ]
    );

    // Ready (min height already reached) is still 200 with the same shape.
    upsert_pending(
        &server,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x33),
            new_resolved_launcher_id: b32(0x44),
            update_confirmation_height: 100,
            minimum_execution_height: 50,
            update_initiator_coin_id: initiator,
        },
    )
    .await;
    let ready: Value = client
        .get(format!("{}/handle/{handle}/pending-transfer", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ready["minimum_execution_height"], 50);
    assert_eq!(
        ready["current_executor_coin_id"].as_str().unwrap(),
        hex::encode(executor)
    );
}

#[tokio::test]
async fn pending_transfer_returns_204_for_non_performable_cases() {
    let registry = b32(0xaa);
    let owner = b32(0x11);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let initiator = b32(0x55);
    let executor = b32(0x66);
    let server = RunningListener::spawn_with_registries(
        FreshnessState::fresh_at(116, 1_700_000_000),
        vec![registry],
        Some(1_700_000_000),
    )
    .await;
    let client = reqwest::Client::new();

    // Handle exists, no pending → 204.
    upsert_handle_slot(&server, unexpired_slot(registry, handle_hash, owner)).await;
    server
        .store
        .upsert(active_record(
            owner,
            owner_executor(owner, initiator, executor),
        ))
        .await;
    let none = client
        .get(format!("{}/handle/{handle}/pending-transfer", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 204);
    let proof: Value = client
        .get(format!("{}/handle/{handle}", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(proof["pending_transfer"].is_null());

    upsert_pending(
        &server,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x33),
            new_resolved_launcher_id: b32(0x44),
            update_confirmation_height: 100,
            minimum_execution_height: 150,
            update_initiator_coin_id: initiator,
        },
    )
    .await;

    // Executed / cleared pending → 204.
    let mut cleared = server
        .pending_updates
        .get(registry, handle_hash)
        .await
        .unwrap();
    cleared.current = None;
    server.pending_updates.upsert(cleared).await;
    assert_eq!(
        client
            .get(format!("{}/handle/{handle}/pending-transfer", server.base))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );

    // Restore pending, then separately spent executor (parent ≠ initiator) → 204.
    upsert_pending(
        &server,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x33),
            new_resolved_launcher_id: b32(0x44),
            update_confirmation_height: 100,
            minimum_execution_height: 150,
            update_initiator_coin_id: initiator,
        },
    )
    .await;
    let mut spent = owner_executor(owner, b32(0x99), b32(0xaa));
    spent.parent_coin_id = b32(0x99); // not initiator
    server.store.upsert(active_record(owner, spent)).await;
    assert_eq!(
        client
            .get(format!("{}/handle/{handle}/pending-transfer", server.base))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );

    // Melted owner → 204.
    server
        .store
        .upsert(active_record(
            owner,
            StoredSingletonState {
                launcher_id: owner,
                parent_coin_id: initiator,
                amount: 1,
                inner_puzzle_hash: b32(0x33),
                confirmation_height: 100,
                melted: true,
                melt_height: Some(110),
                nft: None,
                coin_id: executor,
            },
        ))
        .await;
    assert_eq!(
        client
            .get(format!("{}/handle/{handle}/pending-transfer", server.base))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );

    // Expired Handle → 204 (not 410).
    server
        .store
        .upsert(active_record(
            owner,
            owner_executor(owner, initiator, executor),
        ))
        .await;
    let mut expired = unexpired_slot(registry, handle_hash, owner);
    expired.expiration = 1_000_000_000;
    upsert_handle_slot(&server, expired).await;
    assert_eq!(
        client
            .get(format!("{}/handle/{handle}/pending-transfer", server.base))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
}

#[tokio::test]
async fn pending_transfer_stale_invalid_unknown_and_registry_semantics() {
    let registry = b32(0xaa);
    let other = b32(0xbb);
    let owner = b32(0x11);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let initiator = b32(0x55);
    let executor = b32(0x66);
    let server = RunningListener::spawn_with_registries(
        FreshnessState::fresh_at(116, FreshnessState::now_unix()),
        vec![registry, other],
        None,
    )
    .await;
    let client = reqwest::Client::new();

    upsert_handle_slot(&server, unexpired_slot(registry, handle_hash, owner)).await;
    server
        .store
        .upsert(active_record(
            owner,
            owner_executor(owner, initiator, executor),
        ))
        .await;
    upsert_pending(
        &server,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x33),
            new_resolved_launcher_id: b32(0x44),
            update_confirmation_height: 100,
            minimum_execution_height: 150,
            update_initiator_coin_id: initiator,
        },
    )
    .await;

    let bad = client
        .get(format!("{}/handle/ALICE/pending-transfer", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    assert_eq!(
        normalize_request_id(bad.json().await.unwrap()),
        load_golden("error_invalid_handle.json")
    );

    let unknown = client
        .get(format!("{}/handle/zzz/pending-transfer", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    assert_eq!(
        normalize_request_id(unknown.json().await.unwrap()),
        load_golden("error_handle_not_found.json")
    );

    *server.freshness.write().await = FreshnessState {
        indexed_peak_height: 100,
        upstream_peak_height: 200,
        last_successful_peak_unix: FreshnessState::now_unix(),
        confirmed_timestamp: FreshnessState::now_unix(),
        rolling_back: false,
        resyncing: false,
    };
    let stale = client
        .get(format!("{}/handle/{handle}/pending-transfer", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 503);
    assert_eq!(
        normalize_request_id(stale.json().await.unwrap()),
        load_golden("error_index_stale.json")
    );

    // Restore freshness; explicit untracked registry does not fall back.
    *server.freshness.write().await = FreshnessState::fresh_at(116, FreshnessState::now_unix());
    let untracked = client
        .get(format!(
            "{}/handle/{handle}/pending-transfer?launcher_id={}",
            server.base,
            hex::encode(b32(0xcc))
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(untracked.status(), 404);
    assert_eq!(
        normalize_request_id(untracked.json().await.unwrap())["code"],
        "registry_not_followed"
    );

    // Alternate followed registry with no pending for this handle → 404 (no slot there).
    let other_reg = client
        .get(format!(
            "{}/handle/{handle}/pending-transfer?launcher_id={}",
            server.base,
            hex::encode(other)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(other_reg.status(), 404);
}

#[tokio::test]
async fn pending_transfer_reorganization_restores_or_removes() {
    let registry = b32(0xaa);
    let owner = b32(0x11);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let initiator = b32(0x55);
    let executor = b32(0x66);

    let pending_updates = MemoryPendingUpdateStore::shared();
    let handle_slots = MemoryHandleSlotStore::shared();
    let store = MemorySingletonStore::shared();
    let freshness = Arc::new(RwLock::new(FreshnessState::fresh_at(
        200,
        FreshnessState::now_unix(),
    )));
    let indexer = SingletonIndexer::new(
        store.clone() as Arc<dyn SingletonStore>,
        handle_slots.clone() as Arc<dyn HandleSlotStore>,
        MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        pending_updates.clone() as Arc<dyn PendingUpdateStore>,
        Arc::clone(&freshness),
    );

    handle_slots
        .upsert(HandleSlotRecord {
            registry_launcher_id: registry,
            handle_hash,
            current: Some(unexpired_slot(registry, handle_hash, owner)),
            history: Vec::new(),
        })
        .await;
    store
        .upsert(active_record(
            owner,
            owner_executor(owner, initiator, executor),
        ))
        .await;

    let mut record = PendingUpdateRecord {
        registry_launcher_id: registry,
        handle_hash,
        current: None,
        history: Vec::new(),
    };
    push_pending_replacement(
        &mut record,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x33),
            new_resolved_launcher_id: b32(0x44),
            update_confirmation_height: 100,
            minimum_execution_height: 150,
            update_initiator_coin_id: initiator,
        },
        100,
    );
    // Later initiate replaces pending.
    push_pending_replacement(
        &mut record,
        StoredPendingUpdate {
            registry_launcher_id: registry,
            handle_hash,
            new_owner_launcher_id: b32(0x77),
            new_resolved_launcher_id: b32(0x88),
            update_confirmation_height: 120,
            minimum_execution_height: 170,
            update_initiator_coin_id: initiator,
        },
        120,
    );
    pending_updates.upsert(record).await;

    let state = ListenerApiState {
        store: store.clone() as Arc<dyn SingletonStore>,
        handle_slots: handle_slots.clone() as Arc<dyn HandleSlotStore>,
        registrations: MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        pending_updates: pending_updates.clone() as Arc<dyn PendingUpdateStore>,
        freshness: Arc::clone(&freshness),
        registry_pricing: Arc::new(RwLock::new(HashMap::from([(
            registry,
            RegistryPricing {
                base_price: 5_000,
                registration_period: 31_557_600,
            },
        )]))),
        registry_launcher_ids: vec![registry],
        now_unix_override: Some(1_700_000_000),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = listener_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    for _ in 0..100 {
        if reqwest::get(format!("{base}/healthz")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let client = reqwest::Client::new();

    let before: Value = client
        .get(format!("{base}/handle/{handle}/pending-transfer"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["update_confirmation_height"], 120);
    assert_eq!(
        before["new_owner_launcher_id"].as_str().unwrap(),
        hex::encode(b32(0x77))
    );

    // Roll back the later initiate - prior pending is restored.
    indexer.rollback(120).await;
    *freshness.write().await = FreshnessState::fresh_at(200, FreshnessState::now_unix());

    let restored: Value = client
        .get(format!("{base}/handle/{handle}/pending-transfer"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["update_confirmation_height"], 100);
    assert_eq!(
        restored["new_owner_launcher_id"].as_str().unwrap(),
        hex::encode(b32(0x33))
    );

    // Roll back the original initiate - pending removed → 204.
    indexer.rollback(100).await;
    *freshness.write().await = FreshnessState::fresh_at(200, FreshnessState::now_unix());
    assert_eq!(
        client
            .get(format!("{base}/handle/{handle}/pending-transfer"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
}

#[tokio::test]
async fn project_pending_updates_from_logs_covers_initiate_execute_invalidate() {
    use chia_wallet_sdk::driver::{
        XchandlesActionLog, XchandlesExecuteUpdateActionLog, XchandlesExtendActionLog,
        XchandlesInitiateUpdateActionLog,
    };
    use chia_wallet_sdk::types::puzzles::{XchandlesHandleSlotValue, XchandlesUpdateSlotValue};

    let registry = b32(0xaa);
    let pending_updates = MemoryPendingUpdateStore::shared();
    let indexer = SingletonIndexer::new(
        MemorySingletonStore::shared() as Arc<dyn SingletonStore>,
        MemoryHandleSlotStore::shared() as Arc<dyn HandleSlotStore>,
        MemoryRegistrationStore::shared() as Arc<dyn RegistrationStore>,
        pending_updates.clone() as Arc<dyn PendingUpdateStore>,
        Arc::new(RwLock::new(FreshnessState::fresh_at(
            50,
            FreshnessState::now_unix(),
        ))),
    );

    let handle_hash: Bytes32 = "alice".tree_hash().into();
    let handle_slot = |counter: u64| {
        XchandlesHandleSlotValue::new(
            counter,
            handle_hash,
            Bytes32::default(),
            Bytes32::new([0xff; 32]),
            4_102_444_800,
            b32(0x11),
            b32(0x11),
        )
    };
    let update_slot =
        XchandlesUpdateSlotValue::new(b32(0x55), 150, handle_hash, b32(0x33), b32(0x44));

    let initiate = XchandlesActionLog::InitiateUpdate(XchandlesInitiateUpdateActionLog {
        spent_slot: handle_slot(0),
        created_handle_slot: handle_slot(1),
        created_update_slot: update_slot,
        initiator_coin_id: b32(0x55),
    });
    indexer
        .project_pending_updates_from_logs(registry, 100, &[initiate])
        .await;
    let after_init = pending_updates
        .get(registry, handle_hash)
        .await
        .unwrap()
        .current
        .unwrap();
    assert_eq!(after_init.minimum_execution_height, 150);
    assert_eq!(after_init.update_initiator_coin_id, b32(0x55));

    // Extend invalidates pending.
    let extend = XchandlesActionLog::Extend(XchandlesExtendActionLog {
        spent_slot: handle_slot(1),
        created_slot: handle_slot(2),
        total_price: 1000,
        registered_time: 31_557_600,
    });
    indexer
        .project_pending_updates_from_logs(registry, 110, &[extend])
        .await;
    assert!(pending_updates
        .get(registry, handle_hash)
        .await
        .unwrap()
        .current
        .is_none());

    // Re-initiate then execute clears.
    let initiate2 = XchandlesActionLog::InitiateUpdate(XchandlesInitiateUpdateActionLog {
        spent_slot: handle_slot(2),
        created_handle_slot: handle_slot(3),
        created_update_slot: update_slot,
        initiator_coin_id: b32(0x55),
    });
    indexer
        .project_pending_updates_from_logs(registry, 120, &[initiate2])
        .await;
    assert!(pending_updates
        .get(registry, handle_hash)
        .await
        .unwrap()
        .current
        .is_some());

    let execute = XchandlesActionLog::ExecuteUpdate(XchandlesExecuteUpdateActionLog {
        spent_handle_slot: handle_slot(3),
        spent_update_slot: update_slot,
        created_slot: handle_slot(4),
        owner_coin_id: b32(0x66),
        owner_full_puzzle_hash: b32(0x71),
        resolved_full_puzzle_hash: None,
        owner_inner_puzzle_hash: b32(0x72),
        resolved_inner_puzzle_hash: b32(0x72),
    });
    indexer
        .project_pending_updates_from_logs(registry, 130, &[execute])
        .await;
    assert!(pending_updates
        .get(registry, handle_hash)
        .await
        .unwrap()
        .current
        .is_none());
}

// --- Ticket 14: GET /expiring -------------------------------------------------

const EXPIRING_NOW: u64 = 1_800_000_000;
const EXPIRING_CONFIRMED: u64 = 1_800_000_000;

fn expiring_freshness() -> FreshnessState {
    FreshnessState::fresh_at(116, EXPIRING_NOW).with_confirmed_timestamp(EXPIRING_CONFIRMED)
}

fn named_slot(registry: Bytes32, handle: &str, expiration: u64) -> StoredHandleSlot {
    let handle_hash: Bytes32 = handle.tree_hash().into();
    StoredHandleSlot {
        registry_launcher_id: registry,
        handle_hash,
        counter: 1,
        neighbors_left: Bytes32::default(),
        neighbors_right: Bytes32::new([0xff; 32]),
        expiration,
        owner_launcher_id: b32(0x11),
        resolved_launcher_id: b32(0x11),
        parent_coin_id: b32(0x31),
        confirmation_height: 90,
    }
}

fn named_registration(registry: Bytes32, handle: &str) -> StoredRegistration {
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

async fn seed_named_handle(
    server: &RunningListener,
    registry: Bytes32,
    handle: &str,
    expiration: u64,
) {
    upsert_handle_slot(server, named_slot(registry, handle, expiration)).await;
    upsert_registration(server, named_registration(registry, handle)).await;
}

#[tokio::test]
async fn expiring_active_golden_and_exact_fields() {
    let registry = b32(0xaa);
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![registry],
        Some(EXPIRING_NOW),
    )
    .await;
    let client = reqwest::Client::new();

    // Oldest expiration first: bob (20d), alice (10d), carol (5d).
    seed_named_handle(&server, registry, "bob", EXPIRING_NOW - 20 * 86_400).await;
    seed_named_handle(&server, registry, "alice", EXPIRING_NOW - 10 * 86_400).await;
    seed_named_handle(&server, registry, "carol", EXPIRING_NOW - 5 * 86_400).await;
    // Zero-premium (day 28+) and not-yet-expired must be excluded.
    seed_named_handle(
        &server,
        registry,
        "zeroed",
        EXPIRING_CONFIRMED + 420 - 28 * 86_400,
    )
    .await;
    seed_named_handle(&server, registry, "active", EXPIRING_NOW + 86_400).await;

    let resp = client
        .get(format!("{}/expiring?view=active", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("expiring_active_success.json"));
    let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["confirmed_timestamp", "indexed_peak_height", "items"]
    );
    let item_keys: Vec<_> = body["items"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let mut item_keys = item_keys;
    item_keys.sort();
    assert_eq!(
        item_keys,
        [
            "base_registration_fee",
            "current_premium",
            "expiration",
            "handle",
            "projected_pricing_timestamp",
            "reaches_base_at",
            "total_registration_fee",
        ]
    );
}

#[tokio::test]
async fn expiring_soon_golden_and_30_day_boundary() {
    let registry = b32(0xaa);
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![registry],
        Some(EXPIRING_NOW),
    )
    .await;
    let client = reqwest::Client::new();

    seed_named_handle(&server, registry, "dan", EXPIRING_NOW + 7 * 86_400).await;
    seed_named_handle(&server, registry, "eve", EXPIRING_NOW + 29 * 86_400).await;
    seed_named_handle(&server, registry, "frank", EXPIRING_NOW + 30 * 86_400).await;
    // Outside window / already expired excluded.
    seed_named_handle(&server, registry, "greg", EXPIRING_NOW + 30 * 86_400 + 1).await;
    seed_named_handle(&server, registry, "old", EXPIRING_NOW - 1).await;

    let resp = client
        .get(format!("{}/expiring?view=soon", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body, load_golden("expiring_soon_success.json"));
}

#[tokio::test]
async fn expiring_soon_quotes_test8_from_testnet_base_price() {
    let registry = b32(0xaa);
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![registry],
        Some(EXPIRING_NOW),
    )
    .await;
    server.registry_pricing.write().await.insert(
        registry,
        RegistryPricing {
            base_price: 5,
            registration_period: 31_557_600,
        },
    );
    seed_named_handle(&server, registry, "test8", EXPIRING_NOW + 7 * 86_400).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/expiring?view=soon", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["handle"], "test8");
    // 5-char + digit → 8× live base, not the mainnet 5000 default (40000).
    assert_eq!(items[0]["base_registration_fee"], 40);
}

#[tokio::test]
async fn expiring_invalid_view_and_stale_refuse_entire_page() {
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![b32(0xaa)],
        Some(EXPIRING_NOW),
    )
    .await;
    let client = reqwest::Client::new();

    for view in ["", "Active", "all", "pending"] {
        let url = if view.is_empty() {
            format!("{}/expiring", server.base)
        } else {
            format!("{}/expiring?view={view}", server.base)
        };
        let resp = client.get(url).send().await.unwrap();
        assert_eq!(resp.status(), 400, "view={view:?}");
        let body = normalize_request_id(resp.json().await.unwrap());
        assert_eq!(body, load_golden("error_invalid_view.json"));
    }

    *server.freshness.write().await = FreshnessState {
        indexed_peak_height: 100,
        upstream_peak_height: 200,
        last_successful_peak_unix: FreshnessState::now_unix(),
        confirmed_timestamp: EXPIRING_CONFIRMED,
        rolling_back: false,
        resyncing: false,
    };
    let stale = client
        .get(format!("{}/expiring?view=active", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 503);
    let body = normalize_request_id(stale.json().await.unwrap());
    assert_eq!(body, load_golden("error_index_stale.json"));
}

#[tokio::test]
async fn expiring_pagination_cursor_stable_no_dup_skip() {
    let registry = b32(0xaa);
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![registry],
        Some(EXPIRING_NOW),
    )
    .await;
    let client = reqwest::Client::new();

    // Three active auctions; page size 2 then 1.
    seed_named_handle(&server, registry, "bob", EXPIRING_NOW - 20 * 86_400).await;
    seed_named_handle(&server, registry, "alice", EXPIRING_NOW - 10 * 86_400).await;
    seed_named_handle(&server, registry, "carol", EXPIRING_NOW - 5 * 86_400).await;

    let page1: Value = client
        .get(format!("{}/expiring?view=active&limit=2", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page1["items"].as_array().unwrap().len(), 2);
    assert_eq!(page1["items"][0]["handle"], "bob");
    assert_eq!(page1["items"][1]["handle"], "alice");
    let cursor = page1["next_cursor"].as_str().unwrap().to_string();

    let page2: Value = client
        .get(format!(
            "{}/expiring?view=active&limit=2&cursor={cursor}",
            server.base
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert_eq!(page2["items"][0]["handle"], "carol");
    assert!(page2.get("next_cursor").is_none());

    // Cap at 50 even if asked for more.
    let capped: Value = client
        .get(format!("{}/expiring?view=active&limit=999", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(capped["items"].as_array().unwrap().len(), 3);
    assert!(capped.get("next_cursor").is_none());
}

#[tokio::test]
async fn expiring_day28_premium_transition_and_projection() {
    let registry = b32(0xaa);
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![registry],
        Some(EXPIRING_NOW),
    )
    .await;
    let client = reqwest::Client::new();

    let projected = EXPIRING_CONFIRMED + 420;
    // Exactly at day-28 boundary relative to projected pricing timestamp → excluded.
    seed_named_handle(&server, registry, "edge", projected - 28 * 86_400).await;
    // Decay is already 0 ~18 minutes before the cutoff → also excluded.
    seed_named_handle(&server, registry, "gone", projected - 28 * 86_400 + 1).await;
    // Still a positive premium ~33 minutes before the 28-day mark.
    seed_named_handle(&server, registry, "last", projected - 28 * 86_400 + 2_000).await;

    let body: Value = client
        .get(format!("{}/expiring?view=active", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let handles: Vec<_> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["handle"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(handles, vec!["last".to_string()]);
    assert_eq!(body["items"][0]["projected_pricing_timestamp"], projected);
    assert!(body["items"][0]["current_premium"].as_u64().unwrap() > 0);
    assert_eq!(
        body["items"][0]["reaches_base_at"].as_u64().unwrap(),
        (projected - 28 * 86_400 + 2_000) + 28 * 86_400
    );
}

#[tokio::test]
async fn expiring_membership_updates_after_fork_rollback() {
    let registry = b32(0xaa);
    let handle = "alice";
    let handle_hash: Bytes32 = handle.tree_hash().into();
    let freshness = Arc::new(RwLock::new(expiring_freshness()));
    let store = MemorySingletonStore::shared();
    let handle_slots = MemoryHandleSlotStore::shared();
    let registrations = MemoryRegistrationStore::shared();
    let pending_updates = MemoryPendingUpdateStore::shared();
    let indexer = SingletonIndexer::new(
        store.clone() as Arc<dyn SingletonStore>,
        handle_slots.clone() as Arc<dyn HandleSlotStore>,
        registrations.clone() as Arc<dyn RegistrationStore>,
        pending_updates.clone() as Arc<dyn PendingUpdateStore>,
        Arc::clone(&freshness),
    );

    // Prior longer-lived slot, then shorter expiration at height 120.
    let mut slot_rec = HandleSlotRecord {
        registry_launcher_id: registry,
        handle_hash,
        current: None,
        history: Vec::new(),
    };
    let mut prior = named_slot(registry, handle, EXPIRING_NOW + 40 * 86_400);
    prior.confirmation_height = 100;
    push_handle_replacement(&mut slot_rec, prior, 100);
    let mut later = named_slot(registry, handle, EXPIRING_NOW + 7 * 86_400);
    later.confirmation_height = 120;
    push_handle_replacement(&mut slot_rec, later, 120);
    handle_slots.upsert(slot_rec).await;
    let mut reg_rec = RegistrationRecord {
        registry_launcher_id: registry,
        handle_hash,
        current: None,
        history: Vec::new(),
    };
    push_registration_replacement(&mut reg_rec, named_registration(registry, handle), 100);
    registrations.upsert(reg_rec).await;

    let state = ListenerApiState {
        store: store.clone() as Arc<dyn SingletonStore>,
        handle_slots: handle_slots.clone() as Arc<dyn HandleSlotStore>,
        registrations: registrations.clone() as Arc<dyn RegistrationStore>,
        pending_updates: pending_updates.clone() as Arc<dyn PendingUpdateStore>,
        freshness: Arc::clone(&freshness),
        registry_pricing: Arc::new(RwLock::new(HashMap::from([(
            registry,
            RegistryPricing {
                base_price: 5_000,
                registration_period: 31_557_600,
            },
        )]))),
        registry_launcher_ids: vec![registry],
        now_unix_override: Some(EXPIRING_NOW),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = listener_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    for _ in 0..100 {
        if reqwest::get(format!("{base}/healthz")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let client = reqwest::Client::new();

    let before: Value = client
        .get(format!("{base}/expiring?view=soon"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["items"].as_array().unwrap().len(), 1);
    assert_eq!(before["items"][0]["expiration"], EXPIRING_NOW + 7 * 86_400);

    indexer.rollback(120).await;
    indexer
        .note_peak(200, 200, EXPIRING_NOW, EXPIRING_CONFIRMED)
        .await;

    let restored_slot = handle_slots
        .get(registry, handle_hash)
        .await
        .unwrap()
        .current
        .unwrap();
    assert_eq!(restored_slot.confirmation_height, 100);
    assert_eq!(restored_slot.expiration, EXPIRING_NOW + 40 * 86_400);

    let after: Value = client
        .get(format!("{base}/expiring?view=soon"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // Restored 40-day expiration is outside the 30-day soon window.
    assert!(
        after["items"].as_array().unwrap().is_empty(),
        "unexpected soon items after rollback: {after}"
    );
}

#[tokio::test]
async fn expiring_new_peak_refreshes_projected_pricing() {
    let registry = b32(0xaa);
    let server = RunningListener::spawn_with_registries(
        expiring_freshness(),
        vec![registry],
        Some(EXPIRING_NOW),
    )
    .await;
    let client = reqwest::Client::new();
    seed_named_handle(&server, registry, "bob", EXPIRING_NOW - 10 * 86_400).await;

    let before: Value = client
        .get(format!("{}/expiring?view=active", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let prem_before = before["items"][0]["current_premium"].as_u64().unwrap();
    assert_eq!(before["confirmed_timestamp"], EXPIRING_CONFIRMED);

    // Advance confirmed chain time by one day - premium must fall.
    server.freshness.write().await.confirmed_timestamp = EXPIRING_CONFIRMED + 86_400;
    let after: Value = client
        .get(format!("{}/expiring?view=active", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["confirmed_timestamp"], EXPIRING_CONFIRMED + 86_400);
    assert_eq!(
        after["items"][0]["projected_pricing_timestamp"],
        EXPIRING_CONFIRMED + 86_400 + 420
    );
    let prem_after = after["items"][0]["current_premium"].as_u64().unwrap();
    assert!(prem_after < prem_before);
}

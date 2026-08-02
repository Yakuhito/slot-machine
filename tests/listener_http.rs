//! Real-HTTP listener fixture and golden contracts for Tickets 10–11.
//!
//! Later tickets must extend this same server fixture and consume these goldens
//! rather than redefining the singleton/error envelope independently.

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
    discover_singleton_in_block, listener_router, push_handle_replacement, push_replacement,
    rollback_to_before, DiscoveryResult, FollowRecordStatus, FollowedSingleton, FreshnessState,
    HandleSlotRecord, HandleSlotStore, ListenerApiState, MemoryHandleSlotStore,
    MemorySingletonStore, ParsedNftState, SingletonIndexer, SingletonStore, StoredHandleSlot,
    StoredSingletonState,
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
    freshness: Arc<RwLock<FreshnessState>>,
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
        let freshness = Arc::new(RwLock::new(freshness));
        let state = ListenerApiState {
            store: store.clone() as Arc<dyn SingletonStore>,
            handle_slots: handle_slots.clone() as Arc<dyn HandleSlotStore>,
            freshness: Arc::clone(&freshness),
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
            freshness,
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
    server.store.upsert(active_record(launcher, nft_state)).await;

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
    let new_did = did
        .update(&mut ctx, &p2, Conditions::new())?;
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
        &[discovery_spend.clone()],
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
        .on_block(&mut allocator, 5, &[discovery_spend.clone()])
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
        &[discovery_spend.clone()],
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

    // Strict Handle syntax — no normalization.
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
    server.store.upsert(active_record(resolved, nft_state)).await;

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
        freshness: Arc::clone(&freshness),
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
        .note_peak(50, 50, FreshnessState::now_unix())
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

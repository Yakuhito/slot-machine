//! Real-HTTP listener fixture and golden contracts for Ticket 10.
//!
//! Tickets 11–14 must extend this same server fixture and consume these goldens
//! rather than redefining the singleton/error envelope independently.

use std::sync::Arc;
use std::time::Duration;

use chia_protocol::Bytes32;
use chia_puzzle_types::singleton::SingletonArgs;
use chia_wallet_sdk::driver::{Launcher, SingletonInfo, SpendContext, StandardLayer};
use chia_wallet_sdk::test::{BlsPair, Simulator};
use chia_wallet_sdk::types::Conditions;
use clvmr::Allocator;
use serde_json::Value;
use slot_machine::{
    discover_singleton_in_block, listener_router, push_replacement, rollback_to_before,
    DiscoveryResult, FollowRecordStatus, FollowedSingleton, FreshnessState, ListenerApiState,
    MemorySingletonStore, ParsedNftState, SingletonIndexer, SingletonStore, StoredSingletonState,
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
    freshness: Arc<RwLock<FreshnessState>>,
    _join: tokio::task::JoinHandle<()>,
}

impl RunningListener {
    async fn spawn(freshness: FreshnessState) -> Self {
        let store = MemorySingletonStore::shared();
        let freshness = Arc::new(RwLock::new(freshness));
        let state = ListenerApiState {
            store: store.clone() as Arc<dyn SingletonStore>,
            freshness: Arc::clone(&freshness),
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

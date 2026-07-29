//! Integration tests for DHT record operations.
//!
//! Exercises the record.rs API surface against a live Veilid node:
//! create_dflt, set (with conflict return), get, get_full, inspect
//! (Local scope), close, delete, size limits.
//!
//! Uses veilid_core::api_startup with insecure local storage and
//! delete-on-shutdown. Each test gets an isolated storage directory
//! via process id + atomic counter.
//!
//! All tests use LOCAL-ONLY operations (no force_refresh, no network
//! fanout, no watch, inspect Local only). Network-dependent operations
//! (open after delete, watch, inspect SyncGet/UpdateGet, force_refresh
//! get) require public_internet_ready and are tested separately in
//! environments with network access.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use veilid_core::{
    api_startup, DHTReportScope, DHTSchema, RoutingContext, UpdateCallback, VeilidAPI,
    VeilidConfig, VeilidConfigBlockStore, VeilidConfigNetwork,
    VeilidConfigProtectedStore, VeilidConfigTableStore, VeilidUpdate, CRYPTO_KIND_VLD0,
};

static TEST_INSTANCE: AtomicUsize = AtomicUsize::new(0);

fn test_instance_id() -> String {
    let id = TEST_INSTANCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", std::process::id(), id)
}

fn test_storage_dir(instance: &str, store: &str) -> String {
    let base = std::env::temp_dir()
        .join("rekindle-transport-veilid-tests")
        .join(instance)
        .join(store);
    std::fs::create_dir_all(&base).expect("create test storage dir");
    base.to_string_lossy().into_owned()
}

fn test_config() -> VeilidConfig {
    let instance = test_instance_id();
    VeilidConfig {
        program_name: format!("RekindleTransportTest-{}", instance),
        table_store: VeilidConfigTableStore {
            directory: test_storage_dir(&instance, "table_store"),
            delete: true,
        },
        block_store: VeilidConfigBlockStore {
            directory: test_storage_dir(&instance, "block_store"),
            delete: true,
        },
        protected_store: VeilidConfigProtectedStore {
            allow_insecure_fallback: true,
            always_use_insecure_storage: true,
            directory: test_storage_dir(&instance, "protected_store"),
            delete: true,
            ..Default::default()
        },
        network: VeilidConfigNetwork {
            upnp: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn test_update_callback() -> UpdateCallback {
    Arc::new(|_update: VeilidUpdate| {})
}

async fn start_api() -> VeilidAPI {
    let api = api_startup(test_update_callback(), test_config())
        .await
        .expect("api_startup failed");
    api.attach().await.expect("attach failed");
    // Poll attachment state until the node reports attached.
    // The storage manager is ready once the node reaches any attached state.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let state = api.get_state().await.expect("get_state failed");
        if state.attachment.state.is_attached() {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "veilid node did not attach within 30s, last state: {:?}",
                state.attachment.state
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    api
}

fn routing_context(api: &VeilidAPI) -> RoutingContext {
    // Default routing context (Safe mode, 1 hop).
    // footgun feature is not enabled, so Unsafe mode is unavailable.
    api.routing_context().expect("routing_context failed")
}

// ── Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn create_set_get_close_delete() {
    let api = start_api().await;
    let rc = routing_context(&api);

    // Create a DFLT record with 1 subkey
    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(1).unwrap(), None)
        .await
        .expect("create_dht_record failed");
    let key = rec.key();

    // Set subkey 0 -- set_dht_value returns Option<ValueData>
    let data = b"hello veilid".to_vec();
    let conflict = rc
        .set_dht_value(key.clone(), 0, data.clone(), None)
        .await
        .expect("set_dht_value failed");
    // First write to a fresh record: no conflict (None)
    assert!(conflict.is_none(), "first write should not conflict");

    // Get subkey 0 (local cache hit, no network fanout)
    let value = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get_dht_value failed")
        .expect("subkey 0 should have data");
    assert_eq!(value.data(), data);

    // Close and delete
    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn get_returns_seq_and_writer() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(1).unwrap(), None)
        .await
        .expect("create failed");
    let key = rec.key();
    let owner = rec.owner();

    // Write a value
    rc.set_dht_value(key.clone(), 0, b"first".to_vec(), None)
        .await
        .expect("set failed");

    // get_dht_value returns ValueData with seq and writer
    let vd = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get failed")
        .expect("should have value");

    assert_eq!(vd.data(), b"first");
    assert_eq!(vd.seq(), 0.into());
    assert_eq!(vd.writer(), owner);

    // Overwrite and verify seq increments
    rc.set_dht_value(key.clone(), 0, b"second".to_vec(), None)
        .await
        .expect("set failed");

    let vd2 = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get failed")
        .expect("should have value");

    assert_eq!(vd2.data(), b"second");
    assert_eq!(vd2.seq(), 1.into());
    assert_eq!(vd2.writer(), owner);

    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn set_returns_none_on_success() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(1).unwrap(), None)
        .await
        .expect("create failed");
    let key = rec.key();

    // First write: returns None (no conflict)
    let result = rc
        .set_dht_value(key.clone(), 0, b"value-a".to_vec(), None)
        .await
        .expect("set failed");
    assert!(result.is_none(), "first set should return None");

    // Overwrite same subkey: returns None (same writer, seq advances)
    let result = rc
        .set_dht_value(key.clone(), 0, b"value-b".to_vec(), None)
        .await
        .expect("set failed");
    assert!(result.is_none(), "overwrite by same writer should return None");

    // Verify the overwrite took effect
    let vd = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get failed")
        .expect("should have value");
    assert_eq!(vd.data(), b"value-b");
    assert_eq!(vd.seq(), 1.into());

    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn inspect_local_scope() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(2).unwrap(), None)
        .await
        .expect("create failed");
    let key = rec.key();

    // Write to subkey 0
    rc.set_dht_value(key.clone(), 0, b"inspect-test".to_vec(), None)
        .await
        .expect("set failed");

    // Inspect with Local scope (no network fanout)
    let report = rc
        .inspect_dht_record(key.clone(), None, DHTReportScope::Local)
        .await
        .expect("inspect Local failed");

    // Local scope should return local sequence numbers
    let local_seqs = report.local_seqs();
    assert!(!local_seqs.is_empty(), "local_seqs should not be empty");

    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn delete_nonexistent_record_fails() {
    let api = start_api().await;
    let rc = routing_context(&api);

    // Construct a bogus key
    let bogus = veilid_core::RecordKey::new(
        CRYPTO_KIND_VLD0,
        veilid_core::BareRecordKey::new(
            veilid_core::BareOpaqueRecordKey::new(&[0xABu8; 32]),
            Some(veilid_core::BareSharedSecret::new(&[0xCDu8; 32])),
        ),
    );
    let result = rc.delete_dht_record(bogus).await;
    assert!(result.is_err(), "delete of nonexistent record should fail");

    api.shutdown().await;
}

#[tokio::test]
async fn subkey_size_limit_enforced() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(1).unwrap(), None)
        .await
        .expect("create failed");
    let key = rec.key();

    // Write exactly 32768 bytes -- should succeed (ValueData::MAX_LEN)
    let max_data = vec![0xAAu8; 32768];
    let result = rc.set_dht_value(key.clone(), 0, max_data, None).await;
    assert!(result.is_ok(), "32768 bytes should be accepted");

    // Write 32769 bytes -- should fail
    let over_data = vec![0xBBu8; 32769];
    let result = rc.set_dht_value(key.clone(), 0, over_data, None).await;
    assert!(result.is_err(), "32769 bytes should be rejected");

    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn set_overwrite_increments_seq() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(1).unwrap(), None)
        .await
        .expect("create failed");
    let key = rec.key();

    // Write, read, overwrite, read -- verify seq increments on same subkey
    rc.set_dht_value(key.clone(), 0, b"v1".to_vec(), None)
        .await
        .expect("set v1 failed");
    let vd1 = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get v1 failed")
        .expect("should have v1");
    assert_eq!(vd1.data(), b"v1");
    assert_eq!(vd1.seq(), 0.into());

    rc.set_dht_value(key.clone(), 0, b"v2".to_vec(), None)
        .await
        .expect("set v2 failed");
    let vd2 = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get v2 failed")
        .expect("should have v2");
    assert_eq!(vd2.data(), b"v2");
    assert_eq!(vd2.seq(), 1.into());

    rc.set_dht_value(key.clone(), 0, b"v3".to_vec(), None)
        .await
        .expect("set v3 failed");
    let vd3 = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get v3 failed")
        .expect("should have v3");
    assert_eq!(vd3.data(), b"v3");
    assert_eq!(vd3.seq(), 2.into());

    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn create_with_owner_keypair() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let crypto = api.crypto().expect("crypto failed");
    let cs = crypto.get(CRYPTO_KIND_VLD0).expect("get cs failed");
    let owner_keypair = cs.generate_keypair();

    let rec = rc
        .create_dht_record(
            CRYPTO_KIND_VLD0,
            DHTSchema::dflt(1).unwrap(),
            Some(owner_keypair.clone()),
        )
        .await
        .expect("create with owner failed");

    assert_eq!(rec.ref_owner(), &owner_keypair.key());

    let key = rec.key();

    // Write and read with the owner
    rc.set_dht_value(key.clone(), 0, b"owned-data".to_vec(), None)
        .await
        .expect("set failed");

    let vd = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .expect("get failed")
        .expect("should have value");
    assert_eq!(vd.data(), b"owned-data");
    assert_eq!(vd.writer(), owner_keypair.key());

    rc.close_dht_record(key.clone()).await.expect("close failed");
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    api.shutdown().await;
}

#[tokio::test]
async fn close_then_delete_lifecycle() {
    let api = start_api().await;
    let rc = routing_context(&api);

    let rec = rc
        .create_dht_record(CRYPTO_KIND_VLD0, DHTSchema::dflt(1).unwrap(), None)
        .await
        .expect("create failed");
    let key = rec.key();

    rc.set_dht_value(key.clone(), 0, b"lifecycle-test".to_vec(), None)
        .await
        .expect("set failed");

    // Close must succeed
    rc.close_dht_record(key.clone()).await.expect("close failed");

    // Delete after close must succeed
    rc.delete_dht_record(key.clone()).await.expect("delete failed");

    // Double-delete should fail (record already deleted)
    let result = rc.delete_dht_record(key.clone()).await;
    assert!(result.is_err(), "double delete should fail");

    api.shutdown().await;
}

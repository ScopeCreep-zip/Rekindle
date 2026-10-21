//! The pool on a small in-memory backend, on tokio's paused clock: the watch
//! re-arm's backoff and forgiveness run on `tokio::time`, so a test (and the
//! netsim) drives them without waiting.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rekindle_lifecycle::SessionScope;
use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, RecordDescriptor, RecordReport, RecordSchema,
    RecordValue, ReportScope, ValueChange, WriterKey,
};

use crate::{RecordCloser, RecordPool};

const LEASED_RECORD: &str = "record-a";

/// Opens every key with four subkeys, stores nothing, counts watch calls,
/// and refuses the first `open_failures` opens with `TryAgain`.
#[derive(Default)]
struct TestDht {
    watches: AtomicU32,
    opens: AtomicU32,
    open_failures: u32,
}

#[async_trait::async_trait]
impl DhtBackend for TestDht {
    fn parse_writer(&self, text: &str) -> Result<WriterKey, DhtCallError> {
        Ok(WriterKey {
            keypair: text.to_string(),
            public: text.to_string(),
        })
    }

    fn check_key(&self, key: &str) -> Result<String, DhtCallError> {
        Ok(key.to_string())
    }

    async fn create(
        &self,
        _schema: &RecordSchema,
        _owner: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        Err(DhtCallError {
            kind: DhtErrorKind::Unimplemented,
            message: "the test backend creates nothing".into(),
        })
    }

    async fn open(
        &self,
        key: &str,
        _writer: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        if self.opens.fetch_add(1, Ordering::SeqCst) < self.open_failures {
            return Err(DhtCallError {
                kind: DhtErrorKind::TryAgain,
                message: "offline".into(),
            });
        }
        Ok(RecordDescriptor {
            key: key.to_string(),
            owner: None,
            subkey_count: 4,
        })
    }

    async fn close(&self, _key: &str) -> Result<(), DhtCallError> {
        Ok(())
    }

    async fn get(
        &self,
        _key: &str,
        _subkey: u32,
        _force_refresh: bool,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        Ok(None)
    }

    async fn set(
        &self,
        _key: &str,
        _subkey: u32,
        _data: Vec<u8>,
        _writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        Ok(None)
    }

    async fn inspect(
        &self,
        _key: &str,
        _subkeys: Option<&[u32]>,
        _scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        Ok(RecordReport {
            subkeys: Vec::new(),
            local_seqs: Vec::new(),
            network_seqs: Vec::new(),
            offline_subkeys: Vec::new(),
        })
    }

    async fn watch(
        &self,
        _key: &str,
        _subkeys: Option<&[u32]>,
        _expiration_us: Option<u64>,
        _count: Option<u32>,
    ) -> Result<bool, DhtCallError> {
        self.watches.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }

    async fn cancel_watch(
        &self,
        _key: &str,
        _subkeys: Option<&[u32]>,
    ) -> Result<bool, DhtCallError> {
        Ok(false)
    }
}

fn pool_on(backend: Arc<TestDht>) -> Arc<RecordPool> {
    let (_ready_tx, ready) = tokio::sync::watch::channel(true);
    RecordPool::new(
        backend,
        SessionScope::new("pool test", Arc::new(|_| {})),
        ready,
        RecordCloser::new(),
    )
}

fn death() -> ValueChange {
    ValueChange {
        key: LEASED_RECORD.to_string(),
        subkeys: Vec::new(),
        count: 0,
        value: None,
    }
}

/// Lets the paused clock run `d` forward with every ready task run first.
async fn pass(d: Duration) {
    tokio::time::sleep(d).await;
}

#[tokio::test(start_paused = true)]
async fn a_dead_watch_re_arms_with_backoff_and_is_forgiven_after_five_minutes() {
    let backend = Arc::new(TestDht::default());
    let pool = pool_on(Arc::clone(&backend));
    let lease = pool.acquire(LEASED_RECORD, None).await.expect("open");
    pool.watch_all(lease).await.expect("watch");
    assert_eq!(backend.watches.load(Ordering::SeqCst), 1);

    // First death: re-armed after 1 s, not before.
    assert!(pool.on_value_change(&death()).watch_died);
    pass(Duration::from_millis(990)).await;
    assert_eq!(backend.watches.load(Ordering::SeqCst), 1);
    pass(Duration::from_millis(20)).await;
    assert_eq!(backend.watches.load(Ordering::SeqCst), 2);

    // Second death soon after: the delay doubles to 2 s.
    pool.on_value_change(&death());
    pass(Duration::from_millis(1990)).await;
    assert_eq!(backend.watches.load(Ordering::SeqCst), 2);
    pass(Duration::from_millis(20)).await;
    assert_eq!(backend.watches.load(Ordering::SeqCst), 3);

    // Up for longer than five minutes: forgiven, back to 1 s.
    pass(Duration::from_secs(301)).await;
    pool.on_value_change(&death());
    pass(Duration::from_millis(1010)).await;
    assert_eq!(backend.watches.load(Ordering::SeqCst), 4);
}

#[tokio::test(start_paused = true)]
async fn a_watch_released_during_its_backoff_is_not_re_armed() {
    let backend = Arc::new(TestDht::default());
    let pool = pool_on(Arc::clone(&backend));
    let lease = pool.acquire(LEASED_RECORD, None).await.expect("open");
    pool.watch_all(lease).await.expect("watch");
    pool.on_value_change(&death());
    pool.release(lease).await;
    pass(Duration::from_secs(5)).await;
    assert_eq!(backend.watches.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn an_open_retries_transient_errors_one_second_apart_within_the_budget() {
    let backend = Arc::new(TestDht {
        open_failures: 3,
        ..TestDht::default()
    });
    let pool = pool_on(Arc::clone(&backend));
    let started = tokio::time::Instant::now();
    pool.acquire(LEASED_RECORD, None)
        .await
        .expect("open after retries");
    assert_eq!(backend.opens.load(Ordering::SeqCst), 4);
    assert_eq!(started.elapsed(), Duration::from_secs(3));

    let failing = Arc::new(TestDht {
        open_failures: u32::MAX,
        ..TestDht::default()
    });
    let pool = pool_on(Arc::clone(&failing));
    let err = pool
        .acquire(LEASED_RECORD, None)
        .await
        .expect_err("budget spent");
    assert!(matches!(
        err,
        rekindle_records::port::PortError::Unreachable(_)
    ));
}

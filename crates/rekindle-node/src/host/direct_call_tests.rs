//! A 1:1 call the Host could not judge is answered retry later, never
//! NAKed (RV2, `c37fef40`; RV5 M2): a NAK is a refusal, which ends a first
//! contact; "retry later" frees Veilid's RPC worker at once and the
//! caller's row is attempted again.

use rekindle_transport::operations::direct;

use super::direct_tests::{now_ms, sealed_request, started};

/// A well-formed request to a Host with no identity unlocked: the
/// admission lookup cannot run, so nothing was judged.
#[tokio::test]
async fn a_call_the_host_cannot_judge_is_told_to_retry_later() {
    let dir = tempfile::tempdir().unwrap();
    let host = started(&dir).await;
    let (bob, ann) = (
        rekindle_crypto::Identity::generate(),
        rekindle_crypto::Identity::generate(),
    );
    let frame = sealed_request(&bob, ann.public_key_bytes(), [1; 16], now_ms());
    let datagram = rekindle_transport::operations::dm::wire(&frame).unwrap();
    assert!(
        matches!(
            host.receive_direct_call(&datagram).await,
            direct::Called::RetryLater
        ),
        "an internal error is no refusal"
    );
}

/// LV23 review (amending RV5 R1): a blocked sender's call gets the silent
/// receipt, the empty reply a reachable peer gives, never a NAK that would
/// tell them they are blocked; and nothing of it is stored or published.
#[tokio::test]
async fn a_blocked_sender_is_receipted_silently_and_nothing_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = super::direct_tests::ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let bob = rekindle_crypto::Identity::generate();
    let (o, b) = (hex::encode(owner), hex::encode(bob.public_key_bytes()));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::blocked::insert(conn, &o, &b, "Bob", 1))
        .await
        .unwrap();
    let frame = sealed_request(&bob, owner, [8; 16], now_ms());
    assert!(super::direct_tests::call(&host, &frame).await.is_empty());
    assert_eq!(super::direct_tests::requests(&host, owner).await, 0);
    assert_eq!(super::direct_tests::received_events(&mut events).await, 0);
}

/// RV5 M2: a store the request's own values fail (a constraint) is a
/// judged refusal, answered NAK, and nothing is stored.
#[tokio::test]
async fn a_request_its_own_values_fail_to_store_is_answered_with_a_nak() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = super::direct_tests::ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(|conn| {
            conn.execute_batch(
                "CREATE TEMP TRIGGER refuse_requests BEFORE INSERT ON main.pending_friend_requests \
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
        })
        .await
        .unwrap();
    let frame = sealed_request(&bob, owner, [12; 16], now_ms());
    assert_eq!(
        super::direct_tests::call(&host, &frame).await,
        direct::NAK.to_vec()
    );
    assert_eq!(super::direct_tests::requests(&host, owner).await, 0);
}

/// RV5 R4, through `prekeys::on_rejected`: a friend's `PreKeyRejected`
/// naming no unanswered init of ours is a refusal (NAK), final, so its
/// nonce stays seen and a replay of it is a duplicate (the receipt), never
/// processed again.
#[tokio::test]
async fn an_ignored_prekey_rejection_is_refused_once_and_its_replay_is_a_duplicate() {
    use rekindle_codec::capnp_codec::direct::{DirectBody, PqPrekey, PreKeyRejected};
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = super::direct_tests::ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate();
    super::direct_tests::befriend(&host, owner, bob.public_key_bytes()).await;
    let body = DirectBody::PreKeyRejected(PreKeyRejected {
        signed_prekey_id: 1,
        one_time_prekey_id: None,
        pq_prekey: PqPrekey::LastResort(1),
        base_key_hash: [9; 32],
    });
    let frame = rekindle_transport::operations::dm::frame::seal_presession(
        &bob,
        &owner,
        &body,
        [9; 16],
        now_ms(),
    )
    .unwrap();
    let nak = direct::NAK.to_vec();
    assert_eq!(super::direct_tests::call(&host, &frame).await, nak);
    assert!(
        super::direct_tests::call(&host, &frame).await.is_empty(),
        "a replay of a refused rejection is a duplicate"
    );
}

/// RV5 R2: our deadline fires before Veilid stops waiting for our answer
/// (`rpc.timeout_ms`, `CALL_TIMEOUT_MS`), so a late call is ours to answer
/// retry later, never one Veilid already dropped.
#[test]
fn the_call_deadline_is_under_veilids_app_call_timeout() {
    assert!(
        direct::CALL_DEADLINE
            < std::time::Duration::from_millis(rekindle_types::message::CALL_TIMEOUT_MS)
    );
}

/// A shared buffer the scrubbed log lines land in.
#[derive(Clone, Default)]
pub(super) struct Captured(pub(super) std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);

thread_local! {
    /// The buffer this thread's events are captured into, while armed.
    static ARMED: std::cell::RefCell<Option<Captured>> = const { std::cell::RefCell::new(None) };
}

/// Writes to the armed thread's buffer; on any other thread, nowhere.
#[derive(Clone, Copy)]
struct ToArmed;

impl std::io::Write for ToArmed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        ARMED.with(|armed| {
            if let Some(captured) = armed.borrow().as_ref() {
                captured.0.lock().extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ToArmed {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

/// Disarms this thread when dropped.
pub(super) struct Armed;

impl Drop for Armed {
    fn drop(&mut self) {
        ARMED.with(|armed| armed.borrow_mut().take());
    }
}

/// Capture this thread's DEBUG events, through the log scrubber every
/// host installs, into `captured` until the guard drops.
///
/// Not a thread-local `set_default` subscriber: tracing-core 0.1.36 caches
/// a callsite's interest at its first hit (`DefaultCallsite::register`,
/// callsite.rs:308-321), and while one dispatcher is registered
/// (`has_just_one`, callsite.rs:551-557) asks only the hitting thread's
/// default (`Rebuilder::JustOne`, callsite.rs:544-549, 563-567). A thread
/// with none answers `Interest::never` (dispatcher.rs:379-397,
/// subscriber.rs:676-677), and the line is then skipped on every thread,
/// ours too: a parallel test first reaching the line after our
/// `set_default` lost it (1 run in 21). The callsite cannot be hit before
/// installing either, as `level_enabled!` stops it while the max level is
/// still OFF (metadata.rs:245). So the one global subscriber answers
/// `Interest::sometimes` for every callsite (`DynFilterFn`, filter_fn.rs:
/// 577-622; `Layered::pick_interest`, layered.rs:457-460) and decides per
/// event whether this thread is armed.
pub(super) fn capture(captured: &Captured) -> Armed {
    use tracing_subscriber::layer::SubscriberExt;
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(ToArmed))
            .finish()
            .with(tracing_subscriber::filter::dynamic_filter_fn(|_, _| {
                ARMED.with(|armed| armed.borrow().is_some())
            }));
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other subscriber in this test binary");
    });
    ARMED.with(|armed| *armed.borrow_mut() = Some(captured.clone()));
    Armed
}

/// Feed `host` a request from a fresh sender on a new thread, unarmed, so
/// the frame-received line is first hit there.
fn receive_on_another_thread(host: &std::sync::Arc<super::Host>, owner: [u8; 32], nonce: u8) {
    let sender = rekindle_crypto::Identity::generate();
    let frame = sealed_request(&sender, owner, [nonce; 16], now_ms());
    let datagram = rekindle_transport::operations::dm::wire(&frame).unwrap();
    let host = std::sync::Arc::clone(host);
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move { drop(host.receive_direct_call(&datagram).await) });
    })
    .join()
    .unwrap();
}

/// B19 guard for `1145a7e6`: the frame-received debug line, through the
/// log scrubber every host installs, holds no raw hex of the sender's key,
/// neither the whole key nor a hand-cut prefix (an 8-hex cut passes the
/// scrubber's 16-hex rule in the clear). Another thread reaches the line
/// first while we capture, the interleaving that lost it (see [`capture`]).
#[tokio::test]
async fn the_frame_received_line_holds_no_raw_sender_key() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = super::direct_tests::ann(&dir).await;
    let captured = Captured::default();
    let bob = rekindle_crypto::Identity::generate();
    let key = bob.public_key_bytes();
    let frame = sealed_request(&bob, owner, [10; 16], now_ms());
    let datagram = rekindle_transport::operations::dm::wire(&frame).unwrap();
    {
        let _armed = capture(&captured);
        receive_on_another_thread(&host, owner, 11);
        drop(host.receive_direct_call(&datagram).await);
    }
    let log = String::from_utf8(captured.0.lock().clone()).unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("1:1 frame received"))
        .expect("the debug line was written");
    for cut in [
        hex::encode(key),
        hex::encode(&key[..4]),
        hex::encode(&key[28..]),
    ] {
        assert!(!line.contains(&cut), "raw key hex {cut} in {line}");
    }
}

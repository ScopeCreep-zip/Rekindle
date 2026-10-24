//! A 1:1 call the Host could not judge is left unanswered, never NAKed
//! (RV2, `c37fef40`): a NAK is a refusal, which ends a first contact; no
//! answer lets the caller's `app_call` time out and its row be attempted
//! again.

use rekindle_transport::operations::direct;

use super::direct_tests::{now_ms, sealed_request, started};

/// A well-formed request to a Host with no identity unlocked: the
/// admission lookup cannot run, so nothing was judged.
#[tokio::test]
async fn a_call_the_host_cannot_judge_is_left_unanswered() {
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
            direct::Called::Unanswered
        ),
        "an internal error is no refusal"
    );
}

/// RV5 R1: a blocked sender's call is judged (a block is a lookup that
/// succeeded) and answered NAK (register :76, "a NAK means … admission"),
/// which frees the Veilid RPC worker at once; it is never left
/// unanswered, which would pin the worker for Veilid's 5 s timeout.
#[tokio::test]
async fn a_blocked_sender_is_answered_with_a_nak() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = super::direct_tests::ann(&dir).await;
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
    let datagram = rekindle_transport::operations::dm::wire(&frame).unwrap();
    match host.receive_direct_call(&datagram).await {
        direct::Called::Answered(reply) => assert_eq!(reply, direct::NAK.to_vec()),
        other => panic!("a block is a judged refusal, not {other:?}"),
    }
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
/// (`rpc.timeout_ms`, `CALL_TIMEOUT_MS`), so a late call is ours to leave
/// unanswered, never one Veilid already dropped.
#[test]
fn the_call_deadline_is_under_veilids_app_call_timeout() {
    assert!(
        direct::CALL_DEADLINE
            < std::time::Duration::from_millis(rekindle_types::message::CALL_TIMEOUT_MS)
    );
}

/// A shared buffer the scrubbed log lines land in.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// B19 guard for `1145a7e6`: the frame-received debug line, through the
/// log scrubber every host installs, holds no raw hex of the sender's key,
/// neither the whole key nor a hand-cut prefix (an 8-hex cut passes the
/// scrubber's 16-hex rule in the clear).
#[tokio::test]
async fn the_frame_received_line_holds_no_raw_sender_key() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = super::direct_tests::ann(&dir).await;
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(
            captured.clone(),
        ))
        .finish();
    let bob = rekindle_crypto::Identity::generate();
    let key = bob.public_key_bytes();
    let frame = sealed_request(&bob, owner, [10; 16], now_ms());
    let datagram = rekindle_transport::operations::dm::wire(&frame).unwrap();
    {
        let _guard = tracing::subscriber::set_default(subscriber);
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

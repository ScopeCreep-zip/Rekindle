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

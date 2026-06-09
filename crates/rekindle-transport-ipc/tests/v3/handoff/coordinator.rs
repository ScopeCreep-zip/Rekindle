use rekindle_transport_ipc::v3::handoff::coordinator::{
    HandoffCoordinator, HandoffOutcome, HandoffError, HandoffRejectReason,
};
use rekindle_transport_ipc::v3::handoff::transport::{LinuxTransport, LinuxMemfd, linux_pair};
use rekindle_transport_ipc::v3::handoff::credit::SideChannelCreditTracker;
use rekindle_transport_ipc::v3::handoff::sidechannel::SideChannel;

fn make_pair_with_credit(credit: u16) -> (
    HandoffCoordinator<LinuxTransport, LinuxMemfd>,
    HandoffCoordinator<LinuxTransport, LinuxMemfd>,
) {
    let (sc_a, sc_b) = SideChannel::create_pair().expect("socketpair");
    let (sender_transport, sender_memfd) = linux_pair(sc_a);
    let (receiver_transport, receiver_memfd) = linux_pair(sc_b);
    let handoff_key = [0xAA; 32];
    let sender = HandoffCoordinator::new(
        handoff_key,
        SideChannelCreditTracker::new(credit),
        sender_transport,
        sender_memfd,
    );
    let receiver = HandoffCoordinator::new(
        handoff_key,
        SideChannelCreditTracker::new(credit),
        receiver_transport,
        receiver_memfd,
    );
    (sender, receiver)
}

fn make_pair() -> (
    HandoffCoordinator<LinuxTransport, LinuxMemfd>,
    HandoffCoordinator<LinuxTransport, LinuxMemfd>,
) {
    make_pair_with_credit(8)
}

#[test]
fn begin_handoff_produces_offer_and_fd() {
    let (mut sender, _receiver) = make_pair();
    let payload = vec![0xDE; 1024];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(5, &payload, content_hash)
        .expect("begin_handoff must succeed");

    assert_eq!(offer.stream_id, 5);
    assert_ne!(offer.handoff_id, uuid::Uuid::nil());
    assert_eq!(offer.content_hash, content_hash);
    assert_ne!(offer.handoff_mac, [0u8; 32]);
}

#[test]
fn begin_handoff_with_wrong_hash_fails() {
    let (mut sender, _receiver) = make_pair();
    let payload = vec![0xBE; 2048];
    let wrong_hash = [0xFF; 32];

    let result = sender.begin_handoff(3, &payload, wrong_hash);
    assert!(result.is_err());
    match result.unwrap_err() {
        HandoffError::ContentHashMismatch { claimed, actual } => {
            assert_eq!(claimed, wrong_hash);
            assert_eq!(actual, *blake3::hash(&payload).as_bytes());
        }
        other => panic!("Expected ContentHashMismatch, got {other:?}"),
    }
}

#[test]
fn full_handoff_lifecycle() {
    let (mut sender, mut receiver) = make_pair();
    let payload = vec![0xBE; 2048];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(3, &payload, content_hash).unwrap();

    let accept = receiver.receive_offer(&offer).expect("receive_offer must succeed");
    assert_eq!(accept.verified_content_hash, content_hash);
    assert_eq!(accept.payload, payload);

    let outcome = sender.receive_accept(offer.handoff_id).unwrap();
    match outcome {
        HandoffOutcome::Delivered { stream_id, content_hash: ch, payload_id, .. } => {
            assert_eq!(stream_id, 3);
            assert_eq!(ch, content_hash);
            assert_eq!(payload_id, offer.payload_id);
        }
        other => panic!("Expected Delivered, got {other:?}"),
    }
}

#[test]
fn receive_accept_completes_handoff() {
    let (mut sender, mut receiver) = make_pair();
    let payload = vec![0xCC; 512];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(1, &payload, content_hash).unwrap();
    receiver.receive_offer(&offer).unwrap();
    let outcome = sender.receive_accept(offer.handoff_id).unwrap();
    match outcome {
        HandoffOutcome::Delivered { stream_id, content_hash: ch, .. } => {
            assert_eq!(stream_id, 1);
            assert_eq!(ch, content_hash);
        }
        other => panic!("Expected Delivered, got {other:?}"),
    }
}

#[test]
fn receive_reject_records_reason() {
    let (mut sender, _receiver) = make_pair();
    let payload = vec![0xDD; 512];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(2, &payload, content_hash).unwrap();
    let outcome = sender.receive_reject(
        offer.handoff_id,
        HandoffRejectReason::ContentHashMismatch,
    );
    match outcome {
        HandoffOutcome::Rejected { reason, stream_id } => {
            assert_eq!(reason, HandoffRejectReason::ContentHashMismatch);
            assert_eq!(stream_id, 2);
        }
        other => panic!("Expected Rejected, got {other:?}"),
    }
}

#[test]
fn cancel_pending_handoff() {
    let (mut sender, _receiver) = make_pair();
    let payload = vec![0xEE; 256];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(4, &payload, content_hash).unwrap();
    assert_eq!(sender.pending_count(), 1);
    sender.cancel(offer.handoff_id);
    assert_eq!(sender.pending_count(), 0);
}

#[test]
fn pending_count_tracks_active() {
    let (mut sender, _receiver) = make_pair();
    let h = *blake3::hash(b"a").as_bytes();

    let o1 = sender.begin_handoff(1, b"a", h).unwrap();
    sender.begin_handoff(2, b"a", h).unwrap();
    sender.begin_handoff(3, b"a", h).unwrap();
    assert_eq!(sender.pending_count(), 3);

    sender.receive_accept(o1.handoff_id).unwrap();
    assert_eq!(sender.pending_count(), 2);
}

#[test]
fn handoff_id_is_unique_per_handoff() {
    let (mut sender, _receiver) = make_pair();
    let h = *blake3::hash(b"x").as_bytes();

    let o1 = sender.begin_handoff(1, b"x", h).unwrap();
    let o2 = sender.begin_handoff(2, b"x", h).unwrap();
    let o3 = sender.begin_handoff(3, b"x", h).unwrap();
    assert_ne!(o1.handoff_id, o2.handoff_id);
    assert_ne!(o2.handoff_id, o3.handoff_id);
    assert_ne!(o1.handoff_id, o3.handoff_id);
}

#[test]
fn credit_consumed_on_begin() {
    let (mut sender, _receiver) = make_pair_with_credit(2);
    let h = *blake3::hash(b"y").as_bytes();

    sender.begin_handoff(1, b"y", h).unwrap();
    sender.begin_handoff(2, b"y", h).unwrap();
    let result = sender.begin_handoff(3, b"y", h);
    assert!(result.is_err());
    match result.unwrap_err() {
        HandoffError::CreditExhausted => {}
        other => panic!("Expected CreditExhausted, got {other:?}"),
    }
}

#[test]
fn offer_mac_with_wrong_key_rejected() {
    let (sc_a, sc_b) = SideChannel::create_pair().expect("socketpair");
    let (sender_transport, sender_memfd) = linux_pair(sc_a);
    let (receiver_transport, receiver_memfd) = linux_pair(sc_b);
    let mut sender = HandoffCoordinator::new(
        [0xAA; 32],
        SideChannelCreditTracker::new(8),
        sender_transport,
        sender_memfd,
    );
    let mut receiver = HandoffCoordinator::new(
        [0xBB; 32], // different key — MAC mismatch
        SideChannelCreditTracker::new(8),
        receiver_transport,
        receiver_memfd,
    );

    let payload = vec![0xFF; 512];
    let content_hash = *blake3::hash(&payload).as_bytes();
    let offer = sender.begin_handoff(1, &payload, content_hash).unwrap();

    let result = receiver.receive_offer(&offer);
    assert!(result.is_err());
    match result.unwrap_err() {
        HandoffError::MacFailed => {}
        other => panic!("Expected MacFailed, got {other:?}"),
    }
}

#[test]
fn double_accept_for_same_handoff_is_error() {
    let (mut sender, mut receiver) = make_pair();
    let payload = vec![0xAB; 256];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(1, &payload, content_hash).unwrap();
    receiver.receive_offer(&offer).unwrap();
    sender.receive_accept(offer.handoff_id).unwrap();

    let result = sender.receive_accept(offer.handoff_id);
    assert!(result.is_err());
    match result.unwrap_err() {
        HandoffError::NotFound(_) => {}
        other => panic!("Expected NotFound on double accept, got {other:?}"),
    }
}

#[test]
fn revoke_after_accept_is_noop() {
    let (mut sender, mut receiver) = make_pair();
    let payload = vec![0xCD; 256];
    let content_hash = *blake3::hash(&payload).as_bytes();

    let offer = sender.begin_handoff(1, &payload, content_hash).unwrap();
    receiver.receive_offer(&offer).unwrap();
    sender.receive_accept(offer.handoff_id).unwrap();
    sender.cancel(offer.handoff_id);
    assert_eq!(sender.pending_count(), 0);
}

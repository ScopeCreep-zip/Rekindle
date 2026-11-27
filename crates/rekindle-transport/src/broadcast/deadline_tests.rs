use super::*;
use rekindle_types::config::ANONYMITY_HOP_FLOOR;

fn at(hop_count: u8) -> SafetyProfile {
    SafetyProfile {
        hop_count,
        ..rekindle_route::profile::profile_for_class(rekindle_types::message::MessageClass::Text)
    }
}

/// `answer_wait` equals Veilid 0.5.8's `routed_timeout(timeout_ms,
/// 2 * hops)` (`rpc_processor/mod.rs:109-117`, 10% of the timeout per
/// hop past 4) at every hop count Veilid allows, and at our 5 s timeout
/// that is the 500 ms per hop 0.5.7 added as a constant.
#[test]
fn the_answer_wait_is_veilids_routed_timeout() {
    assert_eq!(CALL_TIMEOUT_MS, 5_000, "Veilid's rpc.timeout_ms");
    for hops in 1..=4u8 {
        let effective = (2 * u64::from(hops.max(ANONYMITY_HOP_FLOOR))).saturating_sub(4);
        let routed = CALL_TIMEOUT_MS + CALL_TIMEOUT_MS * 10 / 100 * effective;
        assert_eq!(answer_wait(&at(hops)), Duration::from_millis(routed));
        let legacy = CALL_TIMEOUT_MS + 500 * effective;
        assert_eq!(answer_wait(&at(hops)), Duration::from_millis(legacy));
    }
}

#[test]
fn the_deadline_is_past_veilids_own_wait() {
    assert_eq!(answer_wait(&at(1)), Duration::from_secs(6), "floor-clamped");
    assert_eq!(answer_wait(&at(3)), Duration::from_secs(6));
    assert_eq!(answer_wait(&at(4)), Duration::from_secs(7));
    assert_eq!(app_call_deadline(&at(3)), Duration::from_secs(11));
    assert!(app_call_deadline(&at(4)) > answer_wait(&at(4)));
}

/// A port whose calls and sends never resolve.
struct Hung;

#[async_trait::async_trait]
impl AppMessagePort for Hung {
    async fn send(&self, _: &PeerTarget, _: &SafetyProfile, _: Vec<u8>) -> Result<(), PortError> {
        std::future::pending().await
    }
    async fn call(
        &self,
        _: &PeerTarget,
        _: &SafetyProfile,
        _: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        std::future::pending().await
    }
    async fn reply(&self, _: AppCallId, _: Vec<u8>) -> Result<(), PortError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_call_that_never_resolves_times_out_at_the_deadline() {
    let port = Bounded::wrap(Arc::new(Hung));
    let target = PeerTarget::new("route".into());
    let profile = at(3);
    let started = tokio::time::Instant::now();
    let called = port.call(&target, &profile, vec![1]).await;
    assert!(matches!(called, Err(PortError::Timeout(ref m)) if m.contains("unresolved")));
    assert_eq!(started.elapsed(), app_call_deadline(&profile));
}

#[tokio::test(start_paused = true)]
async fn a_send_that_never_dispatches_fails_at_its_deadline() {
    let port = Bounded::wrap(Arc::new(Hung));
    let target = PeerTarget::new("route".into());
    let profile = at(3);
    let started = tokio::time::Instant::now();
    let sent = port.send(&target, &profile, vec![1]).await;
    assert!(matches!(sent, Err(PortError::Send(ref m)) if m.contains("not dispatched")));
    assert_eq!(started.elapsed(), app_message_deadline());
}

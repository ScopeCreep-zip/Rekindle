//! Our own deadline on every outbound `app_call` (live finding 78).
//!
//! Veilid times out only the wait for an answer: `rpc_call_app_call` awaits
//! `wait_for_reply`, which waits `get_safety_selection_timeout`
//! (veilid-core 0.5.8 `rpc_processor/mod.rs:1985-1994`, `:637-700`). Everything before it is
//! unbounded: the destination and its reply route are resolved
//! (`get_destination`, `get_destination_respond_to`), the question is
//! rendered and its safety route compiled (`render_signed_operation`), and
//! the envelope goes to the first hop (`send_envelope`), each an `.await`
//! with no deadline of its own (`rpc_processor/mod.rs:2008-2105`,
//! `rpc_app_call.rs:20-36`). A call stuck in one of them never resolves:
//! live, a receipted `FriendRequest` to a dead route logged
//! "1:1 app_call sent" and then neither an answer nor a Timeout for 14+
//! minutes, and its handshake row and the stream behind it waited with it.
//!
//! So the transport bounds every outbound call at the port it is given,
//! one place for every caller (the 1:1 rows, RPCs, community envelopes):
//! [`Bounded`] wraps the node's [`AppMessagePort`] when the node is built.
//! A bound that fires is a timeout, the outcome unknown, reported as Veilid's
//! own `Timeout` is (`PortError::Timeout`, `rekindle-protocol` `ports.rs`
//! `send_error`), so each caller's timeout handling applies unchanged: the
//! route is refreshed and the row attempted again.
//!
//! Dropping the Veilid future at the bound does not cancel the question:
//! Veilid spawns the answer wait into its reply pool so that a dropped caller
//! leaves it to finish (`rpc_processor/mod.rs:662-677`). Veilid logs the
//! dropped call's `DurationRecorder` (`logging/duration_recorder.rs:84-90`);
//! that line appears only for a call Veilid itself left unresolved.
//!
//! An `app_message` goes through the same unbounded stages (destination,
//! render and safety-route compile, `send_envelope` to the first hop:
//! `rpc_processor/mod.rs:2196`) and has no answer wait at all, so a
//! send is bounded at [`app_message_deadline`]. A send that hits it is a
//! send error, `PortError::Send`, as Veilid's own send errors are, and its
//! callers handle it as they handle those.
//!
//! The port cannot see the moment Veilid starts waiting for the answer, so
//! each call's pre-step time is logged at DEBUG from what it can see: a
//! Veilid `Timeout` arrives exactly [`answer_wait`] after the question left,
//! so its pre-steps are the elapsed time less that wait; an answered call's
//! pre-steps are at most its elapsed time; a call at our bound spent at
//! least the bound less the wait before it (live finding 78, L1).

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortError};
use rekindle_types::config::SafetyProfile;
use rekindle_types::message::CALL_TIMEOUT_MS;
use tracing::{debug, warn};

/// Veilid's wait for one answer at a profile's hop count (lives in
/// `rekindle-types` so the governance runtime sizes its bootstrap cap from
/// the same rule, plan E3.2 S7c.8 (i)).
pub use rekindle_types::config::answer_wait;

/// Our bound on one outbound `app_call` at `profile`: Veilid's answer wait
/// ([`answer_wait`]) plus one more `CALL_TIMEOUT_MS` for the stages before
/// it that Veilid does not bound (route resolution, rendering, the first
/// hop's send, each of which is one RPC-scale step). 11 s at our 3-hop
/// floor: past any call Veilid answers or times out itself, so the bound
/// fires only on a call Veilid would leave unresolved.
#[must_use]
pub fn app_call_deadline(profile: &SafetyProfile) -> Duration {
    answer_wait(profile) + Duration::from_millis(CALL_TIMEOUT_MS)
}

/// Our bound on one outbound `app_message`: one `CALL_TIMEOUT_MS` for the
/// stages Veilid does not bound (there is no answer wait), 5 s.
#[must_use]
pub fn app_message_deadline() -> Duration {
    Duration::from_millis(CALL_TIMEOUT_MS)
}

/// The node's app-message port with [`app_call_deadline`] on every call
/// and [`app_message_deadline`] on every send.
pub(crate) struct Bounded {
    inner: Arc<dyn AppMessagePort>,
}

impl Bounded {
    pub(crate) fn wrap(inner: Arc<dyn AppMessagePort>) -> Arc<dyn AppMessagePort> {
        Arc::new(Self { inner })
    }
}

/// `call` within `profile`'s deadline; past it, a timeout.
async fn within(
    profile: &SafetyProfile,
    call: impl std::future::Future<Output = Result<Vec<u8>, PortError>>,
) -> Result<Vec<u8>, PortError> {
    let deadline = app_call_deadline(profile);
    let wait = answer_wait(profile);
    let started = tokio::time::Instant::now();
    let Ok(result) = tokio::time::timeout(deadline, call).await else {
        debug!(
            pre_step_min_ms = deadline.saturating_sub(wait).as_millis(),
            elapsed_ms = deadline.as_millis(),
            "app_call pre-steps: unresolved at our bound"
        );
        warn!(
            deadline_ms = deadline.as_millis(),
            "app_call unresolved at our deadline: timed out, outcome unknown"
        );
        return Err(PortError::Timeout(format!(
            "app call unresolved after {} ms (outcome unknown)",
            deadline.as_millis()
        )));
    };
    log_pre_steps(&result, started.elapsed(), wait);
    result
}

/// The DEBUG line for one call that Veilid resolved itself.
fn log_pre_steps(result: &Result<Vec<u8>, PortError>, elapsed: Duration, wait: Duration) {
    match result {
        // Veilid's answer wait ran in full after the question left.
        Err(PortError::Timeout(_)) => debug!(
            pre_step_ms = elapsed.saturating_sub(wait).as_millis(),
            elapsed_ms = elapsed.as_millis(),
            "app_call pre-steps: Veilid timed out the answer wait"
        ),
        Ok(_) => debug!(
            pre_step_max_ms = elapsed.as_millis(),
            elapsed_ms = elapsed.as_millis(),
            "app_call pre-steps: answered"
        ),
        Err(_) => debug!(
            pre_step_max_ms = elapsed.as_millis(),
            elapsed_ms = elapsed.as_millis(),
            "app_call pre-steps: failed before an answer"
        ),
    }
}

/// `send` within [`app_message_deadline`]; past it, a send error.
async fn sent_within(
    send: impl std::future::Future<Output = Result<(), PortError>>,
) -> Result<(), PortError> {
    let deadline = app_message_deadline();
    tokio::time::timeout(deadline, send)
        .await
        .unwrap_or_else(|_| {
            warn!(
                deadline_ms = deadline.as_millis(),
                "app_message not dispatched at our deadline: send failed"
            );
            Err(PortError::Send(format!(
                "app message not dispatched after {} ms",
                deadline.as_millis()
            )))
        })
}

#[async_trait::async_trait]
impl AppMessagePort for Bounded {
    async fn send(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        sent_within(self.inner.send(target, profile, message)).await
    }

    async fn call(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        within(profile, self.inner.call(target, profile, message)).await
    }

    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        self.inner.reply(call_id, message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::config::ANONYMITY_HOP_FLOOR;

    fn at(hop_count: u8) -> SafetyProfile {
        SafetyProfile {
            hop_count,
            ..rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile()
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
        async fn send(
            &self,
            _: &PeerTarget,
            _: &SafetyProfile,
            _: Vec<u8>,
        ) -> Result<(), PortError> {
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
}

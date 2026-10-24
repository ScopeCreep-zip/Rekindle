//! Our own deadline on every outbound `app_call` (live finding 78).
//!
//! Veilid times out only the wait for an answer: `rpc_call_app_call` awaits
//! `wait_for_reply`, which waits `get_safety_selection_timeout`
//! (`rpc_processor/mod.rs:1688-1704`, `:580-640`). Everything before it is
//! unbounded: the destination and its reply route are resolved
//! (`get_destination`, `get_destination_respond_to`), the question is
//! rendered and its safety route compiled (`render_signed_operation`), and
//! the envelope goes to the first hop (`send_envelope`), each an `.await`
//! with no deadline of its own (`rpc_processor/mod.rs:1718-1830`,
//! `rpc_app_call.rs:21-36`). A call stuck in one of them never resolves:
//! live, a receipted `FriendRequest` to a dead route logged
//! "1:1 app_call sent" and then neither an answer nor a Timeout for 14+
//! minutes, and its handshake row and the stream behind it waited with it.
//!
//! So the transport bounds every outbound call at the port it is given,
//! one place for every caller (the 1:1 rows, RPCs, community envelopes):
//! [`Bounded`] wraps the node's [`AppMessagePort`] when the node is built.
//! A bound that fires is a timeout, the outcome unknown, reported as Veilid's
//! own `Timeout` is (`PortError::Send`, `rekindle-protocol` `ports.rs`
//! `send_error`), so each caller's timeout handling applies unchanged: the
//! route is refreshed and the row attempted again.
//!
//! Dropping the Veilid future at the bound does not cancel the question:
//! Veilid spawns the answer wait into its reply pool so that a dropped caller
//! leaves it to finish (`rpc_processor/mod.rs:606-614`). Veilid logs the
//! dropped call's `DurationRecorder` (`logging/duration_recorder.rs:84-90`);
//! that line appears only for a call Veilid itself left unresolved.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortError, RouteClass};
use rekindle_types::config::{SafetyProfile, ANONYMITY_HOP_FLOOR};
use rekindle_types::message::CALL_TIMEOUT_MS;
use tracing::warn;

/// Veilid's wait for one answer at `profile`'s hop count, floor-clamped as
/// the port clamps it: `rpc.timeout_ms` (`CALL_TIMEOUT_MS`) plus 500 ms for
/// each effective hop (twice the safety route's) above four
/// (`get_safety_selection_timeout`, `rpc_processor/mod.rs:1688-1704`); 6 s at
/// our 3-hop floor, 7 s at Veilid's 4-hop maximum (`max_route_hop_count`,
/// `veilid_config.rs:853`).
#[must_use]
pub fn answer_wait(profile: &SafetyProfile) -> Duration {
    let hops = u64::from(profile.hop_count.max(ANONYMITY_HOP_FLOOR));
    Duration::from_millis(CALL_TIMEOUT_MS + 500 * (2 * hops).saturating_sub(4))
}

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

/// The node's app-message port with [`app_call_deadline`] on every call.
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
    tokio::time::timeout(deadline, call)
        .await
        .unwrap_or_else(|_| {
            warn!(
                deadline_ms = deadline.as_millis(),
                "app_call unresolved at our deadline: timed out, outcome unknown"
            );
            Err(PortError::Send(format!(
                "app call timed out: unresolved after {} ms (outcome unknown)",
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
        self.inner.send(target, profile, message).await
    }

    async fn call(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        within(profile, self.inner.call(target, profile, message)).await
    }

    async fn send_pinned(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        self.inner
            .send_pinned(target, profile, class, message)
            .await
    }

    async fn call_pinned(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        within(
            profile,
            self.inner.call_pinned(target, profile, class, message),
        )
        .await
    }

    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        self.inner.reply(call_id, message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hop_count: u8) -> SafetyProfile {
        SafetyProfile {
            hop_count,
            ..rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile()
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

    /// A port whose calls never resolve.
    struct Hung;

    #[async_trait::async_trait]
    impl AppMessagePort for Hung {
        async fn send(
            &self,
            _: &PeerTarget,
            _: &SafetyProfile,
            _: Vec<u8>,
        ) -> Result<(), PortError> {
            Ok(())
        }
        async fn call(
            &self,
            _: &PeerTarget,
            _: &SafetyProfile,
            _: Vec<u8>,
        ) -> Result<Vec<u8>, PortError> {
            std::future::pending().await
        }
        async fn send_pinned(
            &self,
            _: &PeerTarget,
            _: &SafetyProfile,
            _: RouteClass,
            _: Vec<u8>,
        ) -> Result<(), PortError> {
            Ok(())
        }
        async fn call_pinned(
            &self,
            _: &PeerTarget,
            _: &SafetyProfile,
            _: RouteClass,
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
        let pinned = port
            .call_pinned(&target, &profile, RouteClass::Sending, vec![1])
            .await;
        assert!(matches!(pinned, Err(PortError::Send(ref m)) if m.contains("timed out")));
        assert_eq!(started.elapsed(), app_call_deadline(&profile));
        assert!(port.call(&target, &profile, vec![1]).await.is_err());
    }
}

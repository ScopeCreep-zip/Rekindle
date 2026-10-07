//! `DaemonContext` inherent methods: request-handling helpers shared across
//! dispatch submodules, the state-violation error constructor, and the
//! daemon's own bus-subscriber loop.

use std::sync::Arc;

use rekindle_transport::{Session, TransportNode};

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::{BusPayload, IpcRequest, IpcResponse, Lane};

use crate::daemon::shutdown::{ExitReason, REQUEST_DRAIN_DEADLINE};

use super::in_flight::{shutting_down, Correlation, Finished, InFlight};
use super::{CallerContext, DaemonContext};

// ── Shared helpers used across dispatch submodules ───────────────────────

impl DaemonContext {
    /// Get a reference to the transport node, or return a 503 error response.
    pub(crate) fn require_transport(&self) -> Result<Arc<TransportNode>, IpcResponse> {
        self.transport.read().clone().ok_or_else(|| {
            IpcResponse::error_with_remediation(
                503,
                "transport not started — daemon not yet operational",
                "wait for the daemon to reach operational state, then retry",
            )
        })
    }

    /// Briefly hold the session lock, apply a closure, or return 404.
    pub(crate) fn require_session<F, T>(&self, f: F) -> Result<T, IpcResponse>
    where
        F: FnOnce(&Session) -> T,
    {
        let guard = self.session.read();
        guard.as_ref().map(f).ok_or_else(|| {
            IpcResponse::error_with_remediation(
                404,
                "no identity loaded",
                "initialize an identity first",
            )
        })
    }

    /// Get the signing key bytes, or return 403 if locked.
    pub(crate) fn require_signing_key(&self) -> Result<[u8; 32], IpcResponse> {
        let guard = self.signing_key.read();
        guard.as_ref().map(|k| *k.as_bytes()).ok_or_else(|| {
            IpcResponse::error_with_remediation(
                403,
                "signing key not available — daemon is locked",
                "unlock the daemon first",
            )
        })
    }

    /// Save session to disk, returning an IPC error on failure.
    pub(crate) fn save_session(&self) -> Result<(), IpcResponse> {
        let guard = self.session.read();
        if let Some(ref session) = *guard {
            crate::state::save_session(session, &self.session_path)
                .map_err(|e| IpcResponse::error(500, format!("session persistence failed: {e}")))
        } else {
            Ok(())
        }
    }

    /// Resolve a community by governance key or name from session.
    pub(crate) fn resolve_community(
        &self,
        target: &str,
    ) -> Result<rekindle_transport::CommunityMembership, IpcResponse> {
        let guard = self.session.read();
        let session = guard
            .as_ref()
            .ok_or_else(|| IpcResponse::error(404, "no identity loaded"))?;
        // Exact governance key match
        if let Some(m) = session.community(target) {
            return Ok(m.clone());
        }
        // Case-insensitive name match
        if let Some(m) = session.community_by_name(target) {
            return Ok(m.clone());
        }
        Err(IpcResponse::error(
            404,
            format!("community '{target}' not found"),
        ))
    }
}

/// Move the lifecycle to `next`, or answer 409 if the FSM refuses the edge.
pub(crate) fn transition(ctx: &DaemonContext, next: DaemonState) -> Result<(), IpcResponse> {
    ctx.lifecycle
        .transition(next)
        .map(drop)
        .map_err(|e| IpcResponse::error(409, e.to_string()))
}

/// Produce a standard error response for state violations.
pub(crate) fn state_error(state: DaemonState, required: &str) -> IpcResponse {
    IpcResponse::error_with_remediation(
        409,
        format!(
            "cannot perform {required} operation in state '{}'",
            state.as_str()
        ),
        if state == DaemonState::Locked {
            "unlock the daemon first"
        } else {
            "wait for the daemon to reach operational state"
        },
    )
}

// ── Daemon bus subscriber ───────────────────────────────────────────────

impl DaemonContext {
    /// Run the daemon as a bus subscriber.
    ///
    /// Connects to the daemon's own IPC socket as a privileged internal
    /// agent, receives `BusPayload::Request` messages routed by the server,
    /// dispatches each as its own task in its lane, and sends correlated
    /// `BusPayload::Response` messages back through the bus as they finish.
    /// An interval arm beats the heartbeat the watchdog checks.
    ///
    /// This method runs until the bus connection is closed (daemon shutdown).
    pub async fn run_subscriber(
        self: &std::sync::Arc<Self>,
        mut client: rekindle_ipc::client::BusClient,
    ) {
        tracing::info!("daemon bus subscriber started");
        let mut in_flight = InFlight::default();
        let mut tick = tokio::time::interval(self.subscriber_heartbeat.tick());

        loop {
            tokio::select! {
                () = self.shutdown.requested() => break,
                msg = client.recv_bus_message() => match msg {
                    Some(Ok(msg)) => {
                        let BusPayload::Request(request) = msg.payload else {
                            tracing::debug!("daemon subscriber: non-request payload, ignoring");
                            continue;
                        };
                        let correlation = Correlation {
                            id: msg.msg_id,
                            level: msg.security_level,
                        };
                        let caller = CallerContext {
                            verified_name: msg.verified_sender_name,
                            static_key: msg.verified_sender_key,
                            level: msg.security_level,
                        };
                        let ctx = Arc::clone(self);
                        in_flight.spawn(correlation, async move {
                            ctx.dispatch_in_lane(request, &caller).await
                        });
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "daemon subscriber: decode failed, skipping");
                    }
                    None => {
                        tracing::info!("daemon subscriber: bus connection closed");
                        break;
                    }
                },
                Some(done) = in_flight.next() => self.answer(&client, done).await,
                _ = tick.tick() => self.subscriber_heartbeat.beat(),
            }
        }

        self.drain(&client, &mut in_flight).await;
        client.shutdown().await;
        tracing::info!("daemon bus subscriber stopped");
    }

    /// Send a finished request's response. A handler panic may have left
    /// shared state half-mutated, so after answering it the daemon shuts
    /// down for its supervisor to restart it from fresh state.
    async fn answer(&self, client: &rekindle_ipc::client::BusClient, done: Finished) {
        respond(client, done.correlation, &done.response).await;
        if done.panicked {
            self.shutdown.request(ExitReason::HandlerPanic);
        }
    }

    /// Answer every request still in flight: let them finish until
    /// [`REQUEST_DRAIN_DEADLINE`], then abort the rest and answer 503.
    async fn drain(&self, client: &rekindle_ipc::client::BusClient, in_flight: &mut InFlight) {
        let deadline = tokio::time::Instant::now() + REQUEST_DRAIN_DEADLINE;
        while let Ok(Some(done)) = tokio::time::timeout_at(deadline, in_flight.next()).await {
            self.answer(client, done).await;
        }
        for correlation in in_flight.abort_all().await {
            respond(client, correlation, &shutting_down()).await;
        }
    }

    /// Dispatch `request` holding its lane for the whole handler.
    async fn dispatch_in_lane(&self, request: IpcRequest, caller: &CallerContext) -> IpcResponse {
        match request.lane() {
            Lane::Query => super::router::dispatch(self, request, caller).await,
            Lane::Write => {
                let _lane = self.write_lane.read().await;
                super::router::dispatch(self, request, caller).await
            }
            Lane::Exclusive => {
                let _lane = self.write_lane.write().await;
                super::router::dispatch(self, request, caller).await
            }
        }
    }
}

/// Send `response` back to the requester `correlation` names.
async fn respond(
    client: &rekindle_ipc::client::BusClient,
    correlation: Correlation,
    response: &IpcResponse,
) {
    // Serialize IpcResponse to JSON bytes for the bus wire format.
    // IpcResponse contains serde_json::Value which postcard cannot handle.
    let response_bytes = match serde_json::to_vec(response) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "daemon subscriber: failed to serialize response");
            return;
        }
    };
    if let Err(e) = client
        .respond(response_bytes, correlation.id, correlation.level)
        .await
    {
        tracing::error!(error = %e, "daemon subscriber: failed to send response");
    }
}

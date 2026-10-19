//! `DaemonContext` inherent methods: request-handling helpers shared across
//! dispatch submodules, the state-violation error constructor, and the
//! daemon's own bus-subscriber loop.

use std::sync::Arc;

use rekindle_transport::{Session, TransportNode};

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::{BusPayload, IpcRequest, IpcResponse, Lane};

use crate::daemon::shutdown::{ExitReason, REQUEST_DRAIN_DEADLINE};

use super::in_flight::{shutting_down, Correlation, Finished, InFlight};
use super::{Caller, DaemonContext};

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

    /// Briefly hold the session lock, apply a closure, or return 423
    /// (the session exists only while an identity is unlocked).
    pub(crate) fn require_session<F, T>(&self, f: F) -> Result<T, IpcResponse>
    where
        F: FnOnce(&Session) -> T,
    {
        let guard = self.session.read();
        guard.as_ref().map(f).ok_or_else(|| {
            IpcResponse::error_with_remediation(
                423,
                "no identity is unlocked",
                "unlock an identity first (rekindle identity list, then rekindle unlock)",
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

    /// The unlocked identity's database, or 423 while locked.
    pub(crate) fn require_db(&self) -> Result<rekindle_db::Db, IpcResponse> {
        self.db.current().map_err(|_| {
            IpcResponse::error_with_remediation(
                423,
                "no identity is unlocked",
                "unlock the daemon first",
            )
        })
    }

    /// The unlocked identity's public key, or 423 while locked.
    pub(crate) fn require_owner(&self) -> Result<String, IpcResponse> {
        self.require_session(|s| s.identity.public_key_hex.clone())
    }

    /// The host's identity slots, for `host::identity`.
    pub(crate) fn identity_stores(&self) -> crate::host::identity::Stores<'_> {
        crate::host::identity::Stores {
            db: &self.db,
            vault: &self.vault,
            attempts: &self.identity_attempts,
        }
    }

    /// The unlock's Signal manager, or 423 while locked.
    pub(crate) fn require_signal(
        &self,
    ) -> Result<Arc<rekindle_crypto::signal::SignalSessionManager>, IpcResponse> {
        self.signal.read().clone().ok_or_else(|| {
            IpcResponse::error_with_remediation(
                423,
                "no identity is unlocked",
                "unlock the daemon first",
            )
        })
    }

    /// Resolve a community by governance key or name from session.
    pub(crate) fn resolve_community(
        &self,
        target: &str,
    ) -> Result<rekindle_transport::CommunityMembership, IpcResponse> {
        let guard = self.session.read();
        let session = guard
            .as_ref()
            .ok_or_else(|| IpcResponse::error(423, "no identity is unlocked"))?;
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
                        // A frame the server did not stamp with its
                        // connection cannot be answered for one.
                        let Some(conn) = msg.verified_sender_conn else {
                            tracing::warn!("daemon subscriber: unstamped request, ignoring");
                            continue;
                        };
                        let caller = Caller {
                            conn: rekindle_ipc::ConnId::Bus(conn),
                            clearance: msg.security_level,
                            verified_name: msg.verified_sender_name,
                            static_key: msg.verified_sender_key,
                        };
                        let ctx = Arc::clone(self);
                        in_flight.spawn(correlation, async move {
                            ctx.call(request, &caller).await
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

    /// `HostApi::call`: the sender's clearance (stamped by the bus server
    /// from the registry, or the shell's) must meet the request's (plan D1),
    /// and an agent's capabilities admit it.
    pub(crate) async fn call(&self, request: IpcRequest, caller: &Caller) -> IpcResponse {
        let required = request.required_clearance();
        if caller.clearance < required {
            return IpcResponse::error_with_remediation(
                403,
                format!(
                    "{} needs {required:?} clearance; this client has {:?}",
                    request.name(),
                    caller.clearance
                ),
                "unlock the identity from this client first",
            );
        }
        // An agent is scoped by its capabilities above the locked-state
        // requests (plan D1).
        if required > rekindle_ipc::message::SecurityLevel::Open
            && caller.clearance == rekindle_ipc::message::SecurityLevel::Agent
        {
            let permitted = match caller.static_key {
                Some(key) => self
                    .registry
                    .read()
                    .await
                    .lookup(&key)
                    .is_some_and(|identity| identity.permits(request.agent_capability())),
                None => false,
            };
            if !permitted {
                return IpcResponse::error(
                    403,
                    format!(
                        "{} is not within this agent's capabilities{}",
                        request.name(),
                        request.agent_capability().map_or(
                            String::from(" (no agent may send it)"),
                            |c| format!(" (needs {})", c.as_str())
                        )
                    ),
                );
            }
        }
        // Then `validate_request`, the shell's refusals and the intent gate
        // (plan D3.3), which dispatches what may run.
        crate::host::gate::run(self, request, caller).await
    }

    /// Dispatch `request` holding its lane for the whole handler.
    pub(crate) async fn dispatch_in_lane(
        &self,
        request: IpcRequest,
        caller: &Caller,
    ) -> IpcResponse {
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

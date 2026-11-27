//! `DaemonContext` inherent methods: request-handling helpers shared across
//! dispatch submodules, the state-violation error constructor, and the
//! daemon's own bus-subscriber loop.

use std::sync::Arc;

use rekindle_transport::{Session, TransportNode};

use crate::daemon::DaemonState;
use crate::host::gate::Acknowledged;
use crate::host::skin::actions::Resolved;
use rekindle_ipc::protocol::{BusPayload, IpcRequest, IpcResponse, Lane};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

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
            device_vault: &self.device_vault,
            attempts: &self.identity_attempts,
        }
    }

    /// Resolve a community by anchor key or name from session.
    pub(crate) fn resolve_community(
        &self,
        target: &str,
    ) -> Result<rekindle_transport::CommunityMembership, IpcResponse> {
        let guard = self.session.read();
        let session = guard
            .as_ref()
            .ok_or_else(|| IpcResponse::error(423, "no identity is unlocked"))?;
        // Exact anchor key match
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

/// A governance runtime error as the answer: 403 when we lack the
/// permission, 400 when the entry is malformed or out of bounds, 500
/// otherwise.
pub(crate) fn runtime_refused(
    e: &rekindle_governance_runtime::GovernanceRuntimeError,
    what: &str,
) -> IpcResponse {
    match e {
        rekindle_governance_runtime::GovernanceRuntimeError::PermissionDenied => {
            IpcResponse::error(403, format!("{what}: {e}"))
        }
        rekindle_governance_runtime::GovernanceRuntimeError::Malformed
        | rekindle_governance_runtime::GovernanceRuntimeError::OutOfBounds { .. }
        | rekindle_governance_runtime::GovernanceRuntimeError::NoMessageId => {
            IpcResponse::error(400, format!("{what}: {e}"))
        }
        _ => IpcResponse::error(500, format!("{what}: {e}")),
    }
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
            // Biased: shutdown, the heartbeat, finished requests, then new
            // ones. New requests go last because they can arrive without
            // pause: ahead of the tick they would stop the beat of a loop
            // that is not stuck (the watchdog would restart the daemon),
            // and ahead of the answers they would hold every reply off.
            // The tick is ready once a period and each answer once, so
            // neither starves the requests.
            tokio::select! {
                biased;
                () = self.shutdown.requested() => break,
                _ = tick.tick() => self.subscriber_heartbeat.beat(),
                Some(done) = in_flight.next() => self.answer(&client, done).await,
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
                        in_flight.spawn(correlation, request.name(), async move {
                            ctx.call(*request, &caller).await
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
    /// [`InFlight::drain`] logs which kinds it waited for and cut off.
    async fn drain(&self, client: &rekindle_ipc::client::BusClient, in_flight: &mut InFlight) {
        let drained = in_flight
            .drain(REQUEST_DRAIN_DEADLINE, |done| self.answer(client, done))
            .await;
        if !drained.waited.is_empty() {
            tracing::info!(
                answered = drained.waited.len() - drained.aborted.len(),
                aborted = drained.aborted.len(),
                "shutdown drain finished"
            );
        }
        for (correlation, _) in drained.aborted {
            respond(client, correlation, &shutting_down()).await;
        }
    }

    /// `HostApi::call`: the sender's clearance (stamped by the bus server
    /// from the registry, or the shell's) must meet the request's (plan D1),
    /// and an agent's capabilities admit it.
    /// A skin press is traced with what it mapped to and its answer
    /// (finding OBS1, [`crate::host::skin::press_trace`]).
    pub(crate) async fn call(&self, request: IpcRequest, caller: &Caller) -> IpcResponse {
        let press = crate::host::skin::press_trace::Press::of(&request);
        let mut mapped = None;
        let answer = self.call_resolved(request, caller, &mut mapped).await;
        if let Some(press) = press {
            press.trace(mapped.as_deref(), &answer);
        }
        answer
    }

    /// [`Self::call`]'s work; `mapped` gets what a skin action resolved to.
    async fn call_resolved(
        &self,
        request: IpcRequest,
        caller: &Caller,
        mapped: &mut Option<String>,
    ) -> IpcResponse {
        // A skin action that is a feature becomes its own request first
        // (plan SK2-A4), so that request meets its own clearance and gate.
        let skin = &self.skin;
        let reads =
            match crate::host::skin::reads::load(self, &skin.selection, &skin.launchers, &request)
                .await
            {
                Ok(reads) => reads,
                Err(refused) => return refused,
            };
        // The action resolves here only (with its reads made once): a run
        // is the answer once the request is admitted.
        let (request, run) = match crate::host::skin::actions::resolve(&self.skin, &reads, request)
        {
            Ok(Resolved::Request(request)) => {
                *mapped = Some(crate::host::skin::press_trace::request_kind(&request));
                (request, None)
            }
            Ok(Resolved::Run { request, run }) => {
                *mapped = Some(crate::host::skin::press_trace::run_kind(&run));
                (request, Some(run))
            }
            Err(refused) => return refused,
        };
        let pressed_in = match &request {
            IpcRequest::RunSkinAction { window, .. } => Some(window.clone()),
            _ => None,
        };
        let required = request.required_clearance();
        if caller.clearance < required {
            return self.clearance_refusal(&request, required, caller);
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
        let answered = crate::host::gate::run(self, request, caller).await;
        match (run, pressed_in, &answered) {
            (Some(run), Some(window), IpcResponse::Ok { .. }) => {
                let (run, elsewhere) =
                    crate::host::skin::actions::addressed(&self.skin.launchers, &window, run);
                self.skin.views.record(&window, &run, elsewhere.as_ref());
                if let Some((window, op)) = elsewhere {
                    self.publish_event(SubscriptionEvent::System(SystemEvent::SkinShell {
                        window,
                        op,
                    }));
                }
                IpcResponse::ok(&run)
            }
            _ => answered,
        }
    }

    /// Why `caller` lacks `required` (plan D1). A client's clearance lasts
    /// as long as the unlock: the registry holds the identity's clients
    /// from unlock to lock (`daemon::clients::{load, unload}`), and nothing
    /// lapses in between. So a refused client either finds no identity
    /// unlocked (the daemon starts locked, and a lock, logout or restart
    /// ends the unlock: live R5-4, a restarted daemon), or holds a key the
    /// unlocked identity does not trust.
    fn clearance_refusal(
        &self,
        request: &IpcRequest,
        required: rekindle_ipc::message::SecurityLevel,
        caller: &Caller,
    ) -> IpcResponse {
        if self.session.read().is_none() {
            return IpcResponse::error_with_remediation(
                403,
                format!(
                    "{} needs an unlocked identity, and this daemon holds none: \
                     a daemon starts locked, and a lock, logout or daemon restart \
                     ends the unlock and every client's clearance with it",
                    request.name()
                ),
                "unlock an identity first (rekindle identity list, then rekindle unlock)",
            );
        }
        IpcResponse::error_with_remediation(
            403,
            format!(
                "{} needs {required:?} clearance; this client has {:?}: \
                 the unlocked identity does not trust this client's key",
                request.name(),
                caller.clearance
            ),
            "unlock the identity from this client once, which registers its key",
        )
    }

    /// Dispatch `request` holding its lane for the whole handler;
    /// `acknowledged` is what committing its review acknowledged.
    pub(crate) async fn dispatch_in_lane(
        &self,
        request: IpcRequest,
        caller: &Caller,
        acknowledged: Acknowledged,
    ) -> IpcResponse {
        match request.lane() {
            Lane::Query => super::router::dispatch(self, request, caller, acknowledged).await,
            Lane::Write => {
                let _lane = self.write_lane.read().await;
                super::router::dispatch(self, request, caller, acknowledged).await
            }
            Lane::Exclusive => {
                let _lane = self.write_lane.write().await;
                super::router::dispatch(self, request, caller, acknowledged).await
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

#[cfg(test)]
#[path = "clearance_tests.rs"]
mod clearance_tests;

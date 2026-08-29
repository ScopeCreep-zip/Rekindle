//! `DaemonContext` inherent methods: request-handling helpers shared across
//! dispatch submodules, the state-violation error constructor, and the
//! daemon's own bus-subscriber loop.

use std::sync::Arc;

use rekindle_transport::{Session, TransportNode};

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use super::DaemonContext;

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
                "no identity loaded — run: rekindle init",
                "initialize an identity first: rekindle init",
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
                "unlock the daemon first: rekindle unlock",
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

/// Produce a standard error response for state violations.
pub(crate) fn state_error(state: DaemonState, required: &str) -> IpcResponse {
    IpcResponse::error_with_remediation(
        409,
        format!(
            "cannot perform {required} operation in state '{}'",
            state.as_str()
        ),
        if state == DaemonState::Locked {
            "unlock the daemon first: rekindle unlock"
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
    /// dispatches each to the appropriate handler, and sends correlated
    /// `BusPayload::Response` messages back through the bus.
    ///
    /// This method runs until the bus connection is closed (daemon shutdown).
    pub async fn run_subscriber(
        self: &std::sync::Arc<Self>,
        mut client: crate::ipc::client::BusClient,
    ) {
        tracing::info!("daemon bus subscriber started");

        loop {
            let msg = match client.recv_bus_message().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "daemon subscriber: decode failed, skipping");
                    continue;
                }
                None => {
                    tracing::info!("daemon subscriber: bus connection closed");
                    break;
                }
            };

            let request = match msg.payload {
                crate::ipc::protocol::BusPayload::Request(req) => req,
                other => {
                    tracing::debug!(payload = ?std::mem::discriminant(&other), "daemon subscriber: non-request payload, ignoring");
                    continue;
                }
            };

            let correlation_id = msg.msg_id;
            let level = msg.security_level;
            let response = super::router::dispatch(self, request).await;

            // Serialize IpcResponse to JSON bytes for the bus wire format.
            // IpcResponse contains serde_json::Value which postcard cannot handle.
            let response_bytes = match serde_json::to_vec(&response) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, "daemon subscriber: failed to serialize response");
                    continue;
                }
            };

            if let Err(e) = client.respond(response_bytes, correlation_id, level).await {
                tracing::error!(error = %e, "daemon subscriber: failed to send response");
            }
        }

        tracing::info!("daemon bus subscriber stopped");
    }
}
